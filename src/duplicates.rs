//! Deterministic, AST-normalized clone detection.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

const HASH_BASE: u64 = 1_000_000_007;

#[derive(Debug, Clone)]
pub struct DuplicateConfig {
    pub min_tokens: usize,
    pub min_lines: usize,
    pub max_groups: usize,
}

impl Default for DuplicateConfig {
    fn default() -> Self {
        Self {
            min_tokens: 40,
            min_lines: 5,
            max_groups: 100,
        }
    }
}

#[derive(Debug, Error)]
pub enum DuplicateError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("min_tokens must be greater than zero")]
    InvalidMinTokens,
    #[error("min_lines must be greater than zero")]
    InvalidMinLines,
    #[error("max_groups must be greater than zero")]
    InvalidMaxGroups,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: DuplicateCoverage,
    pub config: DuplicateConfigReport,
    pub totals: DuplicateTotals,
    pub groups: Vec<CloneGroup>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateCoverage {
    pub enumerated_files: usize,
    pub considered_files: usize,
    pub skipped_files: usize,
    pub syntax_error_files: usize,
    pub considered_tokens: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateConfigReport {
    pub min_tokens: usize,
    pub min_lines: usize,
    pub max_groups: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DuplicateTotals {
    pub clone_groups: usize,
    pub clone_occurrences: usize,
    pub duplicated_tokens: usize,
    pub duplicated_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloneGroup {
    pub digest: String,
    pub tokens_per_occurrence: usize,
    pub lines_per_occurrence: usize,
    pub duplicated_token_mass: usize,
    pub duplicated_line_mass: usize,
    pub occurrences: Vec<CloneOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CloneOccurrence {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone)]
struct Token {
    normalized: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Debug)]
struct TokenFile {
    path: String,
    language: SourceLanguage,
    tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    file: usize,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct RawGroup {
    digest: String,
    tokens: usize,
    lines: usize,
    spans: Vec<Span>,
}

pub fn analyze_duplicates(
    input: &Path,
    config: &DuplicateConfig,
) -> Result<DuplicateReport, DuplicateError> {
    if config.min_tokens == 0 {
        return Err(DuplicateError::InvalidMinTokens);
    }
    if config.min_lines == 0 {
        return Err(DuplicateError::InvalidMinLines);
    }
    if config.max_groups == 0 {
        return Err(DuplicateError::InvalidMaxGroups);
    }

    let source_tree = load_source_tree(input)?;
    let mut files = Vec::with_capacity(source_tree.files.len());
    let mut syntax_error_files = 0;
    for file in &source_tree.files {
        let parsed = parse_source(file)?;
        if parsed.has_syntax_errors {
            syntax_error_files += 1;
        }
        files.push(TokenFile {
            path: file.path.clone(),
            language: file.language,
            tokens: normalized_tokens(file, parsed.tree.root_node()),
        });
    }
    let considered_tokens = files.iter().map(|file| file.tokens.len()).sum();
    let mut groups = find_groups(&files, config);
    groups.sort_by(|a, b| {
        let a_mass = a.tokens * a.spans.len();
        let b_mass = b.tokens * b.spans.len();
        Reverse(a_mass)
            .cmp(&Reverse(b_mass))
            .then_with(|| first_path(a, &files).cmp(first_path(b, &files)))
            .then_with(|| a.digest.cmp(&b.digest))
    });
    groups.truncate(config.max_groups);

    let public_groups: Vec<_> = groups
        .iter()
        .map(|group| public_group(group, &files))
        .collect();
    let totals = totals(&groups, &files);
    Ok(DuplicateReport {
        root: source_tree.root,
        analyzer: "tree-sitter normalized-token clone detector".to_owned(),
        coverage: DuplicateCoverage {
            enumerated_files: source_tree.enumerated,
            considered_files: files.len(),
            skipped_files: source_tree.skipped,
            syntax_error_files,
            considered_tokens,
        },
        config: DuplicateConfigReport {
            min_tokens: config.min_tokens,
            min_lines: config.min_lines,
            max_groups: config.max_groups,
        },
        totals,
        groups: public_groups,
        limitations: vec![
            "This is a structural proxy: normalization can create false positives and is not semantic equivalence.".to_owned(),
            "Identifiers and literals are replaced by typed placeholders; binding identity, types, and runtime behavior are not compared.".to_owned(),
            "Only supported source files discovered by the shared source loader are considered.".to_owned(),
            "Syntax-error files are parsed using tree-sitter's error-tolerant tree, so their results may be partial.".to_owned(),
            "Reported groups are capped after deterministic severity ordering; totals describe only reported groups.".to_owned(),
        ],
    })
}

fn normalized_tokens(file: &SourceFile, root: Node<'_>) -> Vec<Token> {
    let mut out = Vec::new();
    collect_leaves(file, root, &mut out);
    out
}

fn collect_leaves(file: &SourceFile, node: Node<'_>, out: &mut Vec<Token>) {
    if is_comment(node.kind()) {
        return;
    }
    if atomic_literal_node(node.kind()) {
        out.push(Token {
            normalized: literal_placeholder(node.kind()).to_owned(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
        return;
    }
    if node.child_count() != 0 {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_leaves(file, child, out);
        }
        return;
    }
    let text = &file.bytes[node.byte_range()];
    if text.iter().all(u8::is_ascii_whitespace) || text.is_empty() {
        return;
    }
    let normalized = normalize_leaf(node, text);
    out.push(Token {
        normalized,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
    });
}

fn is_comment(kind: &str) -> bool {
    kind == "comment" || kind.ends_with("_comment")
}

fn atomic_literal_node(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "string_literal"
            | "raw_string_literal"
            | "interpreted_string_literal"
            | "char_literal"
            | "character_literal"
            | "regex"
            | "regex_literal"
    )
}

fn literal_placeholder(kind: &str) -> &'static str {
    if kind.contains("char") {
        "<character>"
    } else if kind.contains("regex") {
        "<regex>"
    } else {
        "<string>"
    }
}

fn normalize_leaf(node: Node<'_>, text: &[u8]) -> String {
    let kind = node.kind();
    if is_identifier(kind) {
        return "<identifier>".to_owned();
    }
    if is_string(kind) {
        return "<string>".to_owned();
    }
    if is_number(kind) {
        return "<number>".to_owned();
    }
    if kind.contains("char") && (kind.contains("literal") || kind == "char") {
        return "<character>".to_owned();
    }
    if matches!(kind, "true" | "false" | "boolean" | "boolean_literal") {
        return "<boolean>".to_owned();
    }
    if matches!(kind, "null" | "none" | "nil" | "undefined" | "none_literal") {
        return "<null>".to_owned();
    }
    if kind.contains("regex") {
        return "<regex>".to_owned();
    }
    if kind.ends_with("_literal") {
        return format!("<literal:{kind}>");
    }
    String::from_utf8_lossy(text).into_owned()
}

fn is_identifier(kind: &str) -> bool {
    kind == "identifier"
        || kind.ends_with("_identifier")
        || matches!(
            kind,
            "field_name" | "property_name" | "shorthand_property_identifier_pattern"
        )
}

fn is_string(kind: &str) -> bool {
    kind.contains("string") && !kind.contains("interpolation")
        || matches!(
            kind,
            "template_chars" | "interpreted_string_literal" | "raw_string_literal"
        )
}

fn is_number(kind: &str) -> bool {
    kind.contains("integer")
        || kind.contains("float")
        || kind.contains("number")
        || matches!(kind, "decimal_literal" | "imaginary_literal")
}

fn find_groups(files: &[TokenFile], config: &DuplicateConfig) -> Vec<RawGroup> {
    let mut buckets: BTreeMap<(SourceLanguage, u64), Vec<(usize, usize)>> = BTreeMap::new();
    for (file_index, file) in files.iter().enumerate() {
        if file.tokens.len() < config.min_tokens {
            continue;
        }
        for (start, hash) in rolling_hashes(&file.tokens, config.min_tokens)
            .into_iter()
            .enumerate()
        {
            buckets
                .entry((file.language, hash))
                .or_default()
                .push((file_index, start));
        }
    }
    let mut candidates = BTreeSet::new();
    for occurrences in buckets.values() {
        for left in 0..occurrences.len() {
            for right in left + 1..occurrences.len() {
                let (a_file, a_start) = occurrences[left];
                let (b_file, b_start) = occurrences[right];
                if !equal_tokens(
                    &files[a_file].tokens,
                    a_start,
                    &files[b_file].tokens,
                    b_start,
                    config.min_tokens,
                ) {
                    continue;
                }
                let (mut as_, mut bs) = (a_start, b_start);
                while as_ > 0
                    && bs > 0
                    && files[a_file].tokens[as_ - 1].normalized
                        == files[b_file].tokens[bs - 1].normalized
                {
                    as_ -= 1;
                    bs -= 1;
                }
                let (mut ae, mut be) = (a_start + config.min_tokens, b_start + config.min_tokens);
                while ae < files[a_file].tokens.len()
                    && be < files[b_file].tokens.len()
                    && files[a_file].tokens[ae].normalized == files[b_file].tokens[be].normalized
                {
                    ae += 1;
                    be += 1;
                }
                if a_file == b_file {
                    let separation = bs - as_;
                    if separation < config.min_tokens {
                        continue;
                    }
                    let non_overlapping_length = (ae - as_).min(separation);
                    ae = as_ + non_overlapping_length;
                    be = bs + non_overlapping_length;
                }
                if span_lines(&files[a_file].tokens, as_, ae) < config.min_lines
                    || span_lines(&files[b_file].tokens, bs, be) < config.min_lines
                {
                    continue;
                }
                candidates.insert((
                    Span {
                        file: a_file,
                        start: as_,
                        end: ae,
                    },
                    Span {
                        file: b_file,
                        start: bs,
                        end: be,
                    },
                ));
            }
        }
    }

    let mut by_content: BTreeMap<(SourceLanguage, String, usize), BTreeSet<Span>> = BTreeMap::new();
    for (a, b) in candidates {
        let digest = token_digest(&files[a.file].tokens[a.start..a.end]);
        let key = (files[a.file].language, digest, a.end - a.start);
        by_content.entry(key).or_default().extend([a, b]);
    }
    let mut raw = Vec::new();
    for ((_language, digest, tokens), spans) in by_content {
        let spans = suppress_overlapping_spans(spans.into_iter().collect());
        if spans.len() < 2 {
            continue;
        }
        let lines = spans
            .iter()
            .map(|s| span_lines(&files[s.file].tokens, s.start, s.end))
            .max()
            .unwrap_or(0);
        raw.push(RawGroup {
            digest,
            tokens,
            lines,
            spans,
        });
    }
    suppress_subsumed_groups(raw)
}

fn rolling_hashes(tokens: &[Token], width: usize) -> Vec<u64> {
    let values: Vec<u64> = tokens
        .iter()
        .map(|token| stable_hash(token.normalized.as_bytes()))
        .collect();
    let mut power = 1_u64;
    for _ in 1..width {
        power = power.wrapping_mul(HASH_BASE);
    }
    let mut hash = 0_u64;
    for value in &values[..width] {
        hash = hash.wrapping_mul(HASH_BASE).wrapping_add(*value);
    }
    let mut out = Vec::with_capacity(values.len() - width + 1);
    out.push(hash);
    for index in width..values.len() {
        hash = hash.wrapping_sub(values[index - width].wrapping_mul(power));
        hash = hash.wrapping_mul(HASH_BASE).wrapping_add(values[index]);
        out.push(hash);
    }
    out
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn equal_tokens(a: &[Token], ai: usize, b: &[Token], bi: usize, len: usize) -> bool {
    a[ai..ai + len]
        .iter()
        .zip(&b[bi..bi + len])
        .all(|(x, y)| x.normalized == y.normalized)
}

fn token_digest(tokens: &[Token]) -> String {
    let mut hash = Sha256::new();
    for token in tokens {
        hash.update((token.normalized.len() as u64).to_be_bytes());
        hash.update(token.normalized.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn suppress_overlapping_spans(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort();
    let mut kept: Vec<Span> = Vec::new();
    for span in spans {
        if kept.iter().any(|existing| {
            existing.file == span.file
                && ranges_overlap(existing.start, existing.end, span.start, span.end)
        }) {
            continue;
        }
        kept.push(span);
    }
    kept
}

fn suppress_subsumed_groups(mut groups: Vec<RawGroup>) -> Vec<RawGroup> {
    groups.sort_by_key(|group| (Reverse(group.tokens), group.digest.clone()));
    let mut kept: Vec<RawGroup> = Vec::new();
    'candidate: for group in groups {
        for existing in &kept {
            if group.spans.iter().all(|span| {
                existing.spans.iter().any(|outer| {
                    outer.file == span.file && outer.start <= span.start && outer.end >= span.end
                })
            }) {
                continue 'candidate;
            }
        }
        kept.push(group);
    }
    kept
}

fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
}

fn span_lines(tokens: &[Token], start: usize, end: usize) -> usize {
    tokens[end - 1]
        .end_line
        .saturating_sub(tokens[start].start_line)
        + 1
}

fn first_path<'a>(group: &RawGroup, files: &'a [TokenFile]) -> &'a str {
    group
        .spans
        .iter()
        .map(|span| files[span.file].path.as_str())
        .min()
        .unwrap_or("")
}

fn public_group(group: &RawGroup, files: &[TokenFile]) -> CloneGroup {
    let mut occurrences: Vec<_> = group
        .spans
        .iter()
        .map(|span| CloneOccurrence {
            path: files[span.file].path.clone(),
            start_line: files[span.file].tokens[span.start].start_line,
            end_line: files[span.file].tokens[span.end - 1].end_line,
        })
        .collect();
    occurrences.sort();
    CloneGroup {
        digest: group.digest.clone(),
        tokens_per_occurrence: group.tokens,
        lines_per_occurrence: group.lines,
        duplicated_token_mass: group.tokens * group.spans.len(),
        duplicated_line_mass: group
            .spans
            .iter()
            .map(|span| span_lines(&files[span.file].tokens, span.start, span.end))
            .sum(),
        occurrences,
    }
}

fn totals(groups: &[RawGroup], files: &[TokenFile]) -> DuplicateTotals {
    let mut token_intervals: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    let mut line_intervals: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for group in groups {
        for span in &group.spans {
            token_intervals
                .entry(span.file)
                .or_default()
                .push((span.start, span.end));
            let tokens = &files[span.file].tokens;
            line_intervals.entry(span.file).or_default().push((
                tokens[span.start].start_line,
                tokens[span.end - 1].end_line + 1,
            ));
        }
    }
    DuplicateTotals {
        clone_groups: groups.len(),
        clone_occurrences: groups.iter().map(|group| group.spans.len()).sum(),
        duplicated_tokens: token_intervals
            .values_mut()
            .map(|ranges| union_mass(ranges))
            .sum(),
        duplicated_lines: line_intervals
            .values_mut()
            .map(|ranges| union_mass(ranges))
            .sum(),
    }
}

fn union_mass(ranges: &mut [(usize, usize)]) -> usize {
    ranges.sort_unstable();
    let mut total = 0;
    let mut current: Option<(usize, usize)> = None;
    for &(start, end) in ranges.iter() {
        match current {
            Some((open, close)) if start <= close => current = Some((open, close.max(end))),
            Some((open, close)) => {
                total += close - open;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    if let Some((open, close)) = current {
        total += close - open;
    }
    total
}

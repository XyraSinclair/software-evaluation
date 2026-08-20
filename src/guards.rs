//! Guard-predicate census: repeated normalized condition expressions.
//!
//! Clone detection with a 40-token floor cannot see the 5-token guard
//! (`if x.is_none() { return }`) stamped across a codebase — the signature
//! defensive-redundancy pattern of generated code. This census normalizes
//! every branch/loop condition (identifiers and literals to typed
//! placeholders, matching the clone detector's convention) and reports
//! predicates repeated at or above a declared floor, with every occurrence
//! and the distinct raw spellings as witnesses.
//!
//! A repeated guard establishes repetition only. It cannot establish that
//! consolidation is right: idiomatic null checks, loop bounds, and defensive
//! preconditions repeat legitimately. The census is an attention router with
//! declared thresholds, never a verdict.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

pub const GUARDS_DEFAULT_MIN_COUNT: usize = 3;
pub const GUARDS_DEFAULT_MIN_TOKENS: usize = 3;
pub const GUARDS_DEFAULT_MAX_PATTERNS: usize = 100;

#[derive(Debug, Clone)]
pub struct GuardsConfig {
    /// Minimum occurrences for a predicate pattern to be reported.
    pub min_count: usize,
    /// Minimum normalized tokens in a condition to be considered.
    pub min_tokens: usize,
    /// Maximum reported patterns; exceeding it censors the list.
    pub max_patterns: usize,
}

impl Default for GuardsConfig {
    fn default() -> Self {
        Self {
            min_count: GUARDS_DEFAULT_MIN_COUNT,
            min_tokens: GUARDS_DEFAULT_MIN_TOKENS,
            max_patterns: GUARDS_DEFAULT_MAX_PATTERNS,
        }
    }
}

#[derive(Debug, Error)]
pub enum GuardsError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("min-count must be greater than zero")]
    InvalidMinCount,
    #[error("min-tokens must be greater than zero")]
    InvalidMinTokens,
    #[error("max-patterns must be greater than zero")]
    InvalidMaxPatterns,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardsReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: GuardsCoverage,
    pub config: GuardsConfigReport,
    /// Conditions meeting the token floor (the pattern denominator).
    pub considered_conditions: usize,
    /// Patterns found at or above the occurrence floor.
    pub patterns_found: usize,
    /// True when more qualifying patterns existed than `max_patterns`.
    pub patterns_censored: bool,
    /// Occurrences inside reported patterns over considered conditions;
    /// exact integers, the f64 is display-only.
    pub repeated_occurrence_numerator: usize,
    pub repeated_occurrence_denominator: usize,
    pub repeated_occurrence_fraction: f64,
    pub patterns: Vec<GuardPattern>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardsCoverage {
    pub enumerated_files: usize,
    pub considered_files: usize,
    pub skipped_files: usize,
    pub syntax_error_files: usize,
    /// All conditions observed, including those below the token floor.
    pub conditions_observed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardsConfigReport {
    pub min_count: usize,
    pub min_tokens: usize,
    pub max_patterns: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardPattern {
    /// SHA-256 of the normalized token sequence.
    pub digest: String,
    /// The normalized token sequence itself, for reading.
    pub normalized: String,
    pub tokens: usize,
    pub occurrences: Vec<GuardOccurrence>,
    pub occurrence_count: usize,
    pub distinct_files: usize,
    /// Up to eight distinct raw spellings, deterministically first-seen in
    /// path/line order.
    pub raw_spellings: Vec<String>,
    /// Occurrences of the single most-repeated exact raw spelling: the
    /// verbatim-copy tier inside the normalized group.
    pub identical_raw_max: usize,
    /// That most-repeated spelling (lexicographically first on ties).
    pub identical_raw_witness: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardOccurrence {
    pub path: String,
    pub line: usize,
}

pub fn analyze_guards(input: &Path, config: &GuardsConfig) -> Result<GuardsReport, GuardsError> {
    if config.min_count == 0 {
        return Err(GuardsError::InvalidMinCount);
    }
    if config.min_tokens == 0 {
        return Err(GuardsError::InvalidMinTokens);
    }
    if config.max_patterns == 0 {
        return Err(GuardsError::InvalidMaxPatterns);
    }

    let source_tree = load_source_tree(input)?;
    let mut syntax_error_files = 0usize;
    let mut conditions_observed = 0usize;
    let mut considered_conditions = 0usize;
    struct Accum {
        normalized: String,
        tokens: usize,
        occurrences: Vec<GuardOccurrence>,
        raw_spellings: Vec<String>,
        raw_counts: BTreeMap<String, usize>,
    }
    let mut patterns: BTreeMap<String, Accum> = BTreeMap::new();

    for file in &source_tree.files {
        let parsed = parse_source(file)?;
        if parsed.tree.root_node().has_error() {
            syntax_error_files += 1;
        }
        walk(parsed.tree.root_node(), &mut |node| {
            let Some(condition) = condition_of(file, node) else {
                return;
            };
            conditions_observed += 1;
            let mut tokens = Vec::new();
            collect_normalized(file, condition, &mut tokens);
            if tokens.len() < config.min_tokens {
                return;
            }
            considered_conditions += 1;
            let normalized = tokens.join(" ");
            let digest = hex_digest(&normalized);
            let raw = condensed_raw(text(file, condition));
            let entry = patterns.entry(digest).or_insert_with(|| Accum {
                normalized,
                tokens: tokens.len(),
                occurrences: Vec::new(),
                raw_spellings: Vec::new(),
                raw_counts: BTreeMap::new(),
            });
            *entry.raw_counts.entry(raw.clone()).or_insert(0) += 1;
            entry.occurrences.push(GuardOccurrence {
                path: file.path.clone(),
                line: condition.start_position().row + 1,
            });
            if entry.raw_spellings.len() < 8 && !entry.raw_spellings.contains(&raw) {
                entry.raw_spellings.push(raw);
            }
        });
    }

    let mut qualifying: Vec<GuardPattern> = patterns
        .into_iter()
        .filter(|(_, accum)| accum.occurrences.len() >= config.min_count)
        .map(|(digest, accum)| {
            let distinct_files = {
                let mut paths: Vec<&str> = accum
                    .occurrences
                    .iter()
                    .map(|occurrence| occurrence.path.as_str())
                    .collect();
                paths.sort_unstable();
                paths.dedup();
                paths.len()
            };
            let (identical_raw_witness, identical_raw_max) = accum
                .raw_counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(raw, count)| (raw.clone(), *count))
                .unwrap_or_default();
            GuardPattern {
                digest,
                normalized: accum.normalized,
                tokens: accum.tokens,
                occurrence_count: accum.occurrences.len(),
                distinct_files,
                occurrences: accum.occurrences,
                raw_spellings: accum.raw_spellings,
                identical_raw_max,
                identical_raw_witness,
            }
        })
        .collect();
    qualifying.sort_by(|a, b| {
        (b.occurrence_count * b.tokens)
            .cmp(&(a.occurrence_count * a.tokens))
            .then(b.occurrence_count.cmp(&a.occurrence_count))
            .then(a.digest.cmp(&b.digest))
    });
    let patterns_found = qualifying.len();
    let patterns_censored = patterns_found > config.max_patterns;
    qualifying.truncate(config.max_patterns);
    let repeated_occurrence_numerator: usize = qualifying
        .iter()
        .map(|pattern| pattern.occurrence_count)
        .sum();

    Ok(GuardsReport {
        root: source_tree.root,
        analyzer: "normalized branch/loop condition census".to_owned(),
        coverage: GuardsCoverage {
            enumerated_files: source_tree.enumerated,
            considered_files: source_tree.files.len(),
            skipped_files: source_tree.skipped,
            syntax_error_files,
            conditions_observed,
        },
        config: GuardsConfigReport {
            min_count: config.min_count,
            min_tokens: config.min_tokens,
            max_patterns: config.max_patterns,
        },
        considered_conditions,
        patterns_found,
        patterns_censored,
        repeated_occurrence_numerator,
        repeated_occurrence_denominator: considered_conditions,
        repeated_occurrence_fraction: if considered_conditions == 0 {
            0.0
        } else {
            repeated_occurrence_numerator as f64 / considered_conditions as f64
        },
        patterns: qualifying,
        limitations: vec![
            "Identifiers and literals normalize to typed placeholders, so `x.is_none()` and `y.is_none()` group; the raw spellings are retained as witnesses.".to_owned(),
            "Repetition is the only claim: idiomatic null checks, bounds, and preconditions repeat legitimately, and grouping does not establish that consolidation is desirable.".to_owned(),
            "Only if/while/for/do/ternary condition positions are scanned; guards expressed as early-return match arms, boolean assignments, or assert calls are not counted.".to_owned(),
            "Occurrence and token floors and the pattern cap are configuration; a censored pattern list is incomplete, and the repeated-occurrence fraction covers reported patterns only.".to_owned(),
        ],
    })
}

/// The condition child of a branch/loop construct, unwrapped from redundant
/// parentheses.
fn condition_of<'a>(file: &SourceFile, node: Node<'a>) -> Option<Node<'a>> {
    let kind = node.kind();
    let relevant = match file.language {
        SourceLanguage::Rust => matches!(kind, "if_expression" | "while_expression"),
        SourceLanguage::Python => matches!(
            kind,
            "if_statement" | "while_statement" | "conditional_expression" | "elif_clause"
        ),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => matches!(
            kind,
            "if_statement" | "while_statement" | "do_statement" | "ternary_expression"
        ),
        SourceLanguage::Go => matches!(kind, "if_statement" | "for_statement"),
    };
    if !relevant {
        return None;
    }
    let mut condition = node.child_by_field_name("condition")?;
    while condition.kind() == "parenthesized_expression" && condition.named_child_count() == 1 {
        condition = condition.named_child(0)?;
    }
    Some(condition)
}

fn collect_normalized(file: &SourceFile, node: Node<'_>, out: &mut Vec<String>) {
    if node.child_count() == 0 {
        let kind = node.kind();
        let token = if kind.contains("identifier") {
            "ID".to_owned()
        } else if kind.contains("string")
            || kind.contains("char")
            || kind.contains("number")
            || kind.contains("integer")
            || kind.contains("float")
        {
            "LIT".to_owned()
        } else {
            text(file, node).to_owned()
        };
        if !token.trim().is_empty() {
            out.push(token);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_normalized(file, child, out);
    }
}

fn condensed_raw(raw: &str) -> String {
    let condensed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if condensed.len() > 120 {
        format!("{}…", &condensed[..condensed.floor_char_boundary(119)])
    } else {
        condensed
    }
}

fn hex_digest(normalized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn walk(node: Node<'_>, visit: &mut impl FnMut(Node<'_>)) {
    visit(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, visit);
    }
}

fn text<'a>(file: &'a SourceFile, node: Node<'_>) -> &'a str {
    std::str::from_utf8(&file.bytes[node.byte_range()]).unwrap_or("")
}

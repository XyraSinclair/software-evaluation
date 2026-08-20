//! Name-level liveness census: definitions ranked by how often their name is
//! mentioned anywhere else in the repository.
//!
//! This is deliberately a *name* instrument, not a symbol instrument. It never
//! resolves references; it counts identifier-shaped leaves. The direction of
//! error is therefore fixed and stated: any mention of a name counts as
//! liveness for every definition of that name, so a zero-mention name is a
//! strong dead-candidate while a mentioned name proves nothing. Symbol-level
//! liveness for Rust stays with `seval symbols`; this census covers every
//! supported language and survives macros, dispatch, and re-exports by
//! over-counting liveness, never by inventing deadness.
//!
//! Three findings, all witnesses over the declared denominator:
//!
//! - **Dead candidates**: defined names with zero non-definition mentions,
//!   split by lexical publicness (external consumers are invisible to a
//!   repository scan, so public rows are a separate, weaker list).
//! - **Single-use names**: exactly one non-definition mention, with the
//!   mention site — the inline-candidate census.
//! - **Test-only names**: non-test definitions whose every non-definition
//!   mention sits in test-classified files.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};
use crate::tests_analysis::{FileRole, classify_file};

#[derive(Debug, Error)]
pub enum LivenessError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Serialize)]
pub struct LivenessReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: LivenessCoverage,
    /// Named definitions found (functions, methods, types, consts).
    pub definitions: usize,
    /// Distinct defined names; rows aggregate per name because a name-level
    /// census cannot attribute a mention to one definition among several.
    pub distinct_names: usize,
    pub dead_private_names: usize,
    pub dead_public_names: usize,
    pub single_use_names: usize,
    pub test_only_names: usize,
    pub excluded_names: usize,
    pub rows: Vec<NameRow>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LivenessCoverage {
    pub enumerated_files: usize,
    pub considered_files: usize,
    pub skipped_files: usize,
    pub syntax_error_files: usize,
    /// Identifier-shaped leaves scanned across all considered files.
    pub identifier_leaves: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NameRow {
    pub name: String,
    pub definition_count: usize,
    pub definitions: Vec<DefinitionSite>,
    /// True when any definition is lexically public for its language (Rust
    /// bare `pub`, TS/JS `export`, Go capitalized, Python non-underscore).
    pub any_public: bool,
    /// Identifier-leaf mentions of this name excluding the definitions' own
    /// name leaves. Conservative liveness: parameters, fields, or unrelated
    /// bindings sharing the name all count.
    pub mentions: usize,
    /// The subset of `mentions` inside test-classified files.
    pub mentions_in_tests: usize,
    /// Present only when `mentions <= 1`: the mention site, or none for dead
    /// candidates.
    pub mention_witness: Option<MentionSite>,
    /// Present when the name is excluded from dead-candidacy: `entry-point`,
    /// `dunder`, or `test-definition`.
    pub candidacy_exclusion: Option<String>,
    pub status: NameStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NameStatus {
    DeadPrivate,
    DeadPublic,
    SingleUse,
    TestOnly,
    Excluded,
    Live,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefinitionSite {
    pub path: String,
    pub line: usize,
    pub kind: String,
    pub lexically_public: bool,
    pub in_test_file: bool,
    /// Rust: this definition sits in a trait impl, so it is reachable through
    /// dispatch without its name appearing at call sites.
    pub implements_trait: bool,
    /// Rust: a preceding attribute contains `test`; the framework invokes it.
    pub test_attribute: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MentionSite {
    pub path: String,
    pub line: usize,
}

struct DefRecord {
    sites: Vec<DefinitionSite>,
}

pub fn analyze_liveness(input: &Path) -> Result<LivenessReport, LivenessError> {
    let source_tree = load_source_tree(input)?;
    let mut syntax_error_files = 0usize;
    let mut identifier_leaves = 0usize;
    let mut defs: BTreeMap<String, DefRecord> = BTreeMap::new();
    // Per-name identifier-leaf counts: (total, in-test-files).
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    // Per-name count of definition name leaves: (total, in-test-files).
    let mut def_leaves: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    let mut parsed_files = Vec::new();
    for file in &source_tree.files {
        let parsed = parse_source(file)?;
        if parsed.tree.root_node().has_error() {
            syntax_error_files += 1;
        }
        parsed_files.push((file, parsed));
    }

    for (file, parsed) in &parsed_files {
        let in_test = classify_file(file) == FileRole::Test;
        walk(parsed.tree.root_node(), &mut |node| {
            if node.child_count() == 0 && node.kind().contains("identifier") {
                identifier_leaves += 1;
                let name = text(file, node).to_owned();
                let entry = counts.entry(name).or_insert((0, 0));
                entry.0 += 1;
                if in_test {
                    entry.1 += 1;
                }
            }
            // Rust format strings hold `{name}` interpolations invisible to
            // the grammar; count them lexically so they read as liveness.
            if file.language == SourceLanguage::Rust && node.kind() == "string_content" {
                for name in brace_interpolations(text(file, node)) {
                    identifier_leaves += 1;
                    let entry = counts.entry(name).or_insert((0, 0));
                    entry.0 += 1;
                    if in_test {
                        entry.1 += 1;
                    }
                }
            }
            if let Some((name_node, kind, public)) = definition_of(file, node) {
                let name = text(file, name_node).to_owned();
                let leaf_entry = def_leaves.entry(name.clone()).or_insert((0, 0));
                leaf_entry.0 += 1;
                if in_test {
                    leaf_entry.1 += 1;
                }
                let implements_trait =
                    file.language == SourceLanguage::Rust && inside_trait_impl(node);
                let test_attribute =
                    file.language == SourceLanguage::Rust && has_test_attribute(file, node);
                defs.entry(name)
                    .or_insert(DefRecord { sites: Vec::new() })
                    .sites
                    .push(DefinitionSite {
                        path: file.path.clone(),
                        line: name_node.start_position().row + 1,
                        kind: kind.to_owned(),
                        lexically_public: public,
                        in_test_file: in_test,
                        implements_trait,
                        test_attribute,
                    });
            }
        });
    }

    // Second pass: mention witnesses for names with at most one non-definition
    // mention. Definition name leaves are skipped by position.
    let mut sparse: BTreeMap<String, Option<MentionSite>> = BTreeMap::new();
    for name in defs.keys() {
        let (total, _) = counts.get(name).copied().unwrap_or((0, 0));
        let (def_total, _) = def_leaves.get(name).copied().unwrap_or((0, 0));
        let mentions = total.saturating_sub(def_total);
        if mentions <= 1 {
            sparse.insert(name.clone(), None);
        }
    }
    if !sparse.is_empty() {
        for (file, parsed) in &parsed_files {
            walk(parsed.tree.root_node(), &mut |node| {
                if node.child_count() == 0 && node.kind().contains("identifier") {
                    let name = text(file, node);
                    if let Some(slot) = sparse.get_mut(name) {
                        let line = node.start_position().row + 1;
                        let is_def_leaf = defs[name]
                            .sites
                            .iter()
                            .any(|site| site.path == file.path && site.line == line);
                        if !is_def_leaf && slot.is_none() {
                            *slot = Some(MentionSite {
                                path: file.path.clone(),
                                line,
                            });
                        }
                    }
                }
                if file.language == SourceLanguage::Rust && node.kind() == "string_content" {
                    for name in brace_interpolations(text(file, node)) {
                        if let Some(slot) = sparse.get_mut(name.as_str())
                            && slot.is_none()
                        {
                            *slot = Some(MentionSite {
                                path: file.path.clone(),
                                line: node.start_position().row + 1,
                            });
                        }
                    }
                }
            });
        }
    }

    let mut rows: Vec<NameRow> = Vec::new();
    let (mut dead_private, mut dead_public, mut single_use, mut test_only, mut excluded) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let definitions = defs.values().map(|record| record.sites.len()).sum();
    for (name, record) in defs {
        let (total, total_tests) = counts.get(&name).copied().unwrap_or((0, 0));
        let (def_total, def_tests) = def_leaves.get(&name).copied().unwrap_or((0, 0));
        let mentions = total.saturating_sub(def_total);
        let mentions_in_tests = total_tests.saturating_sub(def_tests);
        let any_public = record.sites.iter().any(|site| site.lexically_public);
        let all_defs_in_tests = record
            .sites
            .iter()
            .all(|site| site.in_test_file || site.test_attribute);
        let all_trait_impls = record.sites.iter().all(|site| site.implements_trait);
        let candidacy_exclusion = if name == "main" || name == "init" {
            Some("entry-point".to_owned())
        } else if name.starts_with("__") && name.ends_with("__") {
            Some("dunder".to_owned())
        } else if all_defs_in_tests {
            Some("test-definition".to_owned())
        } else if all_trait_impls {
            Some("trait-impl".to_owned())
        } else {
            None
        };
        let status = if candidacy_exclusion.is_some() {
            excluded += 1;
            NameStatus::Excluded
        } else if mentions == 0 {
            if any_public {
                dead_public += 1;
                NameStatus::DeadPublic
            } else {
                dead_private += 1;
                NameStatus::DeadPrivate
            }
        } else if mentions == 1 {
            single_use += 1;
            NameStatus::SingleUse
        } else if mentions_in_tests == mentions {
            test_only += 1;
            NameStatus::TestOnly
        } else {
            NameStatus::Live
        };
        let mention_witness = if mentions <= 1 {
            sparse.get(&name).cloned().flatten()
        } else {
            None
        };
        rows.push(NameRow {
            definition_count: record.sites.len(),
            definitions: record.sites,
            any_public,
            mentions,
            mentions_in_tests,
            mention_witness,
            candidacy_exclusion,
            status,
            name,
        });
    }

    Ok(LivenessReport {
        root: source_tree.root,
        analyzer: "name-level identifier-mention liveness census".to_owned(),
        coverage: LivenessCoverage {
            enumerated_files: source_tree.enumerated,
            considered_files: source_tree.files.len(),
            skipped_files: source_tree.skipped,
            syntax_error_files,
            identifier_leaves,
        },
        definitions,
        distinct_names: rows.len(),
        dead_private_names: dead_private,
        dead_public_names: dead_public,
        single_use_names: single_use,
        test_only_names: test_only,
        excluded_names: excluded,
        rows,
        limitations: vec![
            "Mentions are identifier-shaped leaves, never resolved references: any binding, field, or parameter sharing a defined name counts as liveness, so mention counts over-state liveness and never invent deadness.".to_owned(),
            "String literals are not scanned except Rust `{name}` format interpolations (Python f-strings and JS templates parse to real identifiers): names referenced only through plain strings (CLI dispatch tables, serde renames, reflection, dynamic import) read as dead and must be cleared by a human or a runtime probe.".to_owned(),
            "Lexically public definitions may have consumers outside this repository; their dead rows are a separate, weaker list, not deletion evidence.".to_owned(),
            "Multiple definitions sharing one name are aggregated: the census cannot attribute mentions among them, so per-name status is the strongest claim available.".to_owned(),
            "Entry points, dunder names, names defined only in test files or under test attributes, and names defined only in Rust trait impls are excluded from dead-candidacy and reported as excluded, not hidden.".to_owned(),
        ],
    })
}

/// Recognize a named definition at `node`. Returns the name node, a stable
/// kind label, and lexical publicness under the per-language documented rule.
fn definition_of<'a>(file: &SourceFile, node: Node<'a>) -> Option<(Node<'a>, &'static str, bool)> {
    let kind = node.kind();
    let label: &'static str = match (file.language, kind) {
        (SourceLanguage::Rust, "function_item") => "fn",
        (SourceLanguage::Rust, "function_signature_item") => "trait-fn",
        (SourceLanguage::Rust, "struct_item") => "struct",
        (SourceLanguage::Rust, "enum_item") => "enum",
        (SourceLanguage::Rust, "union_item") => "union",
        (SourceLanguage::Rust, "trait_item") => "trait",
        (SourceLanguage::Rust, "type_item") => "type",
        (SourceLanguage::Rust, "const_item") => "const",
        (SourceLanguage::Rust, "static_item") => "static",
        (SourceLanguage::Rust, "macro_definition") => "macro",
        (SourceLanguage::Python, "function_definition") => "def",
        (SourceLanguage::Python, "class_definition") => "class",
        (SourceLanguage::Go, "function_declaration") => "func",
        (SourceLanguage::Go, "method_declaration") => "method",
        (SourceLanguage::Go, "type_spec") => "type",
        (
            SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx,
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "abstract_class_declaration",
        ) => "function-or-class",
        (SourceLanguage::TypeScript | SourceLanguage::Tsx, "interface_declaration") => "interface",
        (SourceLanguage::TypeScript | SourceLanguage::Tsx, "type_alias_declaration") => "type",
        (SourceLanguage::TypeScript | SourceLanguage::Tsx, "enum_declaration") => "enum",
        (
            SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx,
            "method_definition",
        ) => "method",
        (
            SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx,
            "variable_declarator",
        ) => {
            let value_kind = node.child_by_field_name("value")?.kind();
            if value_kind == "arrow_function" || value_kind == "function_expression" {
                "function-binding"
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let name_node = node.child_by_field_name("name")?;
    if !name_node.kind().contains("identifier") {
        return None;
    }
    let name = text(file, name_node);
    let public = match file.language {
        SourceLanguage::Rust => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .any(|child| child.kind() == "visibility_modifier" && text(file, child) == "pub")
        }
        SourceLanguage::Python => !name.starts_with('_'),
        SourceLanguage::Go => name.chars().next().is_some_and(char::is_uppercase),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            has_export_ancestor(node)
        }
    };
    Some((name_node, label, public))
}

/// True when a Rust item's enclosing impl names a trait: the item is
/// reachable through dispatch without its name at any call site.
fn inside_trait_impl(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "impl_item" {
            return parent.child_by_field_name("trait").is_some();
        }
        current = parent.parent();
    }
    false
}

/// True when a preceding sibling attribute contains `test` (`#[test]`,
/// `#[tokio::test]`, `#[rstest]`-style spellings included lexically).
fn has_test_attribute(file: &SourceFile, node: Node<'_>) -> bool {
    let mut current = node.prev_sibling();
    while let Some(sibling) = current {
        if sibling.kind() == "attribute_item" {
            if text(file, sibling).contains("test") {
                return true;
            }
            current = sibling.prev_sibling();
        } else {
            break;
        }
    }
    false
}

fn has_export_ancestor(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "export_statement" {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Identifier-shaped names inside `{...}` interpolation slots of a Rust
/// format-style string body. `{{` escapes are skipped; a name ends at `}` or
/// a format-spec `:`. Conservative in the stated direction: matching brace
/// text that is not an interpolation only adds liveness.
fn brace_interpolations(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut names = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        if index + 1 < bytes.len() && bytes[index + 1] == b'{' {
            index += 2;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        if end > start
            && !bytes[start].is_ascii_digit()
            && end < bytes.len()
            && (bytes[end] == b'}' || bytes[end] == b':')
        {
            names.push(body[start..end].to_owned());
        }
        index = end.max(start);
    }
    names
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

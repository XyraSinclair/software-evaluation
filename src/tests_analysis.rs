//! Deterministic structural inventory of test machinery.
//!
//! The observations in this module are deliberately not a test-quality score.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

#[derive(Debug, Error)]
pub enum TestAnalysisError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileRole {
    Source,
    Test,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestFileRow {
    pub path: String,
    pub language: SourceLanguage,
    pub role: FileRole,
    pub lines: u64,
    pub syntax_errors: bool,
    pub discovered_test_cases: u64,
    pub ignored_test_cases: u64,
    pub suite_declarations: u64,
    /// Structural proxy: calls/statements whose spelling follows common assertion conventions.
    pub assertion_like_calls: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestCoverage {
    pub enumerated_files: usize,
    pub supported_files: usize,
    pub skipped_unsupported_files: usize,
    pub analyzed_source_files: u64,
    pub analyzed_source_lines: u64,
    pub test_files: u64,
    pub test_lines: u64,
    pub syntax_error_files: u64,
    pub discovered_test_cases: u64,
    pub ignored_test_cases: u64,
    pub non_ignored_test_cases: u64,
    pub assertion_like_calls: u64,
    pub source_modules_considered: u64,
    pub source_modules_with_same_stem_test: u64,
    pub source_modules_without_same_stem_test: u64,
    pub test_lines_per_source_line: Option<f64>,
    pub test_cases_per_ksloc: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: TestCoverage,
    pub files: Vec<TestFileRow>,
    /// Supported files that matched neither the source-module nor dedicated-test-file model.
    pub unknown_files: Vec<String>,
    /// Source modules for which the conservative same-stem rule found no test file.
    pub unmatched_source_modules: Vec<String>,
    /// Dedicated test files for which the conservative same-stem rule found no source module.
    pub unmatched_test_files: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Default)]
struct Observations {
    cases: u64,
    ignored: u64,
    suites: u64,
    assertions: u64,
}

#[derive(Clone)]
struct CaseRange {
    start: usize,
    end: usize,
    ignored: bool,
}

pub fn analyze_tests(input: &Path) -> Result<TestReport, TestAnalysisError> {
    let tree = load_source_tree(input)?;
    let mut files = Vec::with_capacity(tree.files.len());
    let mut source_files = Vec::new();
    let mut test_files = Vec::new();
    let unknown_files = Vec::new();

    for file in &tree.files {
        let role = classify_file(file);
        let parsed = parse_source(file)?;
        let observations = inspect(file, parsed.tree.root_node());
        let lines = line_count(&file.bytes);
        files.push(TestFileRow {
            path: file.path.clone(),
            language: file.language,
            role,
            lines,
            syntax_errors: parsed.has_syntax_errors,
            discovered_test_cases: observations.cases,
            ignored_test_cases: observations.ignored,
            suite_declarations: observations.suites,
            assertion_like_calls: observations.assertions,
        });
        match role {
            FileRole::Source if is_source_module(file) => source_files.push(file),
            FileRole::Source => {}
            FileRole::Test => test_files.push(file),
        }
    }

    let (matched_sources, matched_tests) = same_stem_matches(&source_files, &test_files);
    let unmatched_source_modules = source_files
        .iter()
        .filter(|file| !matched_sources.contains(&file.path))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let unmatched_test_files = test_files
        .iter()
        .filter(|file| !matched_tests.contains(&file.path))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();

    let source_lines = files
        .iter()
        .filter(|row| row.role == FileRole::Source)
        .map(|row| row.lines)
        .sum::<u64>();
    let test_lines = files
        .iter()
        .filter(|row| row.role == FileRole::Test)
        .map(|row| row.lines)
        .sum::<u64>();
    let cases = files
        .iter()
        .map(|row| row.discovered_test_cases)
        .sum::<u64>();
    let ignored = files.iter().map(|row| row.ignored_test_cases).sum::<u64>();
    let coverage = TestCoverage {
        enumerated_files: tree.enumerated,
        supported_files: tree.files.len(),
        skipped_unsupported_files: tree.skipped,
        analyzed_source_files: files
            .iter()
            .filter(|row| row.role == FileRole::Source)
            .count() as u64,
        analyzed_source_lines: source_lines,
        test_files: test_files.len() as u64,
        test_lines,
        syntax_error_files: files.iter().filter(|row| row.syntax_errors).count() as u64,
        discovered_test_cases: cases,
        ignored_test_cases: ignored,
        non_ignored_test_cases: cases.saturating_sub(ignored),
        assertion_like_calls: files.iter().map(|row| row.assertion_like_calls).sum(),
        source_modules_considered: source_files.len() as u64,
        source_modules_with_same_stem_test: matched_sources.len() as u64,
        source_modules_without_same_stem_test: unmatched_source_modules.len() as u64,
        test_lines_per_source_line: ratio(test_lines, source_lines),
        test_cases_per_ksloc: if source_lines == 0 {
            None
        } else {
            Some(cases as f64 * 1_000.0 / source_lines as f64)
        },
    };

    Ok(TestReport {
        root: tree.root,
        analyzer: "tree-sitter-test-machinery-v1".to_owned(),
        coverage,
        files,
        unknown_files,
        unmatched_source_modules,
        unmatched_test_files,
        limitations: vec![
            "All counts are structural observations, not measures of test quality, adequacy, execution, coverage, or correctness.".to_owned(),
            "Assertion-like counts are spelling-based structural proxies and can include helper calls or miss custom assertion APIs.".to_owned(),
            "Non-ignored cases are discovered cases minus structurally marked ignored/skipped/todo cases; no case is executed by this analyzer.".to_owned(),
            "Dedicated test-file classification uses language-native path and filename conventions; inline tests remain in source-file rows.".to_owned(),
            "Same-stem module matching is intentionally conservative and false-negative-prone; it does not infer imports, package layout, generated tests, or integration-test ownership.".to_owned(),
            "Files with tree-sitter syntax errors are counted in syntax_error_files and analyzed using the error-tolerant tree, so their observations may be partial.".to_owned(),
            "Physical line counts include blank and comment lines and are denominators, not executable-line or statement coverage.".to_owned(),
        ],
    })
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        0
    } else {
        bytes.iter().filter(|byte| **byte == b'\n').count() as u64
            + u64::from(!bytes.ends_with(b"\n"))
    }
}

fn components(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
}

fn classify_file(file: &SourceFile) -> FileRole {
    let path = file.path.to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    let in_dir = |wanted: &[&str]| components(&path).any(|part| wanted.contains(&part));
    let is_test = match file.language {
        SourceLanguage::Rust => {
            in_dir(&["tests", "benches"])
                || name.ends_with("_test.rs")
                || name.ends_with("_tests.rs")
        }
        SourceLanguage::Python => {
            in_dir(&["test", "tests"])
                || name.starts_with("test_")
                || name.ends_with("_test.py")
                || name.ends_with("_tests.py")
        }
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            in_dir(&["test", "tests", "__tests__", "spec", "specs"])
                || [".test.", ".spec."]
                    .iter()
                    .any(|marker| name.contains(marker))
        }
        SourceLanguage::Go => name.ends_with("_test.go"),
    };
    if is_test {
        FileRole::Test
    } else {
        FileRole::Source
    }
}

fn is_source_module(file: &SourceFile) -> bool {
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    !matches!(
        name,
        "mod.rs"
            | "lib.rs"
            | "main.rs"
            | "__init__.py"
            | "index.js"
            | "index.jsx"
            | "index.ts"
            | "index.tsx"
    )
}

fn canonical_stem(file: &SourceFile, test: bool) -> String {
    let name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .to_ascii_lowercase();
    let mut stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem)
        .to_owned();
    if test {
        for suffix in [".test", ".spec", "_tests", "_test"] {
            if stem.ends_with(suffix) {
                stem.truncate(stem.len() - suffix.len());
                break;
            }
        }
        if let Some(stripped) = stem.strip_prefix("test_") {
            stem = stripped.to_owned();
        }
    }
    stem
}

fn same_stem_matches(
    source: &[&SourceFile],
    tests: &[&SourceFile],
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut by_stem: BTreeMap<String, Vec<&SourceFile>> = BTreeMap::new();
    for file in source {
        by_stem
            .entry(canonical_stem(file, false))
            .or_default()
            .push(file);
    }
    let mut matched_sources = BTreeSet::new();
    let mut matched_tests = BTreeSet::new();
    for test in tests {
        if let Some(candidates) = by_stem.get(&canonical_stem(test, true))
            && candidates.len() == 1
        {
            matched_sources.insert(candidates[0].path.clone());
            matched_tests.insert(test.path.clone());
        }
    }
    (matched_sources, matched_tests)
}

fn inspect(file: &SourceFile, root: Node<'_>) -> Observations {
    let mut observations = Observations::default();
    let mut ranges = Vec::new();
    discover_cases(file, root, &mut observations, &mut ranges);
    discover_assertions(file, root, &mut observations);
    if file.language == SourceLanguage::Go {
        for range in &mut ranges {
            if contains_go_skip(file, root, range.start, range.end) {
                range.ignored = true;
            }
        }
        observations.ignored = ranges.iter().filter(|range| range.ignored).count() as u64;
    }
    observations
}

fn discover_cases(
    file: &SourceFile,
    node: Node<'_>,
    out: &mut Observations,
    ranges: &mut Vec<CaseRange>,
) {
    match file.language {
        SourceLanguage::Rust if node.kind() == "function_item" => {
            let prefix = prefix_text(file, node.start_byte(), 512);
            if has_rust_attribute(prefix, "test") {
                let ignored = has_rust_attribute(prefix, "ignore");
                out.cases += 1;
                out.ignored += u64::from(ignored);
                ranges.push(CaseRange {
                    start: node.start_byte(),
                    end: node.end_byte(),
                    ignored,
                });
            }
        }
        SourceLanguage::Python if node.kind() == "function_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(file, n))
                .unwrap_or("");
            if name.starts_with("test_") {
                let prefix = prefix_text(file, node.start_byte(), 512);
                let ignored = [
                    "@skip",
                    "@unittest.skip",
                    "@unittest.expectedFailure",
                    "@pytest.mark.skip",
                    "@pytest.mark.xfail",
                ]
                .iter()
                .any(|marker| prefix.contains(marker));
                out.cases += 1;
                out.ignored += u64::from(ignored);
            }
        }
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx
            if node.kind() == "call_expression" =>
        {
            if let Some(function) = node.child_by_field_name("function") {
                let callee = compact(text(file, function));
                let base = callee.split('.').next().unwrap_or("");
                if matches!(base, "test" | "it" | "xtest" | "xit") {
                    out.cases += 1;
                    out.ignored += u64::from(
                        matches!(base, "xtest" | "xit")
                            || callee.contains(".skip")
                            || callee.contains(".todo")
                            || callee.contains(".disable"),
                    );
                } else if matches!(base, "describe" | "xdescribe") {
                    out.suites += 1;
                }
            }
        }
        SourceLanguage::Go if node.kind() == "function_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(file, n))
                .unwrap_or("");
            if go_case_name(name) {
                out.cases += 1;
                ranges.push(CaseRange {
                    start: node.start_byte(),
                    end: node.end_byte(),
                    ignored: false,
                });
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        discover_cases(file, child, out, ranges);
    }
}

fn discover_assertions(file: &SourceFile, node: Node<'_>, out: &mut Observations) {
    let assertion = match file.language {
        SourceLanguage::Rust => {
            node.kind() == "macro_invocation"
                && node.child(0).is_some_and(|n| {
                    matches!(
                        text(file, n).trim_end_matches('!'),
                        "assert"
                            | "assert_eq"
                            | "assert_ne"
                            | "debug_assert"
                            | "debug_assert_eq"
                            | "debug_assert_ne"
                    )
                })
        }
        SourceLanguage::Python => {
            node.kind() == "assert_statement"
                || (node.kind() == "call"
                    && node.child_by_field_name("function").is_some_and(|n| {
                        let callee = text(file, n);
                        callee == "assert"
                            || callee.starts_with("assert_")
                            || callee
                                .rsplit('.')
                                .next()
                                .is_some_and(|name| name.starts_with("assert"))
                    }))
        }
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            node.kind() == "call_expression"
                && node.child_by_field_name("function").is_some_and(|n| {
                    let callee = compact(text(file, n));
                    matches!(
                        callee.split('.').next().unwrap_or(""),
                        "expect" | "assert" | "assertThat"
                    )
                })
        }
        SourceLanguage::Go => {
            node.kind() == "call_expression"
                && node.child_by_field_name("function").is_some_and(|n| {
                    let callee = text(file, n);
                    let name = callee.rsplit('.').next().unwrap_or(callee);
                    matches!(
                        name,
                        "Error"
                            | "Errorf"
                            | "Fatal"
                            | "Fatalf"
                            | "Fail"
                            | "FailNow"
                            | "Equal"
                            | "NotEqual"
                            | "True"
                            | "False"
                            | "NoError"
                            | "ErrorIs"
                    )
                })
        }
    };
    out.assertions += u64::from(assertion);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        discover_assertions(file, child, out);
    }
}

fn contains_go_skip(file: &SourceFile, node: Node<'_>, start: usize, end: usize) -> bool {
    if node.start_byte() >= start
        && node.end_byte() <= end
        && node.kind() == "call_expression"
        && node.child_by_field_name("function").is_some_and(|n| {
            matches!(
                text(file, n).rsplit('.').next(),
                Some("Skip" | "Skipf" | "SkipNow")
            )
        })
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        child.end_byte() >= start
            && child.start_byte() <= end
            && contains_go_skip(file, child, start, end)
    })
}

fn go_case_name(name: &str) -> bool {
    ["Test", "Benchmark", "Example"].iter().any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|rest| {
            rest.is_empty() || rest.chars().next().is_some_and(|c| !c.is_ascii_lowercase())
        })
    })
}

fn text<'a>(file: &'a SourceFile, node: Node<'_>) -> &'a str {
    std::str::from_utf8(&file.bytes[node.byte_range()]).unwrap_or("")
}

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn prefix_text(file: &SourceFile, offset: usize, limit: usize) -> &str {
    let start = offset.saturating_sub(limit);
    std::str::from_utf8(&file.bytes[start..offset]).unwrap_or("")
}

fn has_rust_attribute(prefix: &str, name: &str) -> bool {
    prefix
        .lines()
        .rev()
        .take_while(|line| {
            let trimmed = line.trim();
            trimmed.is_empty() || trimmed.starts_with("#")
        })
        .any(|line| {
            let compacted = compact(line);
            compacted == format!("#[{name}]")
                || compacted.starts_with(&format!("#[{name}("))
                || compacted.starts_with(&format!("#[{name}="))
        })
}

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::duplicates::{DuplicateConfig, DuplicateError, analyze_duplicates};
use tempfile::TempDir;

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create duplicate fixture parent");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write duplicate fixture {}: {error}", path.display()));
}

fn structural_fixture() -> TempDir {
    let directory = TempDir::new().expect("temporary structural-clone repository");
    let root = directory.path();
    write_file(
        root,
        "src/alpha.rs",
        r#"pub fn sum_positive(values: &[i32]) -> i32 {
    let mut total = 0;
    for value in values.iter() {
        if *value > 0 {
            total += *value;
        }
    }
    total
}
"#,
    );
    write_file(
        root,
        "src/beta.rs",
        r#"// Names, literals, whitespace, and comments are deliberately different.
pub fn accumulate_nonzero ( entries : &[i32] ) -> i32 {
    let mut answer=17; // literal normalization must not split this clone
    for entry in entries . iter ( ) {
        /* formatting is not structure */ if *entry > 999 {
            answer += *entry;
        }
    }
    answer
}
"#,
    );
    write_file(
        root,
        "src/near_miss.rs",
        r#"pub fn almost_the_same(items: &[i32]) -> i32 {
    let mut result = 0;
    for item in items.iter() {
        if *item >= 0 {
            result += *item;
        }
    }
    result
}
"#,
    );
    write_file(
        root,
        "src/boilerplate.rs",
        "pub fn first() -> i32 { 1 }\npub fn second() -> i32 { 2 }\n",
    );
    directory
}

fn config(min_tokens: usize, min_lines: usize, max_groups: usize) -> DuplicateConfig {
    DuplicateConfig {
        min_tokens,
        min_lines,
        max_groups,
    }
}

fn normalized_paths(value: impl IntoIterator<Item = String>) -> Vec<String> {
    value
        .into_iter()
        .map(|path| path.replace('\\', "/"))
        .collect()
}

fn run_cli(root: &Path, trailing: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg("duplicates")
        .arg(root)
        .args(trailing)
        .output()
        .expect("run seval duplicates")
}

fn successful_cli(root: &Path, trailing: &[&str]) -> Output {
    let output = run_cli(root, trailing);
    assert!(
        output.status.success(),
        "seval duplicates failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn normalized_clone_is_maximal_stable_and_rejects_a_structural_near_miss() {
    let fixture = structural_fixture();

    // Precommitted smallest-case oracle: exactly the two structurally equal functions
    // form one clone. The changed comparison operator and the two tiny functions do not.
    let expected_paths = vec!["src/alpha.rs".to_owned(), "src/beta.rs".to_owned()];
    let expected_group_count = 1;
    let expected_occurrence_count = 2;

    let first = analyze_duplicates(fixture.path(), &config(38, 7, 20))
        .expect("analyze structural-clone fixture");
    let second = analyze_duplicates(fixture.path(), &config(38, 7, 20))
        .expect("repeat structural-clone analysis");

    assert_eq!(
        first.groups.len(),
        expected_group_count,
        "groups: {:#?}",
        first.groups
    );
    let group = &first.groups[0];
    assert_eq!(group.occurrences.len(), expected_occurrence_count);
    assert_eq!(
        normalized_paths(group.occurrences.iter().map(|row| row.path.clone())),
        expected_paths
    );
    assert!(
        group
            .occurrences
            .iter()
            .all(|row| row.end_line > row.start_line),
        "the accepted clone must span the multiline function, not boilerplate"
    );
    assert_eq!(first.totals.clone_groups, expected_group_count);
    assert_eq!(first.totals.clone_occurrences, expected_occurrence_count);

    let first_json = serde_json::to_value(&first).expect("serialize first duplicate report");
    let second_json = serde_json::to_value(&second).expect("serialize second duplicate report");
    assert_eq!(
        first_json, second_json,
        "ordering and digests must be deterministic"
    );
    assert_eq!(
        group.digest.len(),
        64,
        "digest must be a full SHA-256 hex receipt"
    );
    assert!(group.digest.bytes().all(|byte| byte.is_ascii_hexdigit()));

    // Positive control above proves the detector ran. Raising the token threshold beyond
    // the entire fixture must reject every candidate rather than returning partial noise.
    let above_threshold = analyze_duplicates(fixture.path(), &config(10_000, 7, 20))
        .expect("analyze fixture above its token threshold");
    assert!(above_threshold.groups.is_empty());
    assert_eq!(above_threshold.totals.clone_occurrences, 0);
}

#[test]
fn overlapping_windows_collapse_to_maximal_non_overlapping_occurrences_and_union_totals() {
    let directory = TempDir::new().expect("temporary repeated-token repository");
    write_file(
        directory.path(),
        "src/repeated.rs",
        r#"pub fn first(values: &[i32]) -> i32 {
    let mut total = 0;
    for value in values.iter() {
        total += *value;
    }
    total
}

pub fn second(entries: &[i32]) -> i32 {
    let mut answer = 1;
    for entry in entries.iter() {
        answer += *entry;
    }
    answer
}

pub fn third(items: &[i32]) -> i32 {
    let mut result = 9;
    for item in items.iter() {
        result += *item;
    }
    result
}
"#,
    );

    // Precommitted oracle for F F F: the many equal rolling windows represent three
    // maximal disjoint occurrences, not O(window-count²) overlapping clone reports.
    let expected_occurrences = 3;
    let expected_path = "src/repeated.rs";

    let report = analyze_duplicates(directory.path(), &config(30, 6, 20))
        .expect("analyze overlapping-window fixture");
    assert_eq!(report.groups.len(), 1, "groups: {:#?}", report.groups);
    let group = &report.groups[0];
    assert_eq!(group.occurrences.len(), expected_occurrences);
    assert!(
        group
            .occurrences
            .iter()
            .all(|row| row.path == expected_path)
    );

    for pair in group.occurrences.windows(2) {
        assert!(
            pair[0].end_line < pair[1].start_line,
            "maximal occurrences in one file must not overlap: {pair:?}"
        );
    }

    let expected_union_lines: usize = group
        .occurrences
        .iter()
        .map(|row| row.end_line - row.start_line + 1)
        .sum();
    assert_eq!(report.totals.duplicated_lines, expected_union_lines);
    assert_eq!(
        report.totals.duplicated_tokens,
        group.tokens_per_occurrence * expected_occurrences
    );
    assert_eq!(group.duplicated_token_mass, report.totals.duplicated_tokens);
    assert_eq!(group.duplicated_line_mass, report.totals.duplicated_lines);
}

#[test]
fn severity_order_is_deterministic_and_max_groups_bounds_rows_and_totals() {
    let directory = TempDir::new().expect("temporary bounded-groups repository");
    for (path, source) in [
        (
            "rust/a.rs",
            "pub fn alpha(xs: &[i32]) -> i32 {\n let mut n = 0;\n for x in xs.iter() {\n  n += *x;\n }\n n\n}\n",
        ),
        (
            "rust/b.rs",
            "pub fn beta(ys: &[i32]) -> i32 {\n let mut m = 4;\n for y in ys.iter() {\n  m += *y;\n }\n m\n}\n",
        ),
        (
            "js/a.js",
            "export function alpha(xs) {\n let n = 0;\n for (const x of xs) {\n  n = n * x;\n }\n return n;\n}\n",
        ),
        (
            "js/b.js",
            "export function beta(ys) {\n let m = 7;\n for (const y of ys) {\n  m = m * y;\n }\n return m;\n}\n",
        ),
    ] {
        write_file(directory.path(), path, source);
    }

    // Precommitted relation: independent Rust and JavaScript pairs produce at least two
    // groups; max_groups=1 retains exactly the first deterministically ranked group.
    let minimum_uncapped_groups = 2;
    let bounded_groups = 1;

    let all = analyze_duplicates(directory.path(), &config(24, 6, 20))
        .expect("analyze independent clone pairs");
    assert!(
        all.groups.len() >= minimum_uncapped_groups,
        "positive controls did not produce both language-specific groups: {:#?}",
        all.groups
    );
    let bounded = analyze_duplicates(directory.path(), &config(24, 6, bounded_groups))
        .expect("analyze bounded clone pairs");
    assert_eq!(bounded.groups.len(), bounded_groups);
    assert_eq!(bounded.groups[0].digest, all.groups[0].digest);
    assert_eq!(bounded.totals.clone_groups, bounded_groups);
    assert_eq!(
        bounded.totals.clone_occurrences,
        bounded.groups[0].occurrences.len(),
        "totals must describe only reported groups"
    );
    assert_eq!(
        bounded.totals.duplicated_tokens,
        bounded.groups[0].duplicated_token_mass
    );
    assert_eq!(
        bounded.totals.duplicated_lines,
        bounded.groups[0].duplicated_line_mass
    );

    let ranking: Vec<_> = all
        .groups
        .iter()
        .map(|group| {
            (
                group.tokens_per_occurrence * group.occurrences.len(),
                group.occurrences[0].path.clone(),
                group.digest.clone(),
            )
        })
        .collect();
    for pair in ranking.windows(2) {
        assert!(
            pair[0].0 > pair[1].0
                || pair[0].0 == pair[1].0
                    && (pair[0].1.as_str(), pair[0].2.as_str())
                        <= (pair[1].1.as_str(), pair[1].2.as_str()),
            "groups violate documented mass/path/digest ordering: {ranking:?}"
        );
    }
}

#[test]
fn zero_configuration_values_fail_with_specific_errors() {
    let directory = TempDir::new().expect("temporary invalid-config repository");
    write_file(directory.path(), "source.rs", "fn value() -> i32 { 1 }\n");

    let cases = [
        (
            config(0, 1, 1),
            "min_tokens",
            DuplicateError::InvalidMinTokens,
        ),
        (
            config(1, 0, 1),
            "min_lines",
            DuplicateError::InvalidMinLines,
        ),
        (
            config(1, 1, 0),
            "max_groups",
            DuplicateError::InvalidMaxGroups,
        ),
    ];
    for (invalid, field, expected) in cases {
        let error = analyze_duplicates(directory.path(), &invalid)
            .expect_err("zero duplicate configuration must fail");
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected),
            "wrong error for zero {field}: {error}"
        );
        assert!(error.to_string().contains(field));
    }
}

#[test]
fn cli_json_and_text_are_observations_without_judgment_fields() {
    let fixture = structural_fixture();
    let arguments = [
        "--min-tokens",
        "38",
        "--min-lines",
        "7",
        "--max-groups",
        "20",
    ];

    // Precommitted CLI oracle: the positive-control fixture emits one group with two
    // occurrences through either renderer, and JSON has no root-level judgment fields.
    let expected_groups = 1_u64;
    let expected_occurrences = 2_u64;
    let forbidden_fields: BTreeSet<_> = ["score", "quality_score", "verdict"].into_iter().collect();

    let mut json_arguments = arguments.to_vec();
    json_arguments.extend(["--format", "json"]);
    let json_output = successful_cli(fixture.path(), &json_arguments);
    assert!(json_output.stderr.is_empty());
    let json: Value = serde_json::from_slice(&json_output.stdout).unwrap_or_else(|error| {
        panic!(
            "duplicates JSON output is invalid: {error}; stdout={}",
            String::from_utf8_lossy(&json_output.stdout)
        )
    });
    let root = json
        .as_object()
        .expect("duplicates JSON root must be an object");
    assert!(forbidden_fields.is_disjoint(&root.keys().map(String::as_str).collect()));
    assert_eq!(
        json["totals"]["clone_groups"].as_u64(),
        Some(expected_groups)
    );
    assert_eq!(
        json["totals"]["clone_occurrences"].as_u64(),
        Some(expected_occurrences)
    );
    assert_eq!(
        json["groups"].as_array().map(Vec::len),
        Some(expected_groups as usize)
    );

    let mut text_arguments = arguments.to_vec();
    text_arguments.extend(["--format", "text"]);
    let text_output = successful_cli(fixture.path(), &text_arguments);
    assert!(text_output.stderr.is_empty());
    let text = String::from_utf8(text_output.stdout).expect("duplicates text must be UTF-8");
    assert!(text.contains("analyzer: tree-sitter normalized-token clone detector"));
    assert!(text.contains("thresholds: min-tokens=38 min-lines=7 max-groups=20"));
    assert!(text.contains("clones: 1 groups, 2 occurrences"));
    assert!(text.contains("src/alpha.rs:"));
    assert!(text.contains("src/beta.rs:"));
    assert!(!text.contains("near_miss.rs:"));
}

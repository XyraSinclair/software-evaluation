use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::metrics::{
    FunctionDistributions, MetricsComparison, MetricsComparisonSide, MetricsReport, analyze_path,
    compare_paths, compare_reports,
};
use tempfile::TempDir;

const LEFT_LIB: &str = r#"pub fn classify(value: i32) -> i32 {
    if value > 0 {
        1
    } else {
        0
    }
}
"#;

const RIGHT_LIB: &str = r#"pub fn classify(value: i32) -> i32 {
    if value > 0 {
        if value % 2 == 0 {
            2
        } else {
            1
        }
    } else {
        0
    }
}
"#;

const STRAIGHT_LINE: &str = r#"pub fn identity(value: i32) -> i32 {
    value
}
"#;

struct ComparisonFixture {
    _directory: TempDir,
    left: std::path::PathBuf,
    right: std::path::PathBuf,
}

impl ComparisonFixture {
    fn new() -> Self {
        let directory = TempDir::new().expect("temporary comparison fixture");
        let left = directory.path().join("left");
        let right = directory.path().join("right");

        // The expected identity sets are deliberately independent of directory insertion order.
        for (root, files) in [
            (
                &left,
                [
                    ("src/z_stable.rs", STRAIGHT_LINE),
                    ("src/old.rs", STRAIGHT_LINE),
                    ("src/lib.rs", LEFT_LIB),
                ],
            ),
            (
                &right,
                [
                    ("src/z_stable.rs", STRAIGHT_LINE),
                    ("src/new.rs", STRAIGHT_LINE),
                    ("src/lib.rs", RIGHT_LIB),
                ],
            ),
        ] {
            for (relative, contents) in files {
                write_file(root, relative, contents);
            }
        }

        Self {
            _directory: directory,
            left,
            right,
        }
    }
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture file has a parent"))
        .expect("create fixture source directory");
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write fixture {}: {error}", path.display()));
}

type Identity<'a> = (&'a str, &'a str);
type IdentityPartition<'a> = (Vec<Identity<'a>>, Vec<Identity<'a>>, Vec<Identity<'a>>);

fn identities(comparison: &MetricsComparison) -> IdentityPartition<'_> {
    (
        comparison
            .matched_files
            .iter()
            .map(|file| (file.path.as_str(), file.language.as_str()))
            .collect(),
        comparison
            .only_left
            .iter()
            .map(|file| (file.path.as_str(), file.language.as_str()))
            .collect(),
        comparison
            .only_right
            .iter()
            .map(|file| (file.path.as_str(), file.language.as_str()))
            .collect(),
    )
}

fn difference<'a>(
    comparison: &'a MetricsComparison,
    metric: &str,
) -> &'a software_evaluation::metrics::NumericDifference {
    comparison
        .differences
        .iter()
        .find(|difference| difference.metric == metric)
        .unwrap_or_else(|| panic!("missing scalar difference {metric:?}"))
}

fn assert_close(actual: f64, expected: f64, contract: &str) {
    let tolerance = f64::EPSILON * expected.abs().max(1.0) * 16.0;
    assert!(
        (actual - expected).abs() <= tolerance,
        "{contract}: expected {expected}, got {actual}"
    );
}

fn assert_rate(actual: Option<f64>, numerator: f64, denominator: usize, contract: &str) {
    let actual = actual.unwrap_or_else(|| panic!("{contract} must be present"));
    assert!(
        actual.is_finite(),
        "{contract} must be finite, got {actual}"
    );
    assert_close(actual, numerator / denominator as f64, contract);
}

fn assert_rates(side: &MetricsComparisonSide) {
    let summary = &side.summary;
    let rates = &side.rates;

    assert_rate(
        rates.functions_per_ksloc,
        summary.functions as f64 * 1_000.0,
        summary.sloc,
        "functions per KSLOC",
    );
    assert_rate(
        rates.cognitive_per_ksloc,
        summary.cognitive * 1_000.0,
        summary.sloc,
        "cognitive complexity per KSLOC",
    );
    assert_rate(
        rates.cyclomatic_per_ksloc,
        summary.cyclomatic * 1_000.0,
        summary.sloc,
        "cyclomatic complexity per KSLOC",
    );
    assert_rate(
        rates.arguments_per_function,
        summary.arguments as f64,
        summary.functions,
        "arguments per function",
    );
    assert_rate(
        rates.exits_per_function,
        summary.exits as f64,
        summary.functions,
        "exits per function",
    );
    assert_rate(
        rates.comment_fraction,
        summary.cloc as f64,
        summary.lines,
        "comment fraction",
    );
    assert_rate(
        rates.blank_fraction,
        summary.blank as f64,
        summary.lines,
        "blank fraction",
    );
}

fn nearest_rank(mut values: Vec<f64>, percentile: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    let rank = percentile.saturating_mul(values.len()).div_ceil(100);
    values[rank - 1]
}

fn assert_distribution(report: &MetricsReport, side: &MetricsComparisonSide) {
    let FunctionDistributions {
        cognitive,
        cyclomatic,
        sloc,
        arguments,
        exits,
    } = &side.distributions;
    let cases = [
        (
            "cognitive",
            report
                .functions
                .iter()
                .map(|function| function.cognitive)
                .collect::<Vec<_>>(),
            cognitive,
        ),
        (
            "cyclomatic",
            report
                .functions
                .iter()
                .map(|function| function.cyclomatic)
                .collect::<Vec<_>>(),
            cyclomatic,
        ),
        (
            "sloc",
            report
                .functions
                .iter()
                .map(|function| function.sloc as f64)
                .collect::<Vec<_>>(),
            sloc,
        ),
        (
            "arguments",
            report
                .functions
                .iter()
                .map(|function| function.arguments as f64)
                .collect::<Vec<_>>(),
            arguments,
        ),
        (
            "exits",
            report
                .functions
                .iter()
                .map(|function| function.exits as f64)
                .collect::<Vec<_>>(),
            exits,
        ),
    ];

    for (name, values, distribution) in cases {
        assert_eq!(
            distribution.count,
            values.len(),
            "{name} distribution count"
        );
        for (percentile, actual) in [
            (50, distribution.p50),
            (90, distribution.p90),
            (99, distribution.p99),
        ] {
            let actual = actual.unwrap_or_else(|| panic!("{name} p{percentile} must be present"));
            assert!(actual.is_finite(), "{name} p{percentile} must be finite");
            assert_close(
                actual,
                nearest_rank(values.clone(), percentile),
                &format!("{name} p{percentile} nearest-rank value"),
            );
        }
    }
}

fn run_cli(left: &Path, right: &Path, trailing: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg("metrics-compare")
        .arg(left)
        .arg(right)
        .args(trailing)
        .output()
        .expect("run compiled seval metrics-compare")
}

fn successful_json(left: &Path, right: &Path, top: usize) -> Value {
    let output = run_cli(
        left,
        right,
        &["--top-files", &top.to_string(), "--format", "json"],
    );
    assert!(
        output.status.success(),
        "metrics-compare JSON failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "metrics-compare emitted invalid JSON: {error}; stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn json_identities(value: &Value, field: &str) -> Vec<(String, String)> {
    value[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field:?} must be an array: {value}"))
        .iter()
        .map(|row| {
            (
                row["path"].as_str().expect("identity path").to_owned(),
                row["language"]
                    .as_str()
                    .expect("identity language")
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn public_comparison_apis_preserve_signed_totals_identities_rates_and_distributions() {
    // Precommitted fixture oracle: one changed and one unchanged identity match by path+language;
    // old.rs exists only on the left and new.rs only on the right.
    let expected_matched = vec![("src/lib.rs", "rust"), ("src/z_stable.rs", "rust")];
    let expected_only_left = vec![("src/old.rs", "rust")];
    let expected_only_right = vec![("src/new.rs", "rust")];
    let fixture = ComparisonFixture::new();

    let left_report = analyze_path(&fixture.left).expect("analyze left source tree");
    let right_report = analyze_path(&fixture.right).expect("analyze right source tree");
    let comparison = compare_reports(&left_report, &right_report);

    assert_eq!(
        identities(&comparison),
        (
            expected_matched.clone(),
            expected_only_left.clone(),
            expected_only_right.clone()
        ),
        "comparison identities and their deterministic order"
    );
    let lib = &comparison.matched_files[0];
    assert!(
        lib.right_minus_left.cognitive > 0.0,
        "the added nested branch must produce a positive right-minus-left cognitive delta"
    );
    assert_close(
        lib.right_minus_left.cognitive,
        lib.right.cognitive - lib.left.cognitive,
        "matched-file cognitive delta",
    );

    let scalar_expectations = [
        (
            "files",
            comparison.left.summary.files as f64,
            comparison.right.summary.files as f64,
        ),
        (
            "functions",
            comparison.left.summary.functions as f64,
            comparison.right.summary.functions as f64,
        ),
        (
            "sloc",
            comparison.left.summary.sloc as f64,
            comparison.right.summary.sloc as f64,
        ),
        (
            "ploc",
            comparison.left.summary.ploc as f64,
            comparison.right.summary.ploc as f64,
        ),
        (
            "lloc",
            comparison.left.summary.lloc as f64,
            comparison.right.summary.lloc as f64,
        ),
        (
            "cloc",
            comparison.left.summary.cloc as f64,
            comparison.right.summary.cloc as f64,
        ),
        (
            "cognitive",
            comparison.left.summary.cognitive,
            comparison.right.summary.cognitive,
        ),
        (
            "cyclomatic",
            comparison.left.summary.cyclomatic,
            comparison.right.summary.cyclomatic,
        ),
        (
            "modified_cyclomatic",
            comparison.left.summary.modified_cyclomatic,
            comparison.right.summary.modified_cyclomatic,
        ),
        (
            "arguments",
            comparison.left.summary.arguments as f64,
            comparison.right.summary.arguments as f64,
        ),
        (
            "exits",
            comparison.left.summary.exits as f64,
            comparison.right.summary.exits as f64,
        ),
    ];
    for (metric, expected_left, expected_right) in scalar_expectations {
        let actual = difference(&comparison, metric);
        assert_close(actual.left, expected_left, &format!("{metric} left total"));
        assert_close(
            actual.right,
            expected_right,
            &format!("{metric} right total"),
        );
        assert_close(
            actual.right_minus_left,
            expected_right - expected_left,
            &format!("{metric} right-minus-left total"),
        );
    }
    let zero_left = difference(&comparison, "cloc");
    assert_eq!(
        zero_left.left, 0.0,
        "fixture precondition for zero baseline"
    );
    assert!(
        zero_left.relative_change_from_left.is_none(),
        "relative change is undefined when the left value is zero"
    );

    assert_rates(&comparison.left);
    assert_rates(&comparison.right);
    assert_distribution(&left_report, &comparison.left);
    assert_distribution(&right_report, &comparison.right);
    assert!(
        comparison
            .limitations
            .iter()
            .any(|limitation| limitation.contains("right minus left")
                && limitation.contains("no intrinsic good/bad direction")),
        "comparison must state sign convention without assigning quality direction"
    );

    let path_comparison = compare_paths(&fixture.left, &fixture.right)
        .expect("compare two directory paths through the public API");
    assert_eq!(
        identities(&path_comparison),
        (expected_matched, expected_only_left, expected_only_right),
        "compare_paths directory analysis must reproduce the precommitted identities"
    );

    let file_comparison = compare_paths(
        &fixture.left.join("src/lib.rs"),
        &fixture.right.join("src/lib.rs"),
    )
    .expect("compare two file paths through the public API");
    assert_eq!(
        identities(&file_comparison),
        (vec![("lib.rs", "rust")], vec![], vec![]),
        "single-file inputs use their common root-relative filename"
    );
    assert!(file_comparison.matched_files[0].right_minus_left.cognitive > 0.0);
}

#[test]
fn cli_json_truncates_only_matched_rows_and_text_disclaims_quality_direction() {
    let expected_matched = [
        ("src/lib.rs".to_owned(), "rust".to_owned()),
        ("src/z_stable.rs".to_owned(), "rust".to_owned()),
    ];
    let expected_only_left = vec![("src/old.rs".to_owned(), "rust".to_owned())];
    let expected_only_right = vec![("src/new.rs".to_owned(), "rust".to_owned())];
    let fixture = ComparisonFixture::new();

    let top_zero = successful_json(&fixture.left, &fixture.right, 0);
    let root = top_zero
        .as_object()
        .expect("metrics-compare JSON root must be an object");
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(
            !root.contains_key(forbidden),
            "comparison JSON must not assign composite judgment field {forbidden:?}"
        );
    }
    assert_eq!(top_zero["matched_files_shown"], 0);
    assert_eq!(top_zero["matched_files_total"], 2);
    assert_eq!(
        json_identities(&top_zero, "matched_files"),
        Vec::<(String, String)>::new()
    );
    assert_eq!(json_identities(&top_zero, "only_left"), expected_only_left);
    assert_eq!(
        json_identities(&top_zero, "only_right"),
        expected_only_right
    );
    assert!(top_zero["left"]["summary"].is_object());
    assert!(top_zero["right"]["summary"].is_object());
    assert!(
        top_zero["differences"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        top_zero["limitations"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );

    let top_one = successful_json(&fixture.left, &fixture.right, 1);
    assert_eq!(top_one["matched_files_shown"], 1);
    assert_eq!(top_one["matched_files_total"], 2);
    assert_eq!(
        json_identities(&top_one, "matched_files"),
        expected_matched[..1].to_vec(),
        "top one preserves deterministic comparison order"
    );
    assert_eq!(json_identities(&top_one, "only_left"), expected_only_left);
    assert_eq!(json_identities(&top_one, "only_right"), expected_only_right);

    let text = run_cli(&fixture.left, &fixture.right, &["--top-files", "1"]);
    assert!(
        text.status.success(),
        "metrics-compare text failed with {:?}: {}",
        text.status.code(),
        String::from_utf8_lossy(&text.stderr)
    );
    let stdout = String::from_utf8(text.stdout).expect("text output is UTF-8");
    assert!(
        stdout.contains("right - left; no quality direction"),
        "text output must explain the sign convention without declaring a winner: {stdout}"
    );
}

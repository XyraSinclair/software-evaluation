use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::tests_analysis::{FileRole, analyze_tests};
use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
struct ExpectedRow {
    path: &'static str,
    language: &'static str,
    role: &'static str,
    lines: u64,
    syntax_errors: bool,
    cases: u64,
    ignored: u64,
    suites: u64,
    assertions: u64,
}

const EXPECTED_ROWS: &[ExpectedRow] = &[
    ExpectedRow {
        path: "go/empty_test.go",
        language: "go",
        role: "test",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "go/logic.go",
        language: "go",
        role: "source",
        lines: 3,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "go/logic_test.go",
        language: "go",
        role: "test",
        lines: 7,
        syntax_errors: false,
        cases: 2,
        ignored: 1,
        suites: 0,
        assertions: 2,
    },
    ExpectedRow {
        path: "go/solo.go",
        language: "go",
        role: "source",
        lines: 2,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "py/solo.py",
        language: "python",
        role: "source",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "py/worker.py",
        language: "python",
        role: "source",
        lines: 2,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "src/orphan.rs",
        language: "rust",
        role: "source",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "src/rust_unit.rs",
        language: "rust",
        role: "source",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "tests/empty_tests.rs",
        language: "rust",
        role: "test",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "tests/rust_unit_test.rs",
        language: "rust",
        role: "test",
        lines: 5,
        syntax_errors: false,
        cases: 2,
        ignored: 1,
        suites: 0,
        assertions: 2,
    },
    ExpectedRow {
        path: "tests/test_empty.py",
        language: "python",
        role: "test",
        lines: 2,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "tests/test_worker.py",
        language: "python",
        role: "test",
        lines: 6,
        syntax_errors: false,
        cases: 2,
        ignored: 1,
        suites: 0,
        assertions: 2,
    },
    ExpectedRow {
        path: "web/empty.spec.ts",
        language: "typescript",
        role: "test",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "web/index.js",
        language: "javascript",
        role: "source",
        lines: 1,
        syntax_errors: true,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "web/solo.ts",
        language: "typescript",
        role: "source",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
    ExpectedRow {
        path: "web/widget.test.ts",
        language: "typescript",
        role: "test",
        lines: 4,
        syntax_errors: false,
        cases: 2,
        ignored: 1,
        suites: 1,
        assertions: 1,
    },
    ExpectedRow {
        path: "web/widget.ts",
        language: "typescript",
        role: "source",
        lines: 1,
        syntax_errors: false,
        cases: 0,
        ignored: 0,
        suites: 0,
        assertions: 0,
    },
];

const EXPECTED_UNMATCHED_SOURCES: &[&str] =
    &["go/solo.go", "py/solo.py", "src/orphan.rs", "web/solo.ts"];
const EXPECTED_UNMATCHED_TESTS: &[&str] = &[
    "go/empty_test.go",
    "tests/empty_tests.rs",
    "tests/test_empty.py",
    "web/empty.spec.ts",
];

struct PolyglotFixture {
    directory: TempDir,
}

impl PolyglotFixture {
    fn new() -> Self {
        let directory = TempDir::new().expect("temporary test-analysis fixture");
        let root = directory.path();
        let mut files = vec![
            ("go/empty_test.go", "package sample\n"),
            (
                "go/logic.go",
                "package sample\n\nfunc Logic() int { return 1 }\n",
            ),
            (
                "go/logic_test.go",
                "package sample\n\nimport \"testing\"\n\nfunc TestGoRuns(t *testing.T) { t.Error(\"seed\") }\n\nfunc TestGoSkipped(t *testing.T) { t.Skip(\"seed\"); t.Fatal(\"unreachable\") }\n",
            ),
            ("go/solo.go", "package sample\nfunc Solo() {}\n"),
            ("notes.txt", "unsupported positive denominator\n"),
            ("py/solo.py", "def solo(): return 0\n"),
            ("py/worker.py", "def work():\n    return 1\n"),
            ("src/orphan.rs", "pub fn orphan() {}\n"),
            ("src/rust_unit.rs", "pub fn calculate() -> i32 { 1 }\n"),
            ("tests/empty_tests.rs", "fn helper() {}\n"),
            (
                "tests/rust_unit_test.rs",
                "#[test]\nfn rust_runs() { assert_eq!(1, 1); }\n#[test]\n#[ignore]\nfn rust_ignored() { assert!(false); }\n",
            ),
            ("tests/test_empty.py", "def helper():\n    pass\n"),
            (
                "tests/test_worker.py",
                "def test_python_runs():\n    assert 1 == 1\n\n@pytest.mark.skip\ndef test_python_skipped():\n    helper.assertEqual(1, 1)\n",
            ),
            ("web/empty.spec.ts", "export const helper = 1;\n"),
            ("web/index.js", "export function broken( {\n"),
            ("web/solo.ts", "export const solo = 0;\n"),
            (
                "web/widget.test.ts",
                "describe(\"widget\", () => {\n  test(\"runs\", () => { expect(1).toBe(1); });\n  test.todo(\"later\");\n});\n",
            ),
            ("web/widget.ts", "export const widget = 1;\n"),
        ];
        files.sort_unstable_by(|left, right| right.0.cmp(left.0));
        for (relative, contents) in files {
            write_file(root, relative, contents);
        }
        Self { directory }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write fixture {}: {error}", path.display()));
}

fn run_cli(root: &Path, trailing: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg("tests")
        .arg(root)
        .args(trailing)
        .output()
        .expect("run seval tests")
}

fn successful_cli(root: &Path, trailing: &[&str]) -> Output {
    let output = run_cli(root, trailing);
    assert!(
        output.status.success(),
        "seval tests failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn assert_no_root_judgment(value: &Value) {
    let root = value
        .as_object()
        .expect("test report root must be an object");
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(
            !root.contains_key(forbidden),
            "observational report must not emit root judgment field {forbidden:?}"
        );
    }
}

#[test]
fn public_analysis_reports_exact_polyglot_observations_and_conservative_matches() {
    // These expected relations are fixed before production is invoked: each detector
    // has two seeded cases, exactly one ignored case, and at least one assertion proxy.
    let expected_cases_by_language = BTreeMap::from([
        ("go", (2_u64, 1_u64, 2_u64)),
        ("python", (2, 1, 2)),
        ("rust", (2, 1, 2)),
        ("typescript", (2, 1, 1)),
    ]);
    let fixture = PolyglotFixture::new();
    let report = analyze_tests(fixture.path()).expect("analyze seeded polyglot fixture");

    let paths = report
        .files
        .iter()
        .map(|row| row.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        EXPECTED_ROWS.iter().map(|row| row.path).collect::<Vec<_>>()
    );
    assert!(report.unknown_files.is_empty());

    for (actual, expected) in report.files.iter().zip(EXPECTED_ROWS) {
        let role = match actual.role {
            FileRole::Source => "source",
            FileRole::Test => "test",
        };
        assert_eq!(actual.path, expected.path);
        assert_eq!(
            actual.language.name(),
            expected.language,
            "language for {}",
            expected.path
        );
        assert_eq!(role, expected.role, "role for {}", expected.path);
        assert_eq!(
            actual.lines, expected.lines,
            "line denominator for {}",
            expected.path
        );
        assert_eq!(
            actual.syntax_errors, expected.syntax_errors,
            "syntax status for {}",
            expected.path
        );
        assert_eq!(
            actual.discovered_test_cases, expected.cases,
            "case count for {}",
            expected.path
        );
        assert_eq!(
            actual.ignored_test_cases, expected.ignored,
            "ignored count for {}",
            expected.path
        );
        assert_eq!(
            actual.suite_declarations, expected.suites,
            "suite count for {}",
            expected.path
        );
        assert_eq!(
            actual.assertion_like_calls, expected.assertions,
            "assertion count for {}",
            expected.path
        );
    }

    let coverage = &report.coverage;
    assert_eq!(coverage.enumerated_files, 18);
    assert_eq!(coverage.supported_files, 17);
    assert_eq!(coverage.skipped_unsupported_files, 1);
    assert_eq!(coverage.analyzed_source_files, 9);
    assert_eq!(coverage.analyzed_source_lines, 13);
    assert_eq!(coverage.test_files, 8);
    assert_eq!(coverage.test_lines, 27);
    assert_eq!(coverage.syntax_error_files, 1);
    assert_eq!(coverage.discovered_test_cases, 8);
    assert_eq!(coverage.ignored_test_cases, 4);
    assert_eq!(coverage.non_ignored_test_cases, 4);
    assert_eq!(
        coverage.discovered_test_cases,
        coverage.ignored_test_cases + coverage.non_ignored_test_cases
    );
    assert_eq!(coverage.assertion_like_calls, 7);
    assert_eq!(coverage.source_modules_considered, 8);
    assert_eq!(coverage.source_modules_with_same_stem_test, 4);
    assert_eq!(coverage.source_modules_without_same_stem_test, 4);
    assert_eq!(coverage.test_lines_per_source_line, Some(27.0 / 13.0));
    assert_eq!(coverage.test_cases_per_ksloc, Some(8_000.0 / 13.0));
    assert_eq!(report.unmatched_source_modules, EXPECTED_UNMATCHED_SOURCES);
    assert_eq!(report.unmatched_test_files, EXPECTED_UNMATCHED_TESTS);

    let mut observed_by_language = BTreeMap::<&str, (u64, u64, u64)>::new();
    for row in &report.files {
        let entry = observed_by_language.entry(row.language.name()).or_default();
        entry.0 += row.discovered_test_cases;
        entry.1 += row.ignored_test_cases;
        entry.2 += row.assertion_like_calls;
    }
    for (language, expected) in expected_cases_by_language {
        assert_eq!(
            observed_by_language.get(language),
            Some(&expected),
            "positive control for {language}"
        );
    }

    let zero_case_languages = report
        .files
        .iter()
        .filter(|row| row.role == FileRole::Test && row.discovered_test_cases == 0)
        .map(|row| row.language.name())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        zero_case_languages,
        BTreeSet::from(["go", "python", "rust", "typescript"])
    );

    let value = serde_json::to_value(&report).expect("serialize public test report");
    assert_no_root_judgment(&value);
}

#[test]
fn cli_json_preserves_denominators_and_text_labels_observations_without_a_verdict() {
    // Smallest-case oracle committed before either CLI invocation.
    let expected = (17_u64, 18_u64, 8_u64, 4_u64, 4_u64, 7_u64);
    let fixture = PolyglotFixture::new();

    let json_output = successful_cli(fixture.path(), &["--format", "json"]);
    let value: Value = serde_json::from_slice(&json_output.stdout)
        .expect("seval tests --format json must emit one JSON report");
    assert_no_root_judgment(&value);
    let coverage = &value["coverage"];
    assert_eq!(coverage["supported_files"].as_u64(), Some(expected.0));
    assert_eq!(coverage["enumerated_files"].as_u64(), Some(expected.1));
    assert_eq!(coverage["discovered_test_cases"].as_u64(), Some(expected.2));
    assert_eq!(coverage["ignored_test_cases"].as_u64(), Some(expected.3));
    assert_eq!(
        coverage["non_ignored_test_cases"].as_u64(),
        Some(expected.4)
    );
    assert_eq!(coverage["assertion_like_calls"].as_u64(), Some(expected.5));
    let json_paths = value["files"]
        .as_array()
        .expect("JSON files must be an array")
        .iter()
        .map(|row| row["path"].as_str().expect("file path"))
        .collect::<Vec<_>>();
    assert_eq!(
        json_paths,
        EXPECTED_ROWS.iter().map(|row| row.path).collect::<Vec<_>>()
    );

    let text_output = successful_cli(fixture.path(), &["--top", "0", "--format", "text"]);
    let text = String::from_utf8(text_output.stdout).expect("text report must be UTF-8");
    assert!(text.contains("coverage: 17 supported / 18 enumerated files; 1 skipped; source=9 files/13 lines; tests=8 files/27 lines; syntax-error-files=1"));
    assert!(
        text.contains(
            "test observations: 8 cases, 4 ignored, 4 non-ignored, 7 assertion-like calls;"
        )
    );
    assert!(text.contains("same-stem matching: 4 / 8 source modules matched; 4 unmatched source modules; 4 unmatched test files"));
    assert!(text.contains("test machinery files: 0 / 17 shown"));
    assert!(text.contains("unmatched source modules: 0 / 4 shown"));
    assert!(text.contains("unmatched test files: 0 / 4 shown"));
    assert!(
        !text.contains("score:"),
        "text report must not present an adequacy score"
    );
    assert!(
        !text.contains("quality-score:"),
        "text report must not present a quality verdict"
    );
    assert!(
        !text.contains("verdict:"),
        "text report must not present a verdict"
    );
}

#[test]
fn skipped_javascript_suites_mark_all_descendant_cases_ignored() {
    let directory = TempDir::new().expect("temporary skipped-suite fixture");
    write_file(
        directory.path(),
        "web/nested.test.ts",
        "describe.skip(\"disabled parent\", () => {\n  test(\"direct child\", () => { expect(1).toBe(1); });\n  describe(\"nested child suite\", () => {\n    it(\"deep descendant\", () => { expect(2).toBe(2); });\n  });\n});\n\nxdescribe(\"ignored parent\", () => {\n  test(\"xdescribe child\", () => { expect(3).toBe(3); });\n});\n\ndescribe(\"active parent\", () => {\n  test(\"active child\", () => { expect(4).toBe(4); });\n});\n",
    );

    let report = analyze_tests(directory.path()).expect("analyze skipped-suite fixture");
    let row = report
        .files
        .iter()
        .find(|row| row.path == "web/nested.test.ts")
        .expect("nested test file observation");

    assert_eq!(row.discovered_test_cases, 4);
    assert_eq!(
        row.ignored_test_cases, 3,
        "cases inherit skipped status from every ignored suite ancestor"
    );
    assert_eq!(report.coverage.ignored_test_cases, 3);
    assert_eq!(report.coverage.non_ignored_test_cases, 1);
}

#[test]
fn same_stem_matching_uses_module_relative_paths_not_basenames() {
    let directory = TempDir::new().expect("temporary path-aware matching fixture");
    for (path, contents) in [
        ("packages/alpha/widget.ts", "export const alpha = 1;\n"),
        (
            "packages/alpha/widget.test.ts",
            "test(\"alpha widget\", () => { expect(1).toBe(1); });\n",
        ),
        ("packages/beta/widget.ts", "export const beta = 2;\n"),
    ] {
        write_file(directory.path(), path, contents);
    }

    let report = analyze_tests(directory.path()).expect("analyze path-aware matching fixture");

    assert_eq!(report.coverage.source_modules_considered, 2);
    assert_eq!(report.coverage.source_modules_with_same_stem_test, 1);
    assert_eq!(report.unmatched_source_modules, ["packages/beta/widget.ts"]);
    assert!(report.unmatched_test_files.is_empty());
}

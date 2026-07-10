use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::metrics::analyze_path;
use tempfile::TempDir;

struct PolyglotFixture {
    directory: TempDir,
}

impl PolyglotFixture {
    fn new() -> Self {
        let directory = TempDir::new().expect("temporary polyglot repository");
        let root = directory.path();

        // Sorting before writing makes filesystem insertion order the reverse of the
        // deterministic lexical order required from the analyzer.
        let mut files = vec![
            (
                "src/zeta.rs",
                r#"pub fn rust_simple(value: i32) -> i32 {
    value + 1
}

pub fn rust_nested(value: i32) -> i32 {
    let mut total = 0;
    if value > 0 {
        for item in 0..value {
            if item % 2 == 0 {
                total += item;
            }
        }
    }
    total
}
"#,
            ),
            (
                "src/widget.tsx",
                r#"export function Widget(props: { label: string }) {
  return <span>{props.label}</span>;
}
"#,
            ),
            (
                "src/tool.py",
                r#"def python_simple(value):
    return value + 1
"#,
            ),
            (
                "src/script.js",
                r#"export function javascriptSimple(value) {
  return value + 1;
}
"#,
            ),
            (
                "src/main.go",
                r#"package sample

func goSimple(value int) int {
	return value + 1
}
"#,
            ),
            (
                "ignored/hidden.rs",
                r#"pub fn ignored_nested(value: i32) -> i32 {
    if value > 0 {
        if value > 1 {
            return value;
        }
    }
    0
}
"#,
            ),
            ("notes.txt", "unsupported source-like text\n"),
            (".gitignore", "ignored/\n"),
        ];
        files.sort_unstable_by(|left, right| right.0.cmp(left.0));
        for (relative, contents) in files {
            write_file(root, relative, contents);
        }

        #[cfg(unix)]
        std::os::unix::fs::symlink("src/zeta.rs", root.join("linked.rs"))
            .expect("create source-file symlink fixture");

        Self { directory }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent directory");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("failed to write fixture {}: {error}", path.display()));
}

fn report_value(root: &Path) -> Value {
    let report = analyze_path(root).expect("polyglot fixture must be analyzable");
    serde_json::to_value(report).expect("public metric report must serialize as JSON")
}

fn rows<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("report field {field:?} must be an array: {value}"))
}

fn string_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("field {field:?} must be a string: {value}"))
}

fn integer_field(value: &Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("field {field:?} must be a non-negative integer: {value}"))
}

fn number_field(value: &Value, field: &str) -> f64 {
    value
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("field {field:?} must be numeric: {value}"))
}

fn relative_paths(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .map(|row| string_field(row, "path").replace('\\', "/"))
        .collect()
}

fn run_cli(root: &Path, command: &str, trailing: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg(command)
        .arg(root.as_os_str())
        .args(trailing)
        .output()
        .unwrap_or_else(|error| panic!("failed to run seval {command}: {error}"))
}

fn successful_json(root: &Path, command: &str, trailing: &[&str]) -> Value {
    let output = run_cli(root, command, trailing);
    assert!(
        output.status.success(),
        "seval {command} failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "seval {command} did not emit valid JSON: {error}; stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_rank_metadata(value: &Value, root: &Path, expected_sort: &str, expected_total: usize) {
    let reported_root = string_field(value, "root");
    assert_eq!(
        Path::new(reported_root).file_name(),
        root.file_name(),
        "rank report root {reported_root:?} does not identify {}",
        root.display()
    );
    assert_eq!(string_field(value, "sort"), expected_sort);
    assert_eq!(integer_field(value, "total") as usize, expected_total);
    let row_count = rows(value, "rows").len();
    assert_eq!(integer_field(value, "shown") as usize, row_count);
    assert!(row_count <= expected_total);
    assert!(
        !string_field(value, "analyzer").trim().is_empty(),
        "rank report must identify its analyzer"
    );
}

#[test]
fn public_report_discovers_polyglot_sources_in_stable_order_and_excludes_non_sources() {
    let fixture = PolyglotFixture::new();
    let first = report_value(fixture.path());
    let second = report_value(fixture.path());

    let root = first
        .as_object()
        .expect("metric report root must be an object");
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(
            !root.contains_key(forbidden),
            "metric report must not emit composite judgment field {forbidden:?}"
        );
    }

    let file_rows = rows(&first, "files");
    let paths = relative_paths(file_rows);
    let mut sorted_paths = paths.clone();
    sorted_paths.sort();
    assert_eq!(
        paths, sorted_paths,
        "file order must be lexical rather than filesystem insertion order"
    );
    assert_eq!(paths, relative_paths(rows(&second, "files")));

    let path_set: BTreeSet<_> = paths.iter().map(String::as_str).collect();
    assert_eq!(
        path_set,
        BTreeSet::from([
            "src/main.go",
            "src/script.js",
            "src/tool.py",
            "src/widget.tsx",
            "src/zeta.rs",
        ]),
        "only supported, non-ignored, non-symlink source files belong in the report"
    );

    let languages: BTreeSet<_> = file_rows
        .iter()
        .map(|row| string_field(row, "language").to_ascii_lowercase())
        .collect();
    assert_eq!(
        languages,
        BTreeSet::from([
            "go".to_owned(),
            "javascript".to_owned(),
            "python".to_owned(),
            "rust".to_owned(),
            "typescript".to_owned(),
        ])
    );

    let function_order = rows(&first, "functions")
        .iter()
        .map(|row| {
            (
                string_field(row, "path").to_owned(),
                integer_field(row, "start_line"),
                string_field(row, "name").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let repeated_order = rows(&second, "functions")
        .iter()
        .map(|row| {
            (
                string_field(row, "path").to_owned(),
                integer_field(row, "start_line"),
                string_field(row, "name").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut lexically_sorted_functions = function_order.clone();
    lexically_sorted_functions.sort();
    assert_eq!(function_order, lexically_sorted_functions);
    assert_eq!(function_order, repeated_order);
}

#[test]
fn nested_control_flow_has_greater_cognitive_complexity_than_a_simple_function() {
    let fixture = PolyglotFixture::new();
    let report = report_value(fixture.path());
    let functions = rows(&report, "functions");
    let by_name = |name: &str| {
        functions
            .iter()
            .find(|row| string_field(row, "name") == name)
            .unwrap_or_else(|| panic!("missing function {name:?}: {functions:?}"))
    };

    let simple = by_name("rust_simple");
    let nested = by_name("rust_nested");
    assert_eq!(number_field(simple, "cognitive"), 0.0);
    assert!(
        number_field(nested, "cognitive") > number_field(simple, "cognitive"),
        "nested control flow must rank above straight-line code"
    );
}

#[test]
fn unsupported_single_file_returns_a_valid_zero_source_report() {
    let fixture = PolyglotFixture::new();
    let unsupported = fixture.path().join("notes.txt");
    let report = analyze_path(&unsupported)
        .expect("an existing unsupported file is a valid analysis with no source rows");
    let value = serde_json::to_value(report).expect("zero-source report must serialize");

    assert!(rows(&value, "files").is_empty());
    assert!(rows(&value, "functions").is_empty());
    assert_eq!(integer_field(&value["summary"], "files"), 0);
    assert_eq!(integer_field(&value["summary"], "functions"), 0);
    assert_eq!(integer_field(&value["coverage"], "analyzed"), 0);
    assert_eq!(integer_field(&value["coverage"], "skipped"), 1);
}

#[test]
fn cli_json_reports_are_parseable_ranked_and_honor_top_including_zero() {
    let fixture = PolyglotFixture::new();
    let root = fixture.path();
    let api = report_value(root);
    let expected_functions = rows(&api, "functions").len();
    let expected_files = rows(&api, "files").len();

    let metrics = successful_json(root, "metrics", &["--format", "json"]);
    assert!(metrics.is_object(), "metrics JSON root must be an object");
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(metrics.get(forbidden).is_none());
    }

    for sort in [
        "cognitive",
        "cyclomatic",
        "sloc",
        "arguments",
        "exits",
        "maintainability",
        "halstead-effort",
    ] {
        let ranked = successful_json(
            root,
            "functions",
            &["--sort", sort, "--top", "2", "--format", "json"],
        );
        assert_rank_metadata(&ranked, root, sort, expected_functions);
        assert_eq!(rows(&ranked, "rows").len(), expected_functions.min(2));
    }

    let top_function = successful_json(
        root,
        "functions",
        &["--sort", "cognitive", "--top", "1", "--format", "json"],
    );
    assert_eq!(rows(&top_function, "rows").len(), 1);
    assert_eq!(
        string_field(&rows(&top_function, "rows")[0], "name"),
        "rust_nested"
    );

    let files = successful_json(
        root,
        "files",
        &["--sort", "sloc", "--top", "2", "--format", "json"],
    );
    assert_rank_metadata(&files, root, "sloc", expected_files);
    assert_eq!(rows(&files, "rows").len(), expected_files.min(2));

    let no_files = successful_json(
        root,
        "files",
        &["--sort", "cognitive", "--top", "0", "--format", "json"],
    );
    assert_rank_metadata(&no_files, root, "cognitive", expected_files);
    assert!(rows(&no_files, "rows").is_empty());
}

#[test]
fn missing_paths_fail_in_the_public_api_and_cli() {
    let directory = TempDir::new().expect("temporary parent directory");
    let missing = PathBuf::from(directory.path()).join("does-not-exist");

    analyze_path(&missing).expect_err("missing analysis roots must fail");

    let output = run_cli(&missing, "metrics", &["--format", "json"]);
    assert!(
        !output.status.success(),
        "CLI accepted a missing analysis root"
    );
    assert!(
        !output.stderr.is_empty(),
        "CLI failure must explain the missing analysis root on stderr"
    );
}

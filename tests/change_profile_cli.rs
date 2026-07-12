#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

// Embedded newline exercises Git quoting/decoding while remaining legal on Darwin.
const RAW_PATH_HEX: &str = "7372632f7261770a6e616d652e7273";

struct Fixture {
    dir: TempDir,
    raw_path: PathBuf,
}

fn git<I, S>(root: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()
        .expect("Git must be installed for change-profile tests");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn write(root: &Path, path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture directory");
    }
    fs::write(path, bytes).expect("write fixture file");
}

fn commit(root: &Path, message: &str, timestamp: &str) {
    git(root, ["add", "--all"]);
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            message,
        ])
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()
        .expect("commit fixture");
    assert!(
        output.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("temporary Git fixture");
        let root = dir.path();
        git(
            root,
            [
                "-c",
                "init.defaultBranch=main",
                "init",
                "--quiet",
                "--template=",
            ],
        );
        let raw_path = PathBuf::from(OsString::from_vec(b"src/raw\nname.rs".to_vec()));

        write(root, "src/stable.rs", "pub fn stable() {}\n");
        write(root, "src/empty.rs", "");
        write(root, "src/old.rs", "pub fn old() {}\n");
        write(root, ".gitattributes", "src/forced.rs binary\n");
        // Path UTF-8-ness, not content decoding, controls the path denominator.
        write(root, "assets/blob.bin", [0, 0xff, 0xfe]);
        write(
            root,
            ".hidden/tracked.rs",
            "pub fn tracked_but_ignored() {}\n",
        );
        commit(root, "outside retained window", "2001-01-01T00:00:00Z");

        fs::rename(root.join("src/old.rs"), root.join("src/live.rs"))
            .expect("rename old path to current path");
        write(root, "src/live.rs", "pub fn live() -> i32 {\n    1\n}\n");
        write(root, "src/mixed.rs", "pub fn mixed() {}\n");
        write(root, &raw_path, "pub fn raw() {}\n");
        write(root, "src/forced.rs", "pub fn forced_binary_numstat() {}\n");
        commit(
            root,
            "delete old and add current paths",
            "2001-01-02T01:00:00Z",
        );

        write(root, "src/live.rs", "pub fn live() -> i32 {\n    2\n}\n");
        write(root, "src/mixed.rs", [0, 0xff, 0, 0xfe]);
        commit(root, "same UTC day text and binary", "2001-01-02T23:00:00Z");

        write(
            root,
            "src/live.rs",
            "pub fn live() -> i32 {\n    2\n}\npub fn extra() {}\n",
        );
        write(root, "src/mixed.rs", "pub fn mixed() {}\n");
        commit(root, "restore text and extend live", "2001-01-04T00:00:00Z");

        // Supported but untracked: committed-tree analysis must exclude it.
        write(
            root,
            ".hidden/untracked.rs",
            "pub fn untracked() -> ! { panic!() }\n",
        );
        Self { dir, raw_path }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
}

fn seval(root: &Path, format: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg("change-profile")
        .arg(root)
        .arg("--history-commits")
        .arg("3")
        .arg("--format")
        .arg(format)
        .args(extra)
        .output()
        .expect("run seval change-profile")
}

fn json(root: &Path, extra: &[&str]) -> Value {
    let output = seval(root, "json", extra);
    assert!(
        output.status.success(),
        "change-profile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("change-profile JSON")
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} must be an array: {value}"))
}

fn row_by_hex<'a>(rows: &'a [Value], hex: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["path_bytes_hex"] == hex)
        .unwrap_or_else(|| panic!("missing raw identity {hex}"))
}

fn row_by_path<'a>(rows: &'a [Value], path: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["path"] == path)
        .unwrap_or_else(|| panic!("missing row {path}"))
}

#[test]
fn json_matches_hand_computed_bounded_history_and_join_oracle() {
    let fixture = Fixture::new();
    let report = json(fixture.root(), &["--top", "1"]);

    assert_eq!(report["analyzer"], "seval-change-profile-v1");
    assert_eq!(report["history_coverage"]["requested_commits"], 3);
    assert_eq!(report["history_coverage"]["commits_analyzed"], 3);
    assert_eq!(report["history_coverage"]["truncated"], true);
    // 2001-01-02 01:00Z and 2001-01-04 00:00Z, derived independently from UTC epoch days.
    assert_eq!(report["artifact"]["kind"], "git-repository");
    assert_eq!(report["artifact"]["revision"].as_str().unwrap().len(), 40);
    assert_eq!(
        report["artifact"]["tree_digest"].as_str().unwrap().len(),
        40
    );
    assert!(
        report["history_coverage"]["git_version"]
            .as_str()
            .unwrap()
            .starts_with("git version ")
    );
    assert!(
        report["history_coverage"]["command"]
            .as_str()
            .unwrap()
            .contains("--no-renames")
    );
    assert_eq!(
        report["history_coverage"]["stdout_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert!(report["history_coverage"]["stdout_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        report["history_coverage"]["earliest_committer_unix_seconds"],
        978_397_200_i64
    );
    assert_eq!(
        report["history_coverage"]["latest_committer_unix_seconds"],
        978_566_400_i64
    );

    let rows = array(&report, "current_rows");
    assert_eq!(rows.len(), 7, "--top is presentation-only for JSON");
    let paths = rows
        .iter()
        .map(|r| r["path_bytes_hex"].as_str().unwrap())
        .collect::<Vec<_>>();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(
        paths, sorted,
        "current rows are ordered by raw Git path bytes"
    );
    assert!(rows.iter().all(|r| r["path"] != ".hidden/untracked.rs"));
    assert_eq!(row_by_path(rows, ".hidden/tracked.rs")["current_sloc"], 1);

    let live = row_by_path(rows, "src/live.rs");
    // Three numstat rows: +3, then +1/-1, then +1 = mass 6; one touch per commit.
    assert_eq!(live["commits_touched"], 3);
    assert_eq!(live["commit_touch_fraction"], 1.0);
    assert_eq!(
        live["active_change_days"], 2,
        "two commits share UTC day 2001-01-02"
    );
    assert_eq!(live["text_commits_touched"], 3);
    assert_eq!(live["binary_change_count"], 0);
    assert_eq!(live["line_change_mass"], 6);
    assert_eq!(live["line_change_mass_complete"], true);
    assert_eq!(live["current_sloc"], 4);
    assert_eq!(live["line_change_mass_per_current_sloc"], 1.5);
    assert_eq!(live["first_observed_change_unix_seconds"], 978_397_200_i64);
    assert_eq!(live["last_observed_change_unix_seconds"], 978_566_400_i64);
    assert_eq!(live["history_status"], "text");

    let mixed = row_by_path(rows, "src/mixed.rs");
    // Add is textual (+1); text->binary and binary->text are both binary numstat touches.
    assert_eq!(mixed["commits_touched"], 3);
    assert_eq!(mixed["active_change_days"], 2);
    assert_eq!(mixed["text_commits_touched"], 1);
    assert_eq!(mixed["binary_change_count"], 2);
    assert_eq!(mixed["line_change_mass"], 1);
    assert_eq!(mixed["line_change_mass_complete"], false);
    assert!(mixed["line_change_mass_per_current_sloc"].is_null());
    assert_eq!(mixed["history_status"], "text_and_binary");

    let stable = row_by_path(rows, "src/stable.rs");
    assert_eq!(stable["join_status"], "current_without_history");
    assert_eq!(stable["commits_touched"], 0);
    assert_eq!(stable["history_status"], "none");
    assert!(stable["first_observed_change_unix_seconds"].is_null());
    let empty = row_by_path(rows, "src/empty.rs");
    assert_eq!(empty["current_sloc"], 0);
    assert!(empty["cognitive_per_ksloc"].is_null());
    assert!(empty["line_change_mass_per_current_sloc"].is_null());
    let forced = row_by_path(rows, "src/forced.rs");
    assert_eq!(forced["commits_touched"], 1);
    assert_eq!(forced["text_commits_touched"], 0);
    assert_eq!(forced["binary_change_count"], 1);
    assert_eq!(forced["line_change_mass"], 0);
    assert_eq!(forced["line_change_mass_complete"], false);
    assert_eq!(forced["history_status"], "binary_only");
    assert!(forced["line_change_mass_per_current_sloc"].is_null());

    let raw = row_by_hex(rows, RAW_PATH_HEX);
    assert_eq!(raw["commits_touched"], 1);
    assert_eq!(raw["commit_touch_fraction"].as_f64(), Some(1.0 / 3.0));
    assert_eq!(raw["line_change_mass"], 1);
    assert_eq!(raw["path"], "src/raw\nname.rs");
    assert!(fixture.root().join(&fixture.raw_path).exists());

    let historical = array(&report, "history_only_rows");
    assert_eq!(historical.len(), 1);
    let old = row_by_path(historical, "src/old.rs");
    assert_eq!(old["line_change_mass"], 1);
    assert_eq!(old["commits_touched"], 1);
    assert!(
        old.get("current_sloc").is_none(),
        "history-only rows cannot fabricate static metrics"
    );

    let join = &report["join_coverage"];
    assert_eq!(join["current_analyzed_paths"], 7);
    assert_eq!(join["sampled_history_paths"], 5);
    assert_eq!(join["matched_paths"], 4);
    assert_eq!(join["current_without_history_paths"], 3);
    assert_eq!(join["historical_without_current_paths"], 1);
    assert_eq!(join["binary_touched_current_paths"], 2);
    assert_eq!(7_u64, 4 + 3, "current partition closes");
    assert_eq!(5_u64, 4 + 1, "history partition closes");

    let source = &report["source_coverage"];
    assert_eq!(
        source["tracked_regular_files"], 9,
        "seven supported blobs plus two unsupported blobs"
    );
    assert_eq!(
        source["utf8_path_regular_files"], 9,
        "raw Git path UTF-8 decodability defines this denominator, not blob contents"
    );
    assert_eq!(source["non_utf8_path_regular_files"], 0);
    assert_eq!(source["supported_source_files"], 7);
    assert_eq!(source["analyzed_source_files"], 7);
    assert_eq!(source["unsupported_regular_files"], 2);
    assert_eq!(source["syntax_error_files"], 0);
}

#[test]
fn profile_reads_committed_blob_when_skip_worktree_hides_modified_bytes() {
    let fixture = Fixture::new();
    git(
        fixture.root(),
        ["update-index", "--skip-worktree", "--", "src/live.rs"],
    );
    write(fixture.root(), "src/live.rs", "pub fn worktree_only() {}\n");

    let report = json(fixture.root(), &[]);
    let revision = String::from_utf8(git(fixture.root(), ["rev-parse", "HEAD"]).stdout)
        .expect("revision is UTF-8");
    assert_eq!(report["artifact"]["revision"], revision.trim());
    let live = row_by_path(array(&report, "current_rows"), "src/live.rs");
    assert_eq!(
        live["current_sloc"], 4,
        "profile must analyze the committed four-line blob"
    );
    assert_ne!(
        live["current_sloc"], 1,
        "skip-worktree bytes must not contaminate metrics"
    );
}

#[test]
fn profile_includes_committed_source_missing_from_sparse_worktree() {
    let fixture = Fixture::new();
    git(
        fixture.root(),
        ["update-index", "--skip-worktree", "--", "src/stable.rs"],
    );
    fs::remove_file(fixture.root().join("src/stable.rs")).expect("remove sparse worktree file");

    let report = json(fixture.root(), &[]);
    let stable = row_by_path(array(&report, "current_rows"), "src/stable.rs");
    assert_eq!(stable["current_sloc"], 1);
    assert_eq!(report["source_coverage"]["supported_source_files"], 7);
    assert_eq!(report["source_coverage"]["analyzed_source_files"], 7);
}

#[test]
fn json_is_complete_while_text_is_top_limited_and_deterministic() {
    let fixture = Fixture::new();
    let complete = json(fixture.root(), &["--top", "0"]);
    assert_eq!(array(&complete, "current_rows").len(), 7);
    assert_eq!(array(&complete, "history_only_rows").len(), 1);

    let first = seval(fixture.root(), "text", &["--top", "1"]);
    let second = seval(fixture.root(), "text", &["--top", "1"]);
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    let text = String::from_utf8(first.stdout).expect("text output is UTF-8");
    assert!(
        text.contains("src/live.rs"),
        "largest complete textual mass is shown"
    );
    assert!(
        !text.contains("src/stable.rs"),
        "top one does not leak extra current rows"
    );
}

#[test]
fn svg_is_static_accessible_and_exposes_every_status_and_language_facet() {
    let fixture = Fixture::new();
    let output = seval(fixture.root(), "svg", &[]);
    assert!(
        output.status.success(),
        "SVG failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let svg = String::from_utf8(output.stdout).expect("SVG is UTF-8");
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("role=\"img\""));
    assert!(svg.contains("<title>Change × structure profile</title>"));
    assert!(svg.contains("<desc>"));
    assert!(!svg.contains("<script"));
    assert!(!svg.contains("href=\"http") && !svg.contains("src=\"http"));
    for class in [
        "history-text",
        "history-text-and-binary",
        "history-binary-only",
        "no-history",
    ] {
        assert!(svg.contains(class), "missing status class {class}");
    }
    assert!(svg.contains("aria-label="));
    assert!(svg.contains("<title>"), "marks need native title fallbacks");
    assert!(
        svg.to_ascii_lowercase().contains("rust"),
        "Rust facet must not be sampled away"
    );
    for forbidden in ["score", "quality", "risk", "grade", "winner"] {
        assert!(
            !svg.to_ascii_lowercase()
                .contains(&format!("class=\"{forbidden}"))
        );
    }
}

#[test]
fn cat_file_batch_exceeds_pipe_capacity_then_svg_refuses_all_rows_without_sampling() {
    let dir = TempDir::new().expect("large SVG fixture");
    git(
        dir.path(),
        [
            "-c",
            "init.defaultBranch=main",
            "init",
            "--quiet",
            "--template=",
        ],
    );
    for index in 0..=5_000 {
        write(
            dir.path(),
            format!("src/f{index:04}.rs"),
            format!("pub fn f{index:04}() {{}}\n"),
        );
    }
    commit(
        dir.path(),
        "5001 current source rows",
        "2001-01-01T00:00:00Z",
    );
    // Batch request alone is at least 5,001 * 41 bytes (40-hex OID + LF) =
    // 205,041 bytes; blob headers and one-line bodies also exceed 128 KiB.
    // Exact completion therefore discriminates concurrent stdin writing/stdout draining
    // from the former sequential pipe deadlock.
    let report = json(dir.path(), &[]);
    let rows = array(&report, "current_rows");
    assert_eq!(rows.len(), 5_001);
    assert_eq!(report["source_coverage"]["supported_source_files"], 5_001);
    assert_eq!(report["source_coverage"]["analyzed_source_files"], 5_001);
    assert_eq!(
        rows.iter()
            .map(|row| row["current_sloc"].as_u64().unwrap())
            .sum::<u64>(),
        5_001,
        "every committed one-line blob must be analyzed exactly once",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_seval"))
        .args([
            "change-profile",
            dir.path().to_str().unwrap(),
            "--history-commits",
            "1",
            "--format",
            "svg",
        ])
        .output()
        .expect("run oversized SVG request");
    assert!(
        !output.status.success(),
        "SVG must reject rather than sample 5001 rows"
    );
    assert!(
        output.stdout.is_empty(),
        "refusal must not emit a partial SVG"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("5,000"));
}

#[test]
fn all_formats_reject_invalid_history_and_tracked_dirt() {
    let fixture = Fixture::new();
    for format in ["json", "text", "svg"] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_seval"))
            .args([
                "change-profile",
                fixture.root().to_str().unwrap(),
                "--history-commits",
                "0",
                "--format",
                format,
            ])
            .output()
            .expect("run invalid config");
        assert!(!invalid.status.success(), "{format} accepted zero history");
    }

    write(fixture.root(), "src/live.rs", "pub fn dirty() {}\n");
    for format in ["json", "text", "svg"] {
        let dirty = seval(fixture.root(), format, &[]);
        assert!(
            !dirty.status.success(),
            "{format} analyzed a dirty tracked file"
        );
        assert!(
            dirty.stdout.is_empty(),
            "failure must not emit a partial {format} report"
        );
    }
}

#[test]
fn forbidden_judgment_vocabulary_is_absent_from_json_keys() {
    let fixture = Fixture::new();
    let report = json(fixture.root(), &[]);
    fn walk(value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let lower = key.to_ascii_lowercase();
                    for forbidden in ["score", "quality", "risk", "grade", "winner"] {
                        assert!(!lower.contains(forbidden), "forbidden judgment key {key:?}");
                    }
                    walk(child);
                }
            }
            Value::Array(values) => values.iter().for_each(walk),
            _ => {}
        }
    }
    walk(&report);
}

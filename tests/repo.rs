use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use approx::assert_abs_diff_eq;
use sha2::{Digest, Sha256};
use software_evaluation::kernel::{
    BeliefState, CriterionProgram, DecisionSpec, ProgramStatus, ResourceBudget, StopReason,
    evaluate_pipeline,
};
use software_evaluation::repo::{
    GitChangeShape, GitChangeShapeProgram, RepoError, RepoProfileConfig, StaticRepoShape,
    StaticRepoShapeProgram, snapshot_git_repo,
};
use tempfile::TempDir;

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
        .expect("Git must be installed for repository program tests");
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn init_repo() -> TempDir {
    let directory = TempDir::new().expect("temporary repository directory");
    git(
        directory.path(),
        [
            "-c",
            "init.defaultBranch=main",
            "init",
            "--quiet",
            "--template=",
        ],
    );
    directory
}

fn write_file(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent directory");
    }
    fs::write(path, bytes).expect("write repository fixture");
}

fn commit_all(root: &Path, message: &str, timestamp: &str) {
    git(root, ["add", "--all"]);
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Repository Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "--message",
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
        .expect("run fixture commit");
    assert!(
        output.status.success(),
        "fixture commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn decision() -> DecisionSpec {
    DecisionSpec {
        id: "repository-evaluation".to_owned(),
        description: "Measure committed repository structure".to_owned(),
        claim_ids: vec!["repository-shape".to_owned()],
    }
}

fn beliefs() -> BeliefState {
    BeliefState {
        probabilities: BTreeMap::from([("repository-shape".to_owned(), 0.5)]),
        observation_digests: Vec::new(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn observation<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> T {
    serde_json::from_value(value.clone()).expect("program observation must match its public type")
}

#[test]
fn snapshot_allows_ignored_untracked_files_but_rejects_tracked_changes() {
    let repository = init_repo();
    write_file(repository.path(), ".gitignore", "ignored.log\n");
    write_file(
        repository.path(),
        "src/lib.rs",
        "pub fn value() -> u8 { 1 }\n",
    );
    commit_all(repository.path(), "initial", "2001-01-01T00:00:00Z");

    write_file(repository.path(), "ignored.log", "local output\n");
    let clean = snapshot_git_repo(repository.path())
        .expect("ignored untracked files must not invalidate a committed snapshot");
    assert_eq!(clean.kind, "git-repository");
    let expected_revision = git(repository.path(), ["rev-parse", "HEAD"]);
    let tree_spec = format!("{}^{{tree}}", clean.revision);
    let expected_tree = git(repository.path(), ["rev-parse", tree_spec.as_str()]);
    assert_eq!(
        clean.revision,
        String::from_utf8(expected_revision.stdout)
            .expect("Git object ID is UTF-8")
            .trim()
    );
    assert_eq!(
        clean.tree_digest,
        String::from_utf8(expected_tree.stdout)
            .expect("Git tree ID is UTF-8")
            .trim()
    );

    write_file(
        repository.path(),
        "src/lib.rs",
        "pub fn value() -> u8 { 2 }\n",
    );
    let error = snapshot_git_repo(repository.path())
        .expect_err("a tracked modification must invalidate the snapshot");
    assert!(matches!(error, RepoError::DirtyWorktree { .. }));
}

#[test]
fn snapshot_rejects_file_and_non_git_directory_roots() {
    let directory = TempDir::new().expect("temporary malformed roots");
    write_file(directory.path(), "plain-file", "not a directory");

    let file_error = snapshot_git_repo(&directory.path().join("plain-file"))
        .expect_err("a file cannot be a repository root");
    assert!(matches!(file_error, RepoError::InvalidRoot { .. }));

    let directory_error = snapshot_git_repo(directory.path())
        .expect_err("a plain directory cannot be snapshotted as Git");
    assert!(matches!(directory_error, RepoError::GitExit { .. }));
}

#[test]
fn history_configuration_accepts_only_the_documented_closed_interval() {
    for accepted in [1, 10_000] {
        let config = RepoProfileConfig {
            history_commits: accepted,
        };
        config.validate().expect("boundary must be accepted");
        GitChangeShapeProgram::new(config).expect("accepted config must construct a program");
    }

    for rejected in [0, 10_001] {
        let config = RepoProfileConfig {
            history_commits: rejected,
        };
        assert!(matches!(
            config.validate(),
            Err(RepoError::InvalidConfig(_))
        ));
        assert!(matches!(
            GitChangeShapeProgram::new(config),
            Err(RepoError::InvalidConfig(_))
        ));
    }
}

#[test]
fn static_program_applies_classifier_precedence_and_reports_exact_byte_metrics() {
    let repository = init_repo();
    let files = [
        ("src/a.rs", "aaaa"),
        ("src/b.rs", "bbbbbbbb"),
        ("tests/README.md", "123456"),
        ("docs/design.rs", "1234567890"),
        ("vendor/check_test.rs", "123456789012"),
        ("settings.json", "{}"),
        ("asset.bin", "x"),
    ];
    for (path, contents) in files {
        write_file(repository.path(), path, contents);
    }
    commit_all(repository.path(), "classified tree", "2001-01-01T00:00:00Z");

    let artifact = snapshot_git_repo(repository.path()).expect("clean committed fixture");
    let program = StaticRepoShapeProgram::new();
    let run = evaluate_pipeline(
        &artifact,
        &[&program],
        &beliefs(),
        &decision(),
        &ResourceBudget {
            max_usd: 0.0,
            max_wall_time_ms: 2_000,
            max_programs: 1,
        },
    )
    .expect("static repository program completes");
    let shape: StaticRepoShape = observation(
        run.steps[0]
            .observation
            .as_ref()
            .expect("completed program has an observation"),
    );

    assert_eq!(shape.tracked_files, 7);
    assert_eq!(shape.tracked_bytes, 43);
    assert_eq!(
        shape.category_files,
        BTreeMap::from([
            ("configuration".to_owned(), 1),
            ("documentation".to_owned(), 1),
            ("generated_or_vendor".to_owned(), 1),
            ("other".to_owned(), 1),
            ("source".to_owned(), 2),
            ("test".to_owned(), 1),
        ])
    );
    assert_eq!(shape.category_bytes["source"], 12);
    assert_eq!(shape.category_bytes["test"], 6);
    assert_eq!(shape.category_bytes["documentation"], 10);
    assert_eq!(shape.category_bytes["configuration"], 2);
    assert_eq!(shape.category_bytes["generated_or_vendor"], 12);
    assert_eq!(shape.category_bytes["other"], 1);
    assert_eq!(shape.source_file_size_median, Some(4));
    assert_eq!(shape.source_file_size_p90, Some(8));
    assert_eq!(shape.largest_source_file_bytes, 8);
    assert_eq!(shape.largest_source_file_path.as_deref(), Some("src/b.rs"));
    assert_abs_diff_eq!(shape.largest_source_file_share.unwrap(), 2.0 / 3.0);
    assert_abs_diff_eq!(shape.top_decile_source_byte_share.unwrap(), 2.0 / 3.0);
    assert_abs_diff_eq!(shape.test_to_source_bytes.unwrap(), 0.5);
    assert_abs_diff_eq!(shape.documentation_to_source_bytes.unwrap(), 5.0 / 6.0);
    assert_abs_diff_eq!(shape.generated_or_vendor_byte_share, 12.0 / 43.0);
    assert_eq!(shape.max_path_depth, 2);
}

#[test]
fn history_program_measures_the_known_two_commit_cochange_topology() {
    let repository = init_repo();
    write_file(repository.path(), "src/lib.rs", "source one\n");
    write_file(repository.path(), "tests/lib.rs", "test one\n");
    commit_all(repository.path(), "source and test", "2001-01-01T00:00:00Z");

    write_file(repository.path(), "src/lib.rs", "source one\nsource two\n");
    write_file(repository.path(), "docs/design.md", "documentation one\n");
    commit_all(
        repository.path(),
        "source and documentation",
        "2001-01-02T00:00:00Z",
    );

    let artifact = snapshot_git_repo(repository.path()).expect("clean two-commit fixture");
    let program = GitChangeShapeProgram::new(RepoProfileConfig { history_commits: 2 })
        .expect("valid bounded history configuration");
    let run = evaluate_pipeline(
        &artifact,
        &[&program],
        &beliefs(),
        &decision(),
        &ResourceBudget {
            max_usd: 0.0,
            max_wall_time_ms: 5_000,
            max_programs: 1,
        },
    )
    .expect("history repository program completes");
    let shape: GitChangeShape = observation(
        run.steps[0]
            .observation
            .as_ref()
            .expect("completed program has an observation"),
    );

    assert_eq!(shape.requested_commits, 2);
    assert_eq!(shape.commits_analyzed, 2);
    assert_eq!(shape.unique_changed_files, 3);
    assert_eq!(shape.total_line_change_mass, 4);
    assert_eq!(shape.files_changed_mean, 2.0);
    assert_eq!(shape.files_changed_median, 2);
    assert_eq!(shape.files_changed_p90, 2);
    assert_eq!(shape.files_changed_max, 2);
    assert_eq!(shape.largest_hotspot_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(shape.largest_hotspot_mass, 2);
    assert_eq!(shape.cross_top_level_pair_ratio, Some(1.0));
    assert_eq!(shape.broad_commit_rate, 0.0);
    assert_eq!(shape.source_commits, 2);
    assert_eq!(shape.source_test_cochange_rate, Some(0.5));
    assert_eq!(shape.source_documentation_cochange_rate, Some(0.5));
}

#[test]
fn empty_initial_commit_supports_snapshot_and_both_repository_programs() {
    let repository = init_repo();
    commit_all(
        repository.path(),
        "empty initial commit",
        "2001-01-01T00:00:00Z",
    );

    let artifact = snapshot_git_repo(repository.path()).expect("empty commit is snapshotable");
    let static_program = StaticRepoShapeProgram::new();
    let history_program = GitChangeShapeProgram::new(RepoProfileConfig { history_commits: 1 })
        .expect("valid single-commit history config");
    let run = evaluate_pipeline(
        &artifact,
        &[
            &static_program as &dyn CriterionProgram,
            &history_program as &dyn CriterionProgram,
        ],
        &beliefs(),
        &decision(),
        &ResourceBudget {
            max_usd: 0.0,
            max_wall_time_ms: 7_000,
            max_programs: 2,
        },
    )
    .expect("empty committed repository supports both programs");

    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.stopped_reason, StopReason::Complete);
    assert!(
        run.steps
            .iter()
            .all(|step| step.receipt.status == ProgramStatus::Completed)
    );

    let static_shape: StaticRepoShape = observation(
        run.steps[0]
            .observation
            .as_ref()
            .expect("static program observes an empty tree"),
    );
    assert_eq!(static_shape.tracked_files, 0);
    assert_eq!(static_shape.tracked_bytes, 0);
    assert!(
        static_shape
            .category_files
            .values()
            .all(|count| *count == 0)
    );
    assert!(
        static_shape
            .category_bytes
            .values()
            .all(|bytes| *bytes == 0)
    );
    assert_eq!(static_shape.source_file_size_median, None);
    assert_eq!(static_shape.source_file_size_p90, None);
    assert_eq!(static_shape.largest_source_file_bytes, 0);
    assert_eq!(static_shape.largest_source_file_path, None);
    assert_eq!(static_shape.largest_source_file_share, None);
    assert_eq!(static_shape.top_decile_source_byte_share, None);
    assert_eq!(static_shape.normalized_source_size_entropy, None);
    assert_eq!(static_shape.effective_source_files, None);
    assert_eq!(static_shape.normalized_top_level_source_entropy, None);
    assert_eq!(static_shape.effective_top_level_components, None);
    assert_eq!(static_shape.test_to_source_bytes, None);
    assert_eq!(static_shape.documentation_to_source_bytes, None);

    let history_shape: GitChangeShape = observation(
        run.steps[1]
            .observation
            .as_ref()
            .expect("history program observes the empty commit"),
    );
    assert_eq!(history_shape.commits_analyzed, 1);
    assert_eq!(history_shape.unique_changed_files, 0);
    assert_eq!(history_shape.total_line_change_mass, 0);
    assert_eq!(history_shape.files_changed_mean, 0.0);
    assert_eq!(history_shape.files_changed_max, 0);
    assert_eq!(history_shape.source_commits, 0);
    assert_eq!(history_shape.source_test_cochange_rate, None);
    assert_eq!(history_shape.source_documentation_cochange_rate, None);
}

#[test]
fn two_repository_programs_complete_with_zero_dollars_and_auditable_receipts() {
    let repository = init_repo();
    write_file(
        repository.path(),
        "src/lib.rs",
        "pub fn answer() -> u8 { 42 }\n",
    );
    write_file(repository.path(), "tests/lib.rs", "assert answer\n");
    commit_all(repository.path(), "fixture", "2001-01-01T00:00:00Z");

    let artifact = snapshot_git_repo(repository.path()).expect("clean committed fixture");
    let static_program = StaticRepoShapeProgram::new();
    let history_program = GitChangeShapeProgram::new(RepoProfileConfig { history_commits: 2 })
        .expect("valid history config");
    let run = evaluate_pipeline(
        &artifact,
        &[
            &static_program as &dyn CriterionProgram,
            &history_program as &dyn CriterionProgram,
        ],
        &beliefs(),
        &decision(),
        &ResourceBudget {
            max_usd: 0.0,
            max_wall_time_ms: 7_000,
            max_programs: 2,
        },
    )
    .expect("both deterministic programs fit a zero-dollar budget");

    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.stopped_reason, StopReason::Complete);
    assert_eq!(run.remaining.usd, 0.0);
    assert_eq!(run.remaining.programs, 0);
    assert_eq!(run.steps[0].receipt.status, ProgramStatus::Completed);
    assert_eq!(run.steps[1].receipt.status, ProgramStatus::Completed);
    assert_eq!(run.steps[0].receipt.program.version, "1");
    assert_eq!(run.steps[1].receipt.program.id, "repo.git-change-shape");
    assert_eq!(run.steps[1].receipt.program.version, "2");
    assert_ne!(
        run.steps[0].receipt.program.id,
        run.steps[1].receipt.program.id
    );

    let static_stdout = git(
        repository.path(),
        ["ls-tree", "-r", "-z", "--long", &artifact.revision],
    )
    .stdout;
    // v2 receipts the full timestamp-aware N+1 probe, not the truncated parsed window:
    // requested N=2 therefore independently reconstructs `git log -n 3` byte-for-byte.
    let history_stdout = git(
        repository.path(),
        [
            "log",
            "--no-merges",
            "--no-renames",
            "--format=%x1e%H%x00%ct%x00",
            "--numstat",
            "-n",
            "3",
            &artifact.revision,
            "--",
        ],
    )
    .stdout;

    for (step, stdout) in run.steps.iter().zip([static_stdout, history_stdout]) {
        assert!(!stdout.is_empty(), "fixture Git evidence must be nonempty");
        assert_eq!(step.receipt.actual_resources.usd, 0.0);
        assert_eq!(step.receipt.actual_resources.programs, 1);
        assert_eq!(
            step.receipt.actual_resources.bytes_read,
            u64::try_from(stdout.len()).expect("fixture output length fits u64")
        );
        assert!(step.receipt.actual_resources.bytes_read > 0);
        assert_eq!(step.evidence.len(), 1);
        assert_eq!(
            step.evidence[0].digest.as_deref(),
            Some(sha256_hex(&stdout).as_str())
        );
        let observation_digest = step
            .receipt
            .observation_digest
            .as_deref()
            .expect("completed receipt records an observation digest");
        assert_eq!(observation_digest.len(), 64);
        assert!(
            observation_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }
}

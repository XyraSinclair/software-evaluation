use std::fs;
use std::path::Path;

use software_evaluation::audit::{AuditIssue, AuditReport, Severity, audit_evaluation_dir};
use tempfile::TempDir;

const PROCEDURE_PATH: &str = "procedure.md";
const EVIDENCE_PATH: &str = "evidence.txt";

fn write_file(root: &Path, relative_path: &str, contents: impl AsRef<[u8]>) {
    fs::write(root.join(relative_path), contents)
        .unwrap_or_else(|error| panic!("failed to write test fixture {relative_path}: {error}"));
}

fn record(id: &str, instrument: &str, procedure: &str, evidence: &str) -> String {
    serde_json::json!({
        "id": id,
        "artifact": "demo@0123456789abcdef",
        "axis": "correctness",
        "instrument": instrument,
        "agent": {"kind": "tool", "id": "audit-test"},
        "procedure": procedure,
        "evidence": evidence,
        "verdict": "pass",
        "integrity": "clean",
        "ts": "2026-07-10T12:34:56Z",
    })
    .to_string()
}
fn customized_record(id: &str, fields: &[(&str, Option<serde_json::Value>)]) -> String {
    let mut value: serde_json::Value =
        serde_json::from_str(&record(id, "mechanical", PROCEDURE_PATH, EVIDENCE_PATH))
            .expect("shared record fixture must be valid JSON");
    let object = value
        .as_object_mut()
        .expect("shared record fixture must be a JSON object");

    for (field, replacement) in fields {
        match replacement {
            Some(replacement) => {
                object.insert((*field).to_owned(), replacement.clone());
            }
            None => {
                object.remove(*field);
            }
        }
    }

    value.to_string()
}

fn write_bundle(root: &Path, records: &str, report: &str) {
    write_file(root, PROCEDURE_PATH, "# Procedure\n\nRun the check.\n");
    write_file(root, EVIDENCE_PATH, "observed output\n");
    write_file(root, "records.jsonl", format!("{records}\n"));
    write_file(root, "report.md", report);
}

fn audit(root: &Path) -> AuditReport {
    audit_evaluation_dir(root).expect("fixture directory should be auditable")
}

fn issues_with_code<'a>(report: &'a AuditReport, code: &str) -> Vec<&'a AuditIssue> {
    report
        .issues
        .iter()
        .filter(|issue| issue.code == code)
        .collect()
}

#[test]
fn minimal_valid_bundle_passes_and_broken_evidence_turns_it_red() {
    let directory = TempDir::new().expect("temporary directory");
    write_bundle(
        directory.path(),
        &record("r-1", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nThe result is supported by r-1.\n",
    );

    let valid = audit(directory.path());
    assert!(valid.passed(), "valid bundle issues: {:?}", valid.issues);
    assert!(valid.issues.is_empty());
    assert_eq!(valid.records_total, 1);
    assert_eq!(valid.instrument_counts.get("mechanical"), Some(&1));
    assert_eq!(valid.referenced_record_ids, ["r-1"]);

    fs::remove_file(directory.path().join(EVIDENCE_PATH))
        .expect("positive control removes the referenced evidence");

    let broken = audit(directory.path());
    assert!(!broken.passed());
    let missing = issues_with_code(&broken, "missing_evidence");
    assert_eq!(
        missing.len(),
        1,
        "broken bundle issues: {:?}",
        broken.issues
    );
    assert_eq!(missing[0].severity, Severity::Error);
}

#[test]
fn missing_or_empty_required_files_fail_closed() {
    #[derive(Clone, Copy)]
    enum State {
        Missing,
        Empty,
    }

    let cases = [
        (
            "missing report",
            "report.md",
            State::Missing,
            "missing_required_file",
        ),
        (
            "missing records",
            "records.jsonl",
            State::Missing,
            "missing_required_file",
        ),
        (
            "empty report",
            "report.md",
            State::Empty,
            "empty_required_file",
        ),
        (
            "empty records",
            "records.jsonl",
            State::Empty,
            "empty_required_file",
        ),
    ];

    for (name, required_file, state, expected_code) in cases {
        let directory = TempDir::new().expect("temporary directory");
        write_bundle(
            directory.path(),
            &record("r-1", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH),
            "# Report\n\nr-1\n",
        );

        let required_path = directory.path().join(required_file);
        match state {
            State::Missing => fs::remove_file(&required_path).expect("remove required fixture"),
            State::Empty => fs::write(&required_path, []).expect("empty required fixture"),
        }

        let report = audit(directory.path());
        assert!(!report.passed(), "{name} unexpectedly passed");
        let matching = issues_with_code(&report, expected_code);
        assert_eq!(matching.len(), 1, "{name} issues: {:?}", report.issues);
        assert_eq!(matching[0].severity, Severity::Error);
        assert!(
            matching[0]
                .path
                .as_deref()
                .is_some_and(|path| path.ends_with(required_file)),
            "{name} issue did not identify {required_file}: {:?}",
            matching[0]
        );
    }
}

#[test]
fn malformed_jsonl_does_not_stop_later_record_auditing() {
    let directory = TempDir::new().expect("temporary directory");
    let later_record = record(
        "late-1",
        "mechanical",
        PROCEDURE_PATH,
        "missing-later-evidence.txt",
    );
    write_bundle(
        directory.path(),
        &format!("{{\n{later_record}"),
        "# Report\n\nNo record citation is needed here.\n",
    );

    let report = audit(directory.path());
    assert_eq!(report.records_total, 1);

    let malformed = issues_with_code(&report, "invalid_json_record");
    assert_eq!(malformed.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(malformed[0].line, Some(1));

    let later_evidence = issues_with_code(&report, "missing_evidence");
    assert_eq!(later_evidence.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(later_evidence[0].line, Some(2));
    assert!(!report.passed());
}

#[test]
fn missing_and_parent_traversal_evidence_have_distinct_errors() {
    let directory = TempDir::new().expect("temporary directory");
    let records = [
        record("r-missing", "mechanical", PROCEDURE_PATH, "missing.txt"),
        record("r-unsafe", "mechanical", PROCEDURE_PATH, "../outside.txt"),
    ]
    .join("\n");
    write_bundle(
        directory.path(),
        &records,
        "# Report\n\nr-missing and r-unsafe support this result.\n",
    );

    let report = audit(directory.path());
    assert_eq!(report.records_total, 2);

    let missing = issues_with_code(&report, "missing_evidence");
    assert_eq!(missing.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(missing[0].line, Some(1));

    let unsafe_reference = issues_with_code(&report, "unsafe_reference");
    assert_eq!(unsafe_reference.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(unsafe_reference[0].line, Some(2));
    assert!(unsafe_reference[0].message.contains("evidence"));
    assert!(!report.passed());
}

#[test]
fn duplicate_record_ids_fail_the_audit() {
    let directory = TempDir::new().expect("temporary directory");
    let duplicate = record("r-dup", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH);
    write_bundle(
        directory.path(),
        &format!("{duplicate}\n{duplicate}"),
        "# Report\n\nr-dup\n",
    );

    let report = audit(directory.path());
    assert_eq!(report.records_total, 2);
    let duplicates = issues_with_code(&report, "duplicate_record_id");
    assert_eq!(duplicates.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(duplicates[0].line, Some(2));
    assert_eq!(duplicates[0].severity, Severity::Error);
    assert!(!report.passed());
}

#[test]
fn report_alias_with_slash_is_a_single_unknown_record_reference() {
    let directory = TempDir::new().expect("temporary directory");
    write_bundle(
        directory.path(),
        &record("r-c", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nThe claim cites r-c/x-axis.\n",
    );

    let report = audit(directory.path());
    assert_eq!(report.referenced_record_ids, ["r-c/x-axis"]);
    let unknown = issues_with_code(&report, "unknown_report_record");
    assert_eq!(unknown.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(unknown[0].line, Some(3));
    assert!(unknown[0].message.contains("r-c/x-axis"));
    assert!(!report.passed());
}

#[test]
fn hyphenated_prose_does_not_create_false_record_references() {
    let directory = TempDir::new().expect("temporary directory");
    write_bundle(
        directory.path(),
        &record("r-clean", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nRecord r-clean supports per-area, near-total, and user-elicited analysis.\n",
    );

    let report = audit(directory.path());
    assert_eq!(report.referenced_record_ids, ["r-clean"]);
    assert!(
        issues_with_code(&report, "unknown_report_record").is_empty(),
        "ordinary prose was parsed as a record reference: {:?}",
        report.issues
    );
    assert!(report.passed(), "issues: {:?}", report.issues);
}

#[test]
fn judged_procedure_without_exact_prompt_marker_warns_but_passes() {
    let directory = TempDir::new().expect("temporary directory");
    write_bundle(
        directory.path(),
        &record("r-judged", "judged", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nr-judged supports the result.\n",
    );
    write_file(
        directory.path(),
        PROCEDURE_PATH,
        "# Method\n\nAsk the reviewer to inspect the artifact.\n",
    );

    let report = audit(directory.path());
    let warnings = issues_with_code(&report, "exact_prompt_unproven");
    assert_eq!(warnings.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(warnings[0].severity, Severity::Warning);
    assert_eq!(warnings[0].line, Some(1));
    assert!(
        report.passed(),
        "a warning alone should not fail: {:?}",
        report.issues
    );
}

#[test]
fn zero_valid_object_records_never_passes() {
    let directory = TempDir::new().expect("temporary directory");
    write_bundle(
        directory.path(),
        "[]\n{not valid json}",
        "# Report\n\nNo citations.\n",
    );

    let report = audit(directory.path());
    assert_eq!(report.records_total, 0);
    let no_records = issues_with_code(&report, "no_valid_records");
    assert_eq!(no_records.len(), 1, "issues: {:?}", report.issues);
    assert_eq!(no_records[0].severity, Severity::Error);
    assert_eq!(issues_with_code(&report, "invalid_json_record").len(), 2);
    assert!(!report.passed());
}

#[test]
fn single_artifact_identity_requires_at_least_seven_hex_commit_digits() {
    let cases = [
        ("seven digit hexadecimal pin", "repo@0123abc", true),
        ("symbolic HEAD pin", "repo@HEAD", false),
        ("pending pin", "repo@pending", false),
        ("short hexadecimal pin", "repo@0123ab", false),
    ];

    for (name, artifact, should_pass) in cases {
        let directory = TempDir::new().expect("temporary directory");
        let record = customized_record(
            "r-artifact",
            &[("artifact", Some(serde_json::json!(artifact)))],
        );
        write_bundle(directory.path(), &record, "# Report\n\nr-artifact\n");

        let report = audit(directory.path());
        assert_eq!(
            report.passed(),
            should_pass,
            "{name} ({artifact:?}) produced issues: {:?}",
            report.issues
        );
    }
}

#[test]
fn two_artifact_comparison_accepts_two_hex_commit_pins() {
    let directory = TempDir::new().expect("temporary directory");
    let artifact = "alpha@0123abc vs beta@89def01";
    let record = customized_record(
        "r-comparison",
        &[("artifact", Some(serde_json::json!(artifact)))],
    );
    write_bundle(directory.path(), &record, "# Report\n\nr-comparison\n");

    let report = audit(directory.path());
    assert!(
        report.passed(),
        "valid comparison identity {artifact:?} produced issues: {:?}",
        report.issues
    );
}

#[test]
fn two_artifact_comparison_rejects_malformed_or_mixed_pins() {
    let cases = [
        ("one symbolic pin", "alpha@0123abc vs beta@HEAD"),
        ("one unpinned artifact", "alpha@0123abc vs beta"),
        (
            "extra artifact",
            "alpha@0123abc vs beta@89def01 vs gamma@7654321",
        ),
    ];

    for (name, artifact) in cases {
        let directory = TempDir::new().expect("temporary directory");
        let record = customized_record(
            "r-comparison",
            &[("artifact", Some(serde_json::json!(artifact)))],
        );
        write_bundle(directory.path(), &record, "# Report\n\nr-comparison\n");

        let report = audit(directory.path());
        assert!(
            !report.passed(),
            "comparison with {name} ({artifact:?}) unexpectedly passed"
        );
    }
}

#[test]
fn instrument_accepts_only_the_three_provenance_classes() {
    let cases = [
        ("mechanical", true),
        ("empirical", true),
        ("judged", true),
        ("measurement", false),
        ("manual", false),
        ("Mechanical", false),
    ];

    for (instrument, should_pass) in cases {
        let directory = TempDir::new().expect("temporary directory");
        write_bundle(
            directory.path(),
            &record("r-instrument", instrument, PROCEDURE_PATH, EVIDENCE_PATH),
            "# Report\n\nr-instrument\n",
        );

        let report = audit(directory.path());
        assert_eq!(
            report.passed(),
            should_pass,
            "instrument {instrument:?} produced issues: {:?}",
            report.issues
        );
    }
}

#[test]
fn agent_provenance_requires_nonempty_string_kind_and_id() {
    let cases = [
        (
            "valid provenance",
            Some(serde_json::json!({"kind": "tool", "id": "seval/0.1.0"})),
            true,
        ),
        ("missing agent", None, false),
        ("null agent", Some(serde_json::Value::Null), false),
        ("empty object", Some(serde_json::json!({})), false),
        (
            "nonempty object without kind or id",
            Some(serde_json::json!({"runner": "local"})),
            false,
        ),
        (
            "missing id",
            Some(serde_json::json!({"kind": "tool"})),
            false,
        ),
        (
            "missing kind",
            Some(serde_json::json!({"id": "seval/0.1.0"})),
            false,
        ),
        (
            "non-string kind",
            Some(serde_json::json!({"kind": 1, "id": "seval/0.1.0"})),
            false,
        ),
        (
            "non-string id",
            Some(serde_json::json!({"kind": "tool", "id": 1})),
            false,
        ),
        (
            "empty kind",
            Some(serde_json::json!({"kind": "", "id": "seval/0.1.0"})),
            false,
        ),
        (
            "empty id",
            Some(serde_json::json!({"kind": "tool", "id": ""})),
            false,
        ),
    ];

    for (name, agent, should_pass) in cases {
        let directory = TempDir::new().expect("temporary directory");
        let record = customized_record("r-agent", &[("agent", agent)]);
        write_bundle(directory.path(), &record, "# Report\n\nr-agent\n");

        let report = audit(directory.path());
        assert_eq!(
            report.passed(),
            should_pass,
            "{name} produced issues: {:?}",
            report.issues
        );
    }
}

#[test]
fn timestamp_is_required_and_accepts_only_null_or_rfc3339_like_strings() {
    let cases = [
        (
            "UTC timestamp",
            Some(serde_json::json!("2026-07-10T12:34:56Z")),
            true,
        ),
        (
            "offset timestamp",
            Some(serde_json::json!("2026-07-10T14:34:56+02:00")),
            true,
        ),
        (
            "honest unknown timestamp",
            Some(serde_json::Value::Null),
            true,
        ),
        ("missing timestamp", None, false),
        ("empty timestamp", Some(serde_json::json!("")), false),
        (
            "non-RFC3339 timestamp",
            Some(serde_json::json!("yesterday")),
            false,
        ),
        (
            "numeric timestamp",
            Some(serde_json::json!(1_720_614_896)),
            false,
        ),
    ];

    for (name, timestamp, should_pass) in cases {
        let directory = TempDir::new().expect("temporary directory");
        let record = customized_record("r-ts", &[("ts", timestamp)]);
        write_bundle(directory.path(), &record, "# Report\n\nr-ts\n");

        let report = audit(directory.path());
        assert_eq!(
            report.passed(),
            should_pass,
            "{name} produced issues: {:?}",
            report.issues
        );
    }
}

#[cfg(unix)]
fn assert_escaping_symlink_is_rejected(escaped_reference: &str) {
    use std::os::unix::fs::symlink;

    let ordinary = TempDir::new().expect("ordinary evaluation directory");
    write_bundle(
        ordinary.path(),
        &record("r-ordinary", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nr-ordinary\n",
    );
    let ordinary_report = audit(ordinary.path());
    assert!(
        ordinary_report.passed(),
        "ordinary in-root references produced issues: {:?}",
        ordinary_report.issues
    );

    let outside = TempDir::new().expect("outside directory");
    write_file(
        outside.path(),
        "outside.txt",
        "content outside the bundle\n",
    );

    let directory = TempDir::new().expect("evaluation directory");
    write_bundle(
        directory.path(),
        &record("r-escape", "mechanical", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nr-escape\n",
    );
    let reference_path = directory.path().join(escaped_reference);
    fs::remove_file(&reference_path).expect("remove ordinary in-root reference");
    symlink(outside.path().join("outside.txt"), &reference_path).expect("create escaping symlink");

    let report = audit(directory.path());
    assert!(
        !report.passed(),
        "escaping {escaped_reference} symlink unexpectedly passed: {:?}",
        report.issues
    );
}

#[cfg(unix)]
#[test]
fn procedure_symlink_cannot_escape_the_evaluation_root() {
    assert_escaping_symlink_is_rejected(PROCEDURE_PATH);
}

#[cfg(unix)]
#[test]
fn evidence_symlink_cannot_escape_the_evaluation_root() {
    assert_escaping_symlink_is_rejected(EVIDENCE_PATH);
}

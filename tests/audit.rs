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
        "procedure": procedure,
        "evidence": evidence,
        "verdict": "pass",
        "integrity": "clean"
    })
    .to_string()
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
        &record("r-1", "measurement", PROCEDURE_PATH, EVIDENCE_PATH),
        "# Report\n\nThe result is supported by r-1.\n",
    );

    let valid = audit(directory.path());
    assert!(valid.passed(), "valid bundle issues: {:?}", valid.issues);
    assert!(valid.issues.is_empty());
    assert_eq!(valid.records_total, 1);
    assert_eq!(valid.instrument_counts.get("measurement"), Some(&1));
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
            &record("r-1", "measurement", PROCEDURE_PATH, EVIDENCE_PATH),
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
        "measurement",
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
        record("r-missing", "measurement", PROCEDURE_PATH, "missing.txt"),
        record("r-unsafe", "measurement", PROCEDURE_PATH, "../outside.txt"),
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
    let duplicate = record("r-dup", "measurement", PROCEDURE_PATH, EVIDENCE_PATH);
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
        &record("r-c", "measurement", PROCEDURE_PATH, EVIDENCE_PATH),
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
        &record("r-clean", "measurement", PROCEDURE_PATH, EVIDENCE_PATH),
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

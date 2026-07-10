//! Read-only integrity checks for archived evaluation bundles.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

const REQUIRED_FIELDS: [&str; 8] = [
    "id",
    "artifact",
    "axis",
    "instrument",
    "procedure",
    "evidence",
    "verdict",
    "integrity",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuditIssue {
    pub code: String,
    pub severity: Severity,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuditReport {
    pub evaluation_dir: String,
    pub records_total: usize,
    pub instrument_counts: BTreeMap<String, usize>,
    pub referenced_record_ids: Vec<String>,
    pub issues: Vec<AuditIssue>,
}

impl AuditReport {
    pub fn passed(&self) -> bool {
        self.records_total > 0
            && !self
                .issues
                .iter()
                .any(|issue| issue.severity == Severity::Error)
    }
}

#[derive(Error, Debug)]
pub enum AuditError {
    #[error("invalid evaluation root {path}: {reason}")]
    InvalidRoot { path: PathBuf, reason: String },

    #[error("failed to read audit input {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    // Malformed JSONL rows are recoverable AuditIssues. This variant is kept
    // for JSON failures that prevent the audit itself from proceeding.
    #[error("failed to parse JSON audit input {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to initialize record-reference matching: {0}")]
    ReferencePattern(#[from] regex::Error),
}

#[derive(Clone, Debug)]
enum ReferenceProbe {
    Missing,
    NotRegular,
    Empty,
    Present { has_exact_prompt: bool },
}

/// Audit an `evaluations/<name>` directory without modifying it.
pub fn audit_evaluation_dir(path: &Path) -> Result<AuditReport, AuditError> {
    validate_root(path)?;

    let evaluation_dir = path.display().to_string();
    let report_path = path.join("report.md");
    let records_path = path.join("records.jsonl");
    let mut issues = Vec::new();

    let report_bytes = read_required_file(&report_path, &mut issues)?;
    let records_bytes = read_required_file(&records_path, &mut issues)?;

    let mut records_total = 0;
    let mut instrument_counts = BTreeMap::new();
    let mut record_ids = HashSet::new();
    let mut reference_cache = HashMap::new();

    if let Some(records_bytes) = records_bytes {
        audit_records(
            path,
            &records_path,
            &records_bytes,
            &mut records_total,
            &mut instrument_counts,
            &mut record_ids,
            &mut reference_cache,
            &mut issues,
        )?;
    }

    if records_total == 0 {
        issues.push(AuditIssue {
            code: "no_valid_records".to_owned(),
            severity: Severity::Error,
            path: Some(records_path.display().to_string()),
            line: None,
            message: "records.jsonl contains no valid JSON object records".to_owned(),
        });
    }

    let report_references = match report_bytes {
        Some(bytes) => {
            if std::str::from_utf8(&bytes).is_err() {
                issues.push(AuditIssue {
                    code: "invalid_report_encoding".to_owned(),
                    severity: Severity::Error,
                    path: Some(report_path.display().to_string()),
                    line: None,
                    message: format!(
                        "report {} is not valid UTF-8 and cannot be audited reliably",
                        report_path.display()
                    ),
                });
            }
            extract_report_references(&bytes)?
        }
        None => BTreeMap::new(),
    };
    let referenced_record_ids = report_references.keys().cloned().collect::<Vec<_>>();

    // BTreeMap iteration keeps these issues sorted by reference, after all
    // record-source issues have been emitted in line order.
    for (reference, line) in report_references {
        if !record_ids.contains(&reference) {
            issues.push(AuditIssue {
                code: "unknown_report_record".to_owned(),
                severity: Severity::Error,
                path: Some(report_path.display().to_string()),
                line: Some(line),
                message: format!(
                    "report references record {reference:?}, but records.jsonl has no parsed record with that id"
                ),
            });
        }
    }

    Ok(AuditReport {
        evaluation_dir,
        records_total,
        instrument_counts,
        referenced_record_ids,
        issues,
    })
}

fn validate_root(path: &Path) -> Result<(), AuditError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(AuditError::InvalidRoot {
            path: path.to_path_buf(),
            reason: "path is not a directory".to_owned(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Err(AuditError::InvalidRoot {
            path: path.to_path_buf(),
            reason: "path does not exist".to_owned(),
        }),
        Err(source) => Err(AuditError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_required_file(
    path: &Path,
    issues: &mut Vec<AuditIssue>,
) -> Result<Option<Vec<u8>>, AuditError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            issues.push(AuditIssue {
                code: "missing_required_file".to_owned(),
                severity: Severity::Error,
                path: Some(path.display().to_string()),
                line: None,
                message: format!("required file {} does not exist", path.display()),
            });
            return Ok(None);
        }
        Err(source) => {
            return Err(AuditError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    if !metadata.is_file() {
        issues.push(AuditIssue {
            code: "invalid_required_file".to_owned(),
            severity: Severity::Error,
            path: Some(path.display().to_string()),
            line: None,
            message: format!("required path {} is not a regular file", path.display()),
        });
        return Ok(None);
    }

    let bytes = fs::read(path).map_err(|source| AuditError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.is_empty() {
        issues.push(AuditIssue {
            code: "empty_required_file".to_owned(),
            severity: Severity::Error,
            path: Some(path.display().to_string()),
            line: None,
            message: format!("required file {} is empty", path.display()),
        });
    }

    Ok(Some(bytes))
}

#[allow(clippy::too_many_arguments)]
fn audit_records(
    evaluation_dir: &Path,
    records_path: &Path,
    records_bytes: &[u8],
    records_total: &mut usize,
    instrument_counts: &mut BTreeMap<String, usize>,
    record_ids: &mut HashSet<String>,
    reference_cache: &mut HashMap<PathBuf, ReferenceProbe>,
    issues: &mut Vec<AuditIssue>,
) -> Result<(), AuditError> {
    for (line_index, raw_line) in records_bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = line_index + 1;
        if raw_line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }

        let value = match serde_json::from_slice::<Value>(raw_line) {
            Ok(value) => value,
            Err(source) => {
                issues.push(record_issue(
                    "invalid_json_record",
                    Severity::Error,
                    records_path,
                    line_number,
                    format!("record is not valid JSON: {source}"),
                ));
                continue;
            }
        };

        let object = match value.as_object() {
            Some(object) => object,
            None => {
                issues.push(record_issue(
                    "invalid_json_record",
                    Severity::Error,
                    records_path,
                    line_number,
                    "record must be a JSON object".to_owned(),
                ));
                continue;
            }
        };

        *records_total += 1;
        audit_record_object(
            evaluation_dir,
            records_path,
            line_number,
            object,
            instrument_counts,
            record_ids,
            reference_cache,
            issues,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn audit_record_object(
    evaluation_dir: &Path,
    records_path: &Path,
    line_number: usize,
    object: &Map<String, Value>,
    instrument_counts: &mut BTreeMap<String, usize>,
    record_ids: &mut HashSet<String>,
    reference_cache: &mut HashMap<PathBuf, ReferenceProbe>,
    issues: &mut Vec<AuditIssue>,
) -> Result<(), AuditError> {
    let mut fields = HashMap::new();
    for field in REQUIRED_FIELDS {
        match nonempty_string(object, field) {
            Some(value) => {
                fields.insert(field, value);
            }
            None => {
                let code = if field == "id" {
                    "missing_record_id"
                } else {
                    "invalid_record_field"
                };
                issues.push(record_issue(
                    code,
                    Severity::Error,
                    records_path,
                    line_number,
                    format!("record field {field:?} must be a nonempty string"),
                ));
            }
        }
    }

    let record_id = fields.get("id").copied();
    if let Some(record_id) = record_id
        && !record_ids.insert(record_id.to_owned())
    {
        issues.push(record_issue(
            "duplicate_record_id",
            Severity::Error,
            records_path,
            line_number,
            format!("record id {record_id:?} is duplicated"),
        ));
    }

    if let Some(instrument) = fields.get("instrument") {
        *instrument_counts
            .entry((*instrument).to_owned())
            .or_insert(0) += 1;
    }

    if let Some(artifact) = fields.get("artifact") {
        if !artifact.contains('@') {
            issues.push(record_issue(
                "invalid_artifact_identity",
                Severity::Error,
                records_path,
                line_number,
                format!("artifact {artifact:?} is not commit-pinned; use one name@commit identity"),
            ));
        }
        if artifact.contains(" vs ") {
            issues.push(record_issue(
                "ambiguous_comparison_identity",
                Severity::Error,
                records_path,
                line_number,
                format!(
                    "artifact {artifact:?} contains a comparison; store one commit-pinned artifact per field"
                ),
            ));
        }
    }

    let procedure_probe = match fields.get("procedure") {
        Some(reference) => audit_reference(
            evaluation_dir,
            records_path,
            line_number,
            record_id,
            "procedure",
            reference,
            reference_cache,
            issues,
        )?,
        None => None,
    };

    if fields.get("instrument").copied() == Some("judged")
        && let Some(ReferenceProbe::Present {
            has_exact_prompt: false,
        }) = procedure_probe
    {
        let reference = fields
            .get("procedure")
            .copied()
            .unwrap_or("<invalid procedure>");
        issues.push(record_issue(
            "exact_prompt_unproven",
            Severity::Warning,
            records_path,
            line_number,
            format!(
                "judged record {} points to procedure {reference:?}, but an exact prompt marker could not be proven in that file",
                display_record_id(record_id)
            ),
        ));
    }

    if let Some(reference) = fields.get("evidence") {
        audit_reference(
            evaluation_dir,
            records_path,
            line_number,
            record_id,
            "evidence",
            reference,
            reference_cache,
            issues,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn audit_reference(
    evaluation_dir: &Path,
    records_path: &Path,
    line_number: usize,
    record_id: Option<&str>,
    kind: &str,
    reference: &str,
    reference_cache: &mut HashMap<PathBuf, ReferenceProbe>,
    issues: &mut Vec<AuditIssue>,
) -> Result<Option<ReferenceProbe>, AuditError> {
    let resolution_reference = strip_markdown_fragment(reference);
    let relative_path = Path::new(resolution_reference);

    if resolution_reference.is_empty()
        || relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        issues.push(record_issue(
            "unsafe_reference",
            Severity::Error,
            records_path,
            line_number,
            format!(
                "record {} has unsafe {kind} path {reference:?}; use a relative path without parent traversal",
                display_record_id(record_id)
            ),
        ));
        return Ok(None);
    }

    let resolved_path = evaluation_dir.join(relative_path);
    let probe = probe_reference(&resolved_path, reference_cache)?;
    match &probe {
        ReferenceProbe::Missing => issues.push(record_issue(
            &format!("missing_{kind}"),
            Severity::Error,
            records_path,
            line_number,
            format!(
                "record {} references missing {kind} {reference:?} (resolved to {})",
                display_record_id(record_id),
                resolved_path.display()
            ),
        )),
        ReferenceProbe::NotRegular => issues.push(record_issue(
            &format!("missing_{kind}"),
            Severity::Error,
            records_path,
            line_number,
            format!(
                "record {} references {kind} {reference:?}, but {} is not a regular file",
                display_record_id(record_id),
                resolved_path.display()
            ),
        )),
        ReferenceProbe::Empty => issues.push(record_issue(
            &format!("empty_{kind}"),
            Severity::Error,
            records_path,
            line_number,
            format!(
                "record {} references empty {kind} {reference:?} at {}",
                display_record_id(record_id),
                resolved_path.display()
            ),
        )),
        ReferenceProbe::Present { .. } => {}
    }

    Ok(Some(probe))
}

fn probe_reference(
    path: &Path,
    cache: &mut HashMap<PathBuf, ReferenceProbe>,
) -> Result<ReferenceProbe, AuditError> {
    if let Some(probe) = cache.get(path) {
        return Ok(probe.clone());
    }

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            let probe = ReferenceProbe::Missing;
            cache.insert(path.to_path_buf(), probe.clone());
            return Ok(probe);
        }
        Err(source) => {
            return Err(AuditError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    if !metadata.is_file() {
        let probe = ReferenceProbe::NotRegular;
        cache.insert(path.to_path_buf(), probe.clone());
        return Ok(probe);
    }

    let bytes = fs::read(path).map_err(|source| AuditError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let probe = if bytes.is_empty() {
        ReferenceProbe::Empty
    } else {
        ReferenceProbe::Present {
            has_exact_prompt: contains_exact_prompt_marker(&bytes),
        }
    };
    cache.insert(path.to_path_buf(), probe.clone());
    Ok(probe)
}

fn strip_markdown_fragment(reference: &str) -> &str {
    reference
        .split_once('#')
        .map_or(reference, |(path, _fragment)| path)
}

fn contains_exact_prompt_marker(contents: &[u8]) -> bool {
    String::from_utf8_lossy(contents).lines().any(|line| {
        let marker = line.trim();
        marker.eq_ignore_ascii_case("# Prompt")
            || marker.eq_ignore_ascii_case("## Prompt")
            || marker.eq_ignore_ascii_case("BEGIN PROMPT")
    })
}

fn extract_report_references(contents: &[u8]) -> Result<BTreeMap<String, usize>, AuditError> {
    let reference_pattern = Regex::new(r"\br-[A-Za-z0-9][A-Za-z0-9_./-]*")?;
    let report = String::from_utf8_lossy(contents);
    let mut references = BTreeMap::new();

    for (line_index, line) in report.lines().enumerate() {
        for matched in reference_pattern.find_iter(line) {
            let reference = matched.as_str().trim_end_matches(|character| {
                matches!(character, '.' | ',' | ';' | ':' | '!' | '?')
            });
            if !reference.is_empty() {
                references
                    .entry(reference.to_owned())
                    .or_insert(line_index + 1);
            }
        }
    }

    Ok(references)
}

fn nonempty_string<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn record_issue(
    code: &str,
    severity: Severity,
    records_path: &Path,
    line: usize,
    message: String,
) -> AuditIssue {
    AuditIssue {
        code: code.to_owned(),
        severity,
        path: Some(records_path.display().to_string()),
        line: Some(line),
        message,
    }
}

fn display_record_id(record_id: Option<&str>) -> String {
    record_id
        .map(|record_id| format!("{record_id:?}"))
        .unwrap_or_else(|| "with no valid id".to_owned())
}

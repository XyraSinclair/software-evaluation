use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::kernel::ArtifactSnapshot;
use crate::metrics::{MetricsError, analyze_committed_files, supports_path};
use crate::repo::{
    RepoError, read_committed_blobs, scan_committed_regular_files, scan_file_history,
    snapshot_git_repo,
};

const ANALYZER: &str = "seval-change-profile-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeProfileConfig {
    pub history_commits: usize,
}

impl Default for ChangeProfileConfig {
    fn default() -> Self {
        Self {
            history_commits: 200,
        }
    }
}

impl ChangeProfileConfig {
    fn validate(&self) -> Result<(), ChangeProfileError> {
        if (1..=10_000).contains(&self.history_commits) {
            Ok(())
        } else {
            Err(ChangeProfileError::InvalidConfig(format!(
                "history_commits must be in 1..=10_000, got {}",
                self.history_commits
            )))
        }
    }
}

#[derive(Debug, Error)]
pub enum ChangeProfileError {
    #[error("invalid change-profile configuration: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    Repository(#[from] RepoError),
    #[error(transparent)]
    Metrics(#[from] MetricsError),
    #[error(
        "repository snapshot changed during analysis (before {before_revision}/{before_tree}, after {after_revision}/{after_tree})"
    )]
    SnapshotDrift {
        before_revision: String,
        before_tree: String,
        after_revision: String,
        after_tree: String,
    },
    #[error("change-profile invariant failed: {0}")]
    Invariant(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeProfileReport {
    pub artifact: ArtifactSnapshot,
    pub analyzer: String,
    pub history_coverage: HistoryCoverage,
    pub source_coverage: SourceCoverage,
    pub source_provenance: SourceProvenance,
    pub join_coverage: JoinCoverage,
    pub current_rows: Vec<CurrentFileRow>,
    pub history_only_rows: Vec<HistoryOnlyRow>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryCoverage {
    pub requested_commits: usize,
    pub commits_analyzed: usize,
    pub truncated: bool,
    pub earliest_committer_unix_seconds: Option<i64>,
    pub latest_committer_unix_seconds: Option<i64>,
    pub git_version: String,
    pub command: String,
    pub stdout_sha256: String,
    pub stdout_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceProvenance {
    pub git_version: String,
    pub ls_tree_command: String,
    pub ls_tree_stdout_sha256: String,
    pub ls_tree_stdout_bytes: u64,
    pub cat_file_command: String,
    pub cat_file_protocol: String,
    pub cat_file_request_sha256: String,
    pub cat_file_stdout_sha256: String,
    pub cat_file_stdout_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceCoverage {
    pub tracked_regular_files: usize,
    pub utf8_path_regular_files: usize,
    pub non_utf8_path_regular_files: usize,
    pub supported_source_files: usize,
    pub analyzed_source_files: usize,
    pub unsupported_regular_files: usize,
    pub syntax_error_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct JoinCoverage {
    pub current_analyzed_paths: usize,
    pub sampled_history_paths: usize,
    pub matched_paths: usize,
    pub current_without_history_paths: usize,
    pub historical_without_current_paths: usize,
    pub binary_touched_current_paths: usize,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinStatus {
    Matched,
    CurrentWithoutHistory,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Text,
    TextAndBinary,
    BinaryOnly,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentFileRow {
    pub path: String,
    pub path_bytes_hex: String,
    pub language: String,
    pub current_sloc: usize,
    pub current_cognitive: f64,
    pub current_cyclomatic: f64,
    pub cognitive_per_ksloc: Option<f64>,
    pub join_status: JoinStatus,
    pub commits_touched: u64,
    pub commit_touch_fraction: Option<f64>,
    pub active_change_days: u64,
    pub text_commits_touched: u64,
    pub binary_change_count: u64,
    pub line_change_mass: u64,
    pub line_change_mass_complete: bool,
    pub line_change_mass_per_current_sloc: Option<f64>,
    pub first_observed_change_unix_seconds: Option<i64>,
    pub last_observed_change_unix_seconds: Option<i64>,
    pub history_status: HistoryStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryOnlyRow {
    pub path: String,
    pub path_bytes_hex: String,
    pub commits_touched: u64,
    pub commit_touch_fraction: Option<f64>,
    pub active_change_days: u64,
    pub text_commits_touched: u64,
    pub binary_change_count: u64,
    pub line_change_mass: u64,
    pub line_change_mass_complete: bool,
    pub first_observed_change_unix_seconds: Option<i64>,
    pub last_observed_change_unix_seconds: Option<i64>,
    pub history_status: HistoryStatus,
}

#[derive(Default)]
struct HistoryAggregate {
    commits_touched: u64,
    days: BTreeSet<i64>,
    text_commits_touched: u64,
    binary_change_count: u64,
    line_change_mass: u64,
    first: Option<i64>,
    last: Option<i64>,
}

pub fn analyze_change_profile(
    root: &Path,
    config: ChangeProfileConfig,
) -> Result<ChangeProfileReport, ChangeProfileError> {
    config.validate()?;
    let artifact = snapshot_git_repo(root)?;
    let tree_scan = scan_committed_regular_files(&artifact)?;
    let tracked_regular_files = tree_scan.files.len();
    let mut utf8_path_regular_files = 0usize;
    let mut selected_files = Vec::new();
    let mut supported_paths = Vec::<(Vec<u8>, PathBuf)>::new();
    for entry in &tree_scan.files {
        if let Ok(path) = std::str::from_utf8(&entry.path) {
            utf8_path_regular_files =
                checked_usize_add(utf8_path_regular_files, 1, "UTF-8 path count")?;
            let relative = PathBuf::from(path);
            if supports_path(&relative) {
                selected_files.push(entry.clone());
                supported_paths.push((entry.path.clone(), relative));
            }
        }
    }
    let blob_read = read_committed_blobs(&artifact, &selected_files)?;
    if blob_read.blobs.len() != supported_paths.len() {
        return Err(ChangeProfileError::Invariant(
            "cat-file did not return exactly one blob per supported committed path".to_owned(),
        ));
    }
    let metric_inputs = supported_paths
        .iter()
        .zip(blob_read.blobs)
        .map(|((raw, path), bytes)| (path.clone(), display_path(raw), bytes))
        .collect::<Vec<_>>();
    let metrics = analyze_committed_files(&artifact.root, metric_inputs)?;
    if metrics.files.len() != supported_paths.len() {
        return Err(ChangeProfileError::Invariant(
            "metrics analyzer did not return exactly one row per supported committed blob"
                .to_owned(),
        ));
    }
    let unsupported_regular_files = utf8_path_regular_files
        .checked_sub(supported_paths.len())
        .ok_or_else(|| {
            ChangeProfileError::Invariant(
                "supported source count exceeded UTF-8-path regular-file count".to_owned(),
            )
        })?;
    let source_provenance = SourceProvenance {
        git_version: tree_scan.git_version,
        ls_tree_command: tree_scan.command,
        ls_tree_stdout_sha256: tree_scan.stdout_sha256,
        ls_tree_stdout_bytes: tree_scan.stdout_bytes,
        cat_file_command: blob_read.command,
        cat_file_protocol: "git-cat-file-batch-v1: request full object id plus LF; response object-id SP type SP size LF content LF, in request order".to_owned(),
        cat_file_request_sha256: blob_read.request_sha256,
        cat_file_stdout_sha256: blob_read.stdout_sha256,
        cat_file_stdout_bytes: blob_read.stdout_bytes,
    };

    let history = scan_file_history(&artifact, config.history_commits)?;
    let commits_analyzed = history.commits.len();
    let mut aggregates: BTreeMap<Vec<u8>, HistoryAggregate> = BTreeMap::new();
    let mut earliest = None;
    let mut latest = None;
    for commit in &history.commits {
        earliest = Some(earliest.map_or(commit.committer_unix_seconds, |v: i64| {
            v.min(commit.committer_unix_seconds)
        }));
        latest = Some(latest.map_or(commit.committer_unix_seconds, |v: i64| {
            v.max(commit.committer_unix_seconds)
        }));
        for (path, change) in &commit.files {
            let aggregate = aggregates.entry(path.clone()).or_default();
            aggregate.commits_touched =
                checked_u64_add(aggregate.commits_touched, 1, "commit touches")?;
            aggregate
                .days
                .insert(commit.committer_unix_seconds.div_euclid(86_400));
            if change.binary {
                aggregate.binary_change_count =
                    checked_u64_add(aggregate.binary_change_count, 1, "binary changes")?;
            }
            if change.text {
                aggregate.text_commits_touched =
                    checked_u64_add(aggregate.text_commits_touched, 1, "text commit touches")?;
                aggregate.line_change_mass = checked_u64_add(
                    aggregate.line_change_mass,
                    change.line_mass,
                    "line change mass",
                )?;
            }
            aggregate.first = Some(aggregate.first.map_or(commit.committer_unix_seconds, |v| {
                v.min(commit.committer_unix_seconds)
            }));
            aggregate.last = Some(aggregate.last.map_or(commit.committer_unix_seconds, |v| {
                v.max(commit.committer_unix_seconds)
            }));
        }
    }

    let mut metric_by_path = BTreeMap::new();
    for metric in metrics.files {
        metric_by_path.insert(metric.path.clone(), metric);
    }
    let current_identities = supported_paths
        .iter()
        .map(|(raw, _)| raw.clone())
        .collect::<BTreeSet<_>>();
    let mut current_rows = Vec::with_capacity(supported_paths.len());
    let mut matched_paths = 0usize;
    let mut binary_touched_current_paths = 0usize;
    for (raw, _relative) in &supported_paths {
        let path = display_path(raw);
        let metric = metric_by_path.remove(&path).ok_or_else(|| {
            ChangeProfileError::Invariant(format!("missing metric row for {path}"))
        })?;
        let aggregate = aggregates.get(raw);
        if aggregate.is_some() {
            matched_paths = checked_usize_add(matched_paths, 1, "matched path count")?;
        }
        if aggregate.is_some_and(|a| a.binary_change_count > 0) {
            binary_touched_current_paths =
                checked_usize_add(binary_touched_current_paths, 1, "binary current path count")?;
        }
        current_rows.push(current_row(raw, metric, aggregate, commits_analyzed)?);
    }
    let mut history_only_rows = Vec::new();
    for (path, aggregate) in &aggregates {
        if !current_identities.contains(path) {
            history_only_rows.push(history_only_row(path, aggregate, commits_analyzed)?);
        }
    }
    let current_without_history_paths =
        current_rows
            .len()
            .checked_sub(matched_paths)
            .ok_or_else(|| {
                ChangeProfileError::Invariant("matched paths exceeded current paths".to_owned())
            })?;
    let historical_without_current_paths = history_only_rows.len();
    if matched_paths.checked_add(current_without_history_paths) != Some(current_rows.len())
        || matched_paths.checked_add(historical_without_current_paths) != Some(aggregates.len())
    {
        return Err(ChangeProfileError::Invariant(
            "join coverage partitions do not close".to_owned(),
        ));
    }

    let after = snapshot_git_repo(&artifact.root)?;
    if after != artifact {
        return Err(ChangeProfileError::SnapshotDrift {
            before_revision: artifact.revision,
            before_tree: artifact.tree_digest,
            after_revision: after.revision,
            after_tree: after.tree_digest,
        });
    }
    Ok(ChangeProfileReport {
        artifact,
        analyzer: ANALYZER.to_owned(),
        history_coverage: HistoryCoverage {
            requested_commits: config.history_commits,
            commits_analyzed,
            truncated: history.truncated,
            earliest_committer_unix_seconds: earliest,
            latest_committer_unix_seconds: latest,
            git_version: history.git_version,
            command: history.command,
            stdout_sha256: history.stdout_sha256,
            stdout_bytes: history.stdout_bytes,
        },
        source_coverage: SourceCoverage {
            tracked_regular_files,
            utf8_path_regular_files,
            non_utf8_path_regular_files: tracked_regular_files - utf8_path_regular_files,
            supported_source_files: supported_paths.len(),
            analyzed_source_files: current_rows.len(),
            unsupported_regular_files,
            syntax_error_files: metrics.coverage.syntax_error_files,
        },
        source_provenance,
        join_coverage: JoinCoverage {
            current_analyzed_paths: current_rows.len(),
            sampled_history_paths: aggregates.len(),
            matched_paths,
            current_without_history_paths,
            historical_without_current_paths,
            binary_touched_current_paths,
        },
        current_rows,
        history_only_rows,
        limitations: limitations(config.history_commits),
    })
}

fn current_row(
    raw: &[u8],
    metric: crate::metrics::FileMetric,
    aggregate: Option<&HistoryAggregate>,
    commits: usize,
) -> Result<CurrentFileRow, ChangeProfileError> {
    let (commits_touched, fraction, days, text, binary, mass, first, last, status) =
        history_fields(aggregate, commits)?;
    let cognitive_per_ksloc = normalized(metric.cognitive, metric.sloc, true)?;
    let line_change_mass_per_current_sloc = if binary > 0 {
        None
    } else {
        normalized(mass as f64, metric.sloc, false)?
    };
    Ok(CurrentFileRow {
        path: display_path(raw),
        path_bytes_hex: hex(raw),
        language: metric.language,
        current_sloc: metric.sloc,
        current_cognitive: metric.cognitive,
        current_cyclomatic: metric.cyclomatic,
        cognitive_per_ksloc,
        join_status: if aggregate.is_some() {
            JoinStatus::Matched
        } else {
            JoinStatus::CurrentWithoutHistory
        },
        commits_touched,
        commit_touch_fraction: fraction,
        active_change_days: days,
        text_commits_touched: text,
        binary_change_count: binary,
        line_change_mass: mass,
        line_change_mass_complete: binary == 0,
        line_change_mass_per_current_sloc,
        first_observed_change_unix_seconds: first,
        last_observed_change_unix_seconds: last,
        history_status: status,
    })
}

fn history_only_row(
    raw: &[u8],
    aggregate: &HistoryAggregate,
    commits: usize,
) -> Result<HistoryOnlyRow, ChangeProfileError> {
    let (commits_touched, fraction, days, text, binary, mass, first, last, status) =
        history_fields(Some(aggregate), commits)?;
    Ok(HistoryOnlyRow {
        path: display_path(raw),
        path_bytes_hex: hex(raw),
        commits_touched,
        commit_touch_fraction: fraction,
        active_change_days: days,
        text_commits_touched: text,
        binary_change_count: binary,
        line_change_mass: mass,
        line_change_mass_complete: binary == 0,
        first_observed_change_unix_seconds: first,
        last_observed_change_unix_seconds: last,
        history_status: status,
    })
}

#[allow(clippy::type_complexity)]
fn history_fields(
    a: Option<&HistoryAggregate>,
    commits: usize,
) -> Result<
    (
        u64,
        Option<f64>,
        u64,
        u64,
        u64,
        u64,
        Option<i64>,
        Option<i64>,
        HistoryStatus,
    ),
    ChangeProfileError,
> {
    let Some(a) = a else {
        return Ok((
            0,
            if commits == 0 { None } else { Some(0.0) },
            0,
            0,
            0,
            0,
            None,
            None,
            HistoryStatus::None,
        ));
    };
    let fraction = if commits == 0 {
        None
    } else {
        Some(finite(
            a.commits_touched as f64 / commits as f64,
            "commit touch fraction",
        )?)
    };
    let days = u64::try_from(a.days.len())
        .map_err(|_| ChangeProfileError::Invariant("active day count overflowed u64".to_owned()))?;
    let status = match (a.text_commits_touched > 0, a.binary_change_count > 0) {
        (true, true) => HistoryStatus::TextAndBinary,
        (true, false) => HistoryStatus::Text,
        (false, true) => HistoryStatus::BinaryOnly,
        (false, false) => {
            return Err(ChangeProfileError::Invariant(
                "touched history path had neither text nor binary observation".to_owned(),
            ));
        }
    };
    Ok((
        a.commits_touched,
        fraction,
        days,
        a.text_commits_touched,
        a.binary_change_count,
        a.line_change_mass,
        a.first,
        a.last,
        status,
    ))
}

fn normalized(
    value: f64,
    sloc: usize,
    multiply_ksloc: bool,
) -> Result<Option<f64>, ChangeProfileError> {
    if sloc == 0 {
        return Ok(None);
    }
    let value = value / sloc as f64 * if multiply_ksloc { 1000.0 } else { 1.0 };
    Ok(Some(finite(value, "normalized metric")?))
}
fn finite(value: f64, name: &str) -> Result<f64, ChangeProfileError> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(ChangeProfileError::Invariant(format!(
            "{name} was nonfinite or negative"
        )))
    }
}
fn checked_u64_add(a: u64, b: u64, name: &str) -> Result<u64, ChangeProfileError> {
    a.checked_add(b)
        .ok_or_else(|| ChangeProfileError::Invariant(format!("{name} overflowed u64")))
}
fn checked_usize_add(a: usize, b: usize, name: &str) -> Result<usize, ChangeProfileError> {
    a.checked_add(b)
        .ok_or_else(|| ChangeProfileError::Invariant(format!("{name} overflowed usize")))
}
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}
fn display_path(path: &[u8]) -> String {
    std::str::from_utf8(path)
        .map(str::to_owned)
        .unwrap_or_else(|_| {
            path.iter()
                .map(|b| {
                    if b.is_ascii_graphic() && *b != b'\\' {
                        char::from(*b).to_string()
                    } else {
                        format!("\\x{b:02x}")
                    }
                })
                .collect()
        })
}
fn limitations(requested: usize) -> Vec<String> {
    vec![
        format!(
            "History is a bounded count window of at most {requested} non-merge commits; requested, actual, and truncation are reported explicitly."
        ),
        "Rename detection is disabled; paths are identities within the sampled window and rename continuity is not inferred."
            .to_owned(),
        "Committer timestamps define observed time and UTC change days; author time and elapsed durations are not used."
            .to_owned(),
        "Binary touches count toward commits, days, and timestamps but provide no textual line mass; normalized line mass is therefore null for any binary-touched path."
            .to_owned(),
        "Only committed regular blobs with UTF-8 paths and supported source extensions receive current structural metrics; raw Git path bytes remain the join identity, while untracked files and symlinks are excluded."
            .to_owned(),
        "Current source bytes come from the pinned Git objects; worktree presence, contents, hidden-path filtering, and ignore rules do not alter the committed-tree denominator."
            .to_owned(),
        "Structural and change measures remain separate proxies; this report provides no score, grade, quality label, risk label, threshold zone, or cross-language ranking."
            .to_owned(),
    ]
}

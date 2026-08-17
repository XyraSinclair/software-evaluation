//! A compact, receipt-bearing Pareto surface over the strongest fast
//! mechanical software-quality proxies already implemented by this crate.
//!
//! The frontier is deliberately not a score. It pairs mutually policing
//! signals, preserves missingness and censoring, and defines only a strict
//! partial order between two commit-pinned artifacts.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::discipline::analyze_discipline;
use crate::duplicates::{DuplicateConfig, analyze_duplicates};
use crate::kernel::ArtifactSnapshot;
use crate::repo::snapshot_git_repo;
use crate::shape::analyze_shape;
use crate::source::SourceCorpusSession;
use crate::symbols::analyze_symbols;

mod compare;
mod signals;

pub use compare::{compare_paths, compare_profiles};

pub const FRONTIER_SCHEMA_VERSION: &str = "seval.frontier.v1";

const SHAPE: &str = "shape";
const SYMBOLS: &str = "symbols";
const DISCIPLINE: &str = "discipline";
const DUPLICATES: &str = "duplicates";

pub(super) const SIGNAL_IDS: [&str; 6] = [
    "reader.local-cognitive-p90",
    "reader.symbol-working-set-p90-fraction",
    "interface.shallow-function-fraction",
    "effects.syntactic-pure-fraction",
    "effects.mutable-live-range-p90-lines",
    "uniformity.reported-clone-token-density",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FrontierConfig {
    pub duplicate_min_tokens: usize,
    pub duplicate_min_lines: usize,
    pub duplicate_max_groups: usize,
    /// Coverage gate, not a quality threshold. Below this fraction the symbol
    /// working-set value remains visible but cannot participate in dominance.
    pub min_symbol_resolution_fraction: f64,
}

impl Default for FrontierConfig {
    fn default() -> Self {
        Self {
            duplicate_min_tokens: 40,
            duplicate_min_lines: 5,
            duplicate_max_groups: 100,
            min_symbol_resolution_fraction: 0.50,
        }
    }
}

impl FrontierConfig {
    fn validate(&self) -> Result<(), FrontierError> {
        if self.duplicate_min_tokens == 0
            || self.duplicate_min_lines == 0
            || self.duplicate_max_groups == 0
        {
            return Err(FrontierError::InvalidConfig(
                "duplicate bounds must be greater than zero".to_owned(),
            ));
        }
        if !self.min_symbol_resolution_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.min_symbol_resolution_fraction)
        {
            return Err(FrontierError::InvalidConfig(
                "min_symbol_resolution_fraction must be finite and in 0..=1".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum FrontierError {
    #[error("cannot inspect frontier input {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("frontier input is a symbolic link and is not followed: {0}")]
    Symlink(PathBuf),
    #[error("frontier input is neither a regular file nor a directory: {0}")]
    UnsupportedInput(PathBuf),
    #[error("invalid frontier configuration: {0}")]
    InvalidConfig(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct FrontierProfile {
    pub schema_version: String,
    pub artifact: FrontierArtifact,
    pub config: FrontierConfig,
    pub elapsed_ms: u128,
    pub analyzers: Vec<AnalyzerReceipt>,
    pub signals: Vec<FrontierSignal>,
    pub families: Vec<SignalFamily>,
    pub coverage: DirectionalCoverage,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrontierArtifact {
    pub input: String,
    /// Present only when a clean Git snapshot remained unchanged across the
    /// complete scan.
    pub git: Option<ArtifactSnapshot>,
    pub identity_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalyzerStatus {
    Complete,
    Failed,
    Panicked,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzerReceipt {
    pub id: String,
    pub status: AnalyzerStatus,
    pub implementation: Option<String>,
    pub elapsed_ms: u128,
    pub payload_sha256: Option<String>,
    pub coverage: Option<Value>,
    pub limitations: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalPolarity {
    LowerIsBetter,
    HigherIsBetter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalStatus {
    Observed,
    Missing,
    Censored,
    InsufficientCoverage,
    SourceFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrontierSignal {
    pub id: String,
    pub family: String,
    pub label: String,
    pub polarity: SignalPolarity,
    pub status: SignalStatus,
    pub value: Option<f64>,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub unit: String,
    pub analyzer_id: String,
    pub json_pointers: Vec<String>,
    pub note: String,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalFamily {
    pub id: String,
    pub label: String,
    pub signal_ids: Vec<String>,
    pub rule: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectionalCoverage {
    pub declared: usize,
    pub observed: usize,
    pub unusable_signal_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalOutcome {
    RightBetter,
    LeftBetter,
    Equivalent,
    Unavailable,
    Incompatible,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalComparison {
    pub id: String,
    pub family: String,
    pub label: String,
    pub polarity: SignalPolarity,
    pub left_status: SignalStatus,
    pub right_status: SignalStatus,
    pub left_value: Option<f64>,
    pub right_value: Option<f64>,
    pub right_minus_left: Option<f64>,
    pub outcome: SignalOutcome,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartialOrder {
    RightDominates,
    LeftDominates,
    Tradeoff,
    Equivalent,
    NoComparableSignals,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonSlice {
    pub complete: bool,
    pub comparable_signals: usize,
    pub unusable_signal_ids: Vec<String>,
    pub order_on_observed_intersection: PartialOrder,
}

#[derive(Debug, Clone, Serialize)]
pub struct FamilyComparison {
    pub id: String,
    pub label: String,
    pub comparison: ComparisonSlice,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonReadiness {
    pub schema_compatible: bool,
    pub analysis_config_compatible: bool,
    pub analyzer_implementations_compatible: bool,
    pub directional_signals_complete: bool,
    pub artifacts_commit_pinned: bool,
    pub qualified: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrontierComparison {
    pub schema_version: String,
    pub left: FrontierProfile,
    pub right: FrontierProfile,
    pub readiness: ComparisonReadiness,
    pub order_on_observed_intersection: PartialOrder,
    /// Present only when both artifacts are commit-pinned, configurations
    /// match, and all six declared signals are usable.
    pub qualified_order: Option<PartialOrder>,
    pub families: Vec<FamilyComparison>,
    pub signals: Vec<SignalComparison>,
    pub limitations: Vec<String>,
}

#[derive(Debug)]
struct AnalyzerRun {
    receipt: AnalyzerReceipt,
    value: Option<Value>,
}

pub fn profile_path(
    input: &Path,
    config: &FrontierConfig,
) -> Result<FrontierProfile, FrontierError> {
    config.validate()?;
    let metadata = fs::symlink_metadata(input).map_err(|source| FrontierError::Inspect {
        path: input.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(FrontierError::Symlink(input.to_owned()));
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(FrontierError::UnsupportedInput(input.to_owned()));
    }

    let started = Instant::now();
    let before = git_snapshot(input, metadata.is_dir());
    let (source_corpus, source_corpus_error) = match SourceCorpusSession::activate(input) {
        Ok(corpus) => (Some(corpus), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let duplicate_config = DuplicateConfig {
        min_tokens: config.duplicate_min_tokens,
        min_lines: config.duplicate_min_lines,
        max_groups: config.duplicate_max_groups,
    };
    let runs = thread::scope(|scope| {
        let corpus = source_corpus.as_ref();
        let shape = scope.spawn(|| {
            with_source_corpus(corpus, || run_analyzer(SHAPE, || analyze_shape(input)))
        });
        let symbols = scope.spawn(|| {
            with_source_corpus(corpus, || run_analyzer(SYMBOLS, || analyze_symbols(input)))
        });
        let discipline = scope.spawn(|| {
            with_source_corpus(corpus, || {
                run_analyzer(DISCIPLINE, || analyze_discipline(input))
            })
        });
        let duplicates = scope.spawn(|| {
            with_source_corpus(corpus, || {
                run_analyzer(DUPLICATES, || {
                    analyze_duplicates(input, &duplicate_config)
                })
            })
        });
        vec![
            join_analyzer(SHAPE, shape.join()),
            join_analyzer(SYMBOLS, symbols.join()),
            join_analyzer(DISCIPLINE, discipline.join()),
            join_analyzer(DUPLICATES, duplicates.join()),
        ]
    });
    let after = git_snapshot(input, metadata.is_dir());

    let mut values = BTreeMap::new();
    let mut analyzers = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(value) = run.value {
            values.insert(run.receipt.id.clone(), value);
        }
        analyzers.push(run.receipt);
    }
    let signals = signals::project(&values, config);
    let mut limitations = signals::limitations();
    if let Some(error) = source_corpus_error {
        limitations.push(format!(
            "Shared source-corpus materialization failed ({error}); analyzers fell back to independent discovery, reads, and parses."
        ));
    }

    Ok(FrontierProfile {
        schema_version: FRONTIER_SCHEMA_VERSION.to_owned(),
        artifact: artifact_identity(input, before, after),
        config: config.clone(),
        elapsed_ms: started.elapsed().as_millis(),
        analyzers,
        coverage: signals::coverage(&signals),
        signals,
        families: signals::families(),
        limitations,
    })
}

fn with_source_corpus<T>(
    corpus: Option<&SourceCorpusSession>,
    operation: impl FnOnce() -> T,
) -> T {
    match corpus {
        Some(corpus) => corpus.scope(operation),
        None => operation(),
    }
}

fn run_analyzer<T, E, F>(id: &str, analyze: F) -> AnalyzerRun
where
    T: Serialize,
    E: std::fmt::Display,
    F: FnOnce() -> Result<T, E>,
{
    let started = Instant::now();
    match analyze() {
        Ok(report) => match serde_json::to_vec(&report)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map(|value| (bytes, value)))
        {
            Ok((bytes, value)) => AnalyzerRun {
                receipt: AnalyzerReceipt {
                    id: id.to_owned(),
                    status: AnalyzerStatus::Complete,
                    implementation: value
                        .get("analyzer")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    elapsed_ms: started.elapsed().as_millis(),
                    payload_sha256: Some(sha256_hex(&bytes)),
                    coverage: value.get("coverage").cloned(),
                    limitations: string_array(value.get("limitations")),
                    error: None,
                },
                value: Some(value),
            },
            Err(error) => failed_run(
                id,
                AnalyzerStatus::Failed,
                started.elapsed().as_millis(),
                format!("observation serialization failed: {error}"),
            ),
        },
        Err(error) => failed_run(
            id,
            AnalyzerStatus::Failed,
            started.elapsed().as_millis(),
            error.to_string(),
        ),
    }
}

fn join_analyzer(id: &str, result: thread::Result<AnalyzerRun>) -> AnalyzerRun {
    match result {
        Ok(run) => run,
        Err(payload) => failed_run(id, AnalyzerStatus::Panicked, 0, panic_message(&payload)),
    }
}

fn failed_run(id: &str, status: AnalyzerStatus, elapsed_ms: u128, error: String) -> AnalyzerRun {
    AnalyzerRun {
        receipt: AnalyzerReceipt {
            id: id.to_owned(),
            status,
            implementation: None,
            elapsed_ms,
            payload_sha256: None,
            coverage: None,
            limitations: Vec::new(),
            error: Some(error),
        },
        value: None,
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "analyzer panicked with a non-string payload".to_owned())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut rendered, "{byte:02x}");
    }
    rendered
}

fn git_snapshot(input: &Path, is_directory: bool) -> Option<Result<ArtifactSnapshot, String>> {
    is_directory.then(|| strict_git_snapshot(input))
}

fn strict_git_snapshot(input: &Path) -> Result<ArtifactSnapshot, String> {
    let artifact = snapshot_git_repo(input).map_err(|error| error.to_string())?;
    let output = Command::new("git")
        .current_dir(&artifact.root)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .map_err(|error| format!("failed to inspect untracked files: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files --others failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if !output.stdout.is_empty() {
        return Err("repository has non-ignored untracked files".to_owned());
    }
    Ok(artifact)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn artifact_identity(
    input: &Path,
    before: Option<Result<ArtifactSnapshot, String>>,
    after: Option<Result<ArtifactSnapshot, String>>,
) -> FrontierArtifact {
    let input = input.to_string_lossy().into_owned();
    match (before, after) {
        (Some(Ok(before)), Some(Ok(after))) if before == after => FrontierArtifact {
            input,
            git: Some(before),
            identity_error: None,
        },
        (Some(Ok(before)), Some(Ok(after))) => FrontierArtifact {
            input,
            git: None,
            identity_error: Some(format!(
                "snapshot changed during scan: {}/{} -> {}/{}",
                before.revision, before.tree_digest, after.revision, after.tree_digest
            )),
        },
        (Some(Err(before)), Some(Err(after))) if before == after => FrontierArtifact {
            input,
            git: None,
            identity_error: Some(before),
        },
        (Some(before), Some(after)) => FrontierArtifact {
            input,
            git: None,
            identity_error: Some(format!(
                "snapshot identity was unstable: before={before:?}, after={after:?}"
            )),
        },
        _ => FrontierArtifact {
            input,
            git: None,
            identity_error: Some("single-file input cannot be commit-pinned".to_owned()),
        },
    }
}

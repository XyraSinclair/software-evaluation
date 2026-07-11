use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const MAX_RUNS: u32 = 10_000;
const MAX_TIMEOUT_MS: u64 = 86_400_000;
const POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug, Clone)]
pub struct BenchmarkSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub warmup_runs: u32,
    pub measured_runs: u32,
    pub timeout_ms: u64,
    pub workload_units: Option<u64>,
    pub workload_bytes: Option<u64>,
}

#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("measured_runs must be between 1 and {MAX_RUNS}, got {0}")]
    InvalidMeasuredRuns(u32),
    #[error("warmup_runs must not exceed {MAX_RUNS}, got {0}")]
    InvalidWarmupRuns(u32),
    #[error("timeout_ms must be between 1 and {MAX_TIMEOUT_MS}, got {0}")]
    InvalidTimeout(u64),
    #[error("benchmark working directory `{path}` is not a readable directory: {message}")]
    InvalidWorkingDirectory { path: String, message: String },
    #[error("benchmark executable `{program}` could not be located or read")]
    ExecutableNotFound { program: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: BenchmarkCoverage,
    pub limitations: Vec<String>,
    pub command: CommandProvenance,
    pub environment: BenchmarkEnvironment,
    pub timestamp_unix_ns: u128,
    pub successful: bool,
    pub warmups: Vec<RunReceipt>,
    pub first_measured_run: RunReceipt,
    pub warmed_samples: Vec<RunReceipt>,
    pub warmed_latency_ns: LatencyDistribution,
    pub warmed_units_per_second: Option<RateDistribution>,
    pub warmed_bytes_per_second: Option<RateDistribution>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkCoverage {
    pub requested_warmup_runs: u32,
    pub observed_warmup_runs: usize,
    pub requested_measured_runs: u32,
    pub observed_measured_runs: usize,
    pub warmed_distribution_denominator: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandProvenance {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub identity_sha256: String,
    pub executable_locator: String,
    pub executable_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkEnvironment {
    pub os: String,
    pub architecture: String,
    pub environment_inheritance: String,
    pub selected_environment: BTreeMap<String, String>,
    pub timer: String,
    pub timeout_mechanism: String,
    pub peak_rss_mechanism: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReceipt {
    pub ordinal: u32,
    pub started: bool,
    pub spawn_error: Option<String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub termination_signal: Option<i32>,
    pub success: bool,
    pub elapsed_ns: u128,
    pub timed_out: bool,
    pub termination_occurred: bool,
    pub peak_rss_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LatencyDistribution {
    pub sample_count: usize,
    pub min_ns: Option<u128>,
    pub p50_ns: Option<u128>,
    pub p95_ns: Option<u128>,
    pub p99_ns: Option<u128>,
    pub max_ns: Option<u128>,
    pub mean_ns: Option<u128>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RateDistribution {
    pub sample_count: usize,
    pub numerator_per_sample: u64,
    pub denominator: String,
    pub min_per_second: Option<f64>,
    pub p50_per_second: Option<f64>,
    pub p95_per_second: Option<f64>,
    pub p99_per_second: Option<f64>,
    pub max_per_second: Option<f64>,
    pub mean_per_second: Option<f64>,
}

pub fn run_benchmark(spec: &BenchmarkSpec) -> Result<BenchmarkReport, BenchmarkError> {
    validate(spec)?;
    let cwd = resolve_cwd(spec)?;
    let executable = locate_executable(&spec.program, &cwd).ok_or_else(|| {
        BenchmarkError::ExecutableNotFound {
            program: spec.program.clone(),
        }
    })?;

    let mut warmups = Vec::with_capacity(spec.warmup_runs as usize);
    for ordinal in 0..spec.warmup_runs {
        warmups.push(run_once(spec, &cwd, ordinal));
    }
    let mut measured = Vec::with_capacity(spec.measured_runs as usize);
    for ordinal in 0..spec.measured_runs {
        measured.push(run_once(spec, &cwd, ordinal));
    }
    let first_measured_run = measured.remove(0);
    let warmed_latency_ns = latency_distribution(&measured);
    let warmed_units_per_second = spec.workload_units.map(|n| rate_distribution(&measured, n));
    let warmed_bytes_per_second = spec.workload_bytes.map(|n| rate_distribution(&measured, n));
    let successful = warmups
        .iter()
        .chain(std::iter::once(&first_measured_run))
        .chain(measured.iter())
        .all(|run| run.success);

    let timestamp_unix_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let selected_environment = ["LANG", "LC_ALL", "TZ"]
        .into_iter()
        .filter_map(|key| env::var(key).ok().map(|value| (key.to_owned(), value)))
        .collect();
    let cwd_string = cwd.to_string_lossy().into_owned();

    Ok(BenchmarkReport {
        root: cwd_string.clone(),
        analyzer: "seval-direct-argv-benchmark-v1".to_owned(),
        coverage: BenchmarkCoverage {
            requested_warmup_runs: spec.warmup_runs,
            observed_warmup_runs: warmups.len(),
            requested_measured_runs: spec.measured_runs,
            observed_measured_runs: measured.len() + 1,
            warmed_distribution_denominator: measured.len(),
        },
        limitations: vec![
            "The first measured run is reported separately, but is not evidence of a cold filesystem, CPU, allocator, or application cache.".to_owned(),
            "Peak RSS is null because stable Rust exposes no portable, per-child peak-RSS measurement that can be attached reliably to this wait path.".to_owned(),
            "Executable version is not probed: invoking an arbitrary benchmark target with an assumed version flag is not guaranteed to be safe or meaningful.".to_owned(),
            "Wall-clock timings include process creation, output capture, and scheduler noise; stdout and stderr capture can perturb workloads that produce substantial output.".to_owned(),
            "The warmed distribution includes failed and timed-out measured runs so its denominator remains explicit; interpret latency alongside each run status.".to_owned(),
        ],
        command: CommandProvenance {
            program: spec.program.clone(),
            args: spec.args.clone(),
            cwd: cwd_string,
            identity_sha256: command_identity(spec, &cwd),
            executable_locator: executable.to_string_lossy().into_owned(),
            executable_version: None,
        },
        environment: BenchmarkEnvironment {
            os: env::consts::OS.to_owned(),
            architecture: env::consts::ARCH.to_owned(),
            environment_inheritance: "Child inherits the parent environment unchanged; LANG, LC_ALL, and TZ are recorded when present. Other inherited variables are omitted to avoid disclosing credentials.".to_owned(),
            selected_environment,
            timer: "std::time::Instant monotonic elapsed time".to_owned(),
            timeout_mechanism: "poll try_wait; on deadline call Child::kill, then Child::wait to reap; stdout and stderr are drained concurrently".to_owned(),
            peak_rss_mechanism: None,
        },
        timestamp_unix_ns,
        successful,
        warmups,
        first_measured_run,
        warmed_samples: measured,
        warmed_latency_ns,
        warmed_units_per_second,
        warmed_bytes_per_second,
    })
}

fn validate(spec: &BenchmarkSpec) -> Result<(), BenchmarkError> {
    if spec.measured_runs == 0 || spec.measured_runs > MAX_RUNS {
        return Err(BenchmarkError::InvalidMeasuredRuns(spec.measured_runs));
    }
    if spec.warmup_runs > MAX_RUNS {
        return Err(BenchmarkError::InvalidWarmupRuns(spec.warmup_runs));
    }
    if spec.timeout_ms == 0 || spec.timeout_ms > MAX_TIMEOUT_MS {
        return Err(BenchmarkError::InvalidTimeout(spec.timeout_ms));
    }
    Ok(())
}

fn resolve_cwd(spec: &BenchmarkSpec) -> Result<PathBuf, BenchmarkError> {
    let cwd = match &spec.cwd {
        Some(path) => path.clone(),
        None => env::current_dir().map_err(|error| BenchmarkError::InvalidWorkingDirectory {
            path: ".".to_owned(),
            message: error.to_string(),
        })?,
    };
    let metadata = fs::metadata(&cwd).map_err(|error| BenchmarkError::InvalidWorkingDirectory {
        path: cwd.to_string_lossy().into_owned(),
        message: error.to_string(),
    })?;
    if !metadata.is_dir() {
        return Err(BenchmarkError::InvalidWorkingDirectory {
            path: cwd.to_string_lossy().into_owned(),
            message: "not a directory".to_owned(),
        });
    }
    fs::canonicalize(&cwd).map_err(|error| BenchmarkError::InvalidWorkingDirectory {
        path: cwd.to_string_lossy().into_owned(),
        message: error.to_string(),
    })
}

fn locate_executable(program: &str, cwd: &Path) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        let path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        return readable_file(path);
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .filter_map(|directory| readable_file(directory.join(program)))
        .next()
}

fn readable_file(path: PathBuf) -> Option<PathBuf> {
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || fs::File::open(&path).is_err() || !is_executable(&metadata) {
        return None;
    }
    fs::canonicalize(path).ok()
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

fn run_once(spec: &BenchmarkSpec, cwd: &Path, ordinal: u32) -> RunReceipt {
    let started_at = Instant::now();
    let spawned = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return spawn_failure(ordinal, started_at.elapsed(), error),
    };
    let stdout_thread = child.stdout.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    });
    let stderr_thread = child.stderr.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    });
    let deadline = Duration::from_millis(spec.timeout_ms);
    let (status, timed_out, termination_occurred) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false, false),
            Ok(None) if started_at.elapsed() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let kill_attempted = child.kill().is_ok();
                break (child.wait().ok(), true, kill_attempted);
            }
            Err(_) => {
                let kill_attempted = child.kill().is_ok();
                break (child.wait().ok(), false, kill_attempted);
            }
        }
    };
    let elapsed_ns = started_at.elapsed().as_nanos();
    let stdout = join_capture(stdout_thread);
    let stderr = join_capture(stderr_thread);
    let (exit_code, termination_signal) = status_parts(status.as_ref());
    let success = !timed_out && status.as_ref().is_some_and(ExitStatus::success);
    RunReceipt {
        ordinal,
        started: true,
        spawn_error: None,
        stdout,
        stderr,
        exit_code,
        termination_signal,
        success,
        elapsed_ns,
        timed_out,
        termination_occurred,
        peak_rss_bytes: None,
    }
}

fn spawn_failure(ordinal: u32, elapsed: Duration, error: io::Error) -> RunReceipt {
    RunReceipt {
        ordinal,
        started: false,
        spawn_error: Some(error.to_string()),
        stdout: Vec::new(),
        stderr: Vec::new(),
        exit_code: None,
        termination_signal: None,
        success: false,
        elapsed_ns: elapsed.as_nanos(),
        timed_out: false,
        termination_occurred: false,
        peak_rss_bytes: None,
    }
}

fn join_capture(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default()
}

#[cfg(unix)]
fn status_parts(status: Option<&ExitStatus>) -> (Option<i32>, Option<i32>) {
    use std::os::unix::process::ExitStatusExt;
    (
        status.and_then(ExitStatus::code),
        status.and_then(ExitStatusExt::signal),
    )
}

#[cfg(not(unix))]
fn status_parts(status: Option<&ExitStatus>) -> (Option<i32>, Option<i32>) {
    (status.and_then(ExitStatus::code), None)
}

fn latency_distribution(samples: &[RunReceipt]) -> LatencyDistribution {
    let mut values: Vec<u128> = samples.iter().map(|sample| sample.elapsed_ns).collect();
    values.sort_unstable();
    LatencyDistribution {
        sample_count: values.len(),
        min_ns: values.first().copied(),
        p50_ns: nearest_rank(&values, 50),
        p95_ns: nearest_rank(&values, 95),
        p99_ns: nearest_rank(&values, 99),
        max_ns: values.last().copied(),
        mean_ns: integer_mean(&values),
    }
}

fn rate_distribution(samples: &[RunReceipt], numerator: u64) -> RateDistribution {
    let mut values: Vec<f64> = samples
        .iter()
        .map(|sample| {
            let elapsed_ns = sample.elapsed_ns.max(1);
            numerator as f64 * 1_000_000_000.0 / elapsed_ns as f64
        })
        .collect();
    values.sort_by(f64::total_cmp);
    RateDistribution {
        sample_count: values.len(),
        numerator_per_sample: numerator,
        denominator: "elapsed_seconds_per_warmed_measured_sample".to_owned(),
        min_per_second: values.first().copied(),
        p50_per_second: nearest_rank(&values, 50),
        p95_per_second: nearest_rank(&values, 95),
        p99_per_second: nearest_rank(&values, 99),
        max_per_second: values.last().copied(),
        mean_per_second: (!values.is_empty())
            .then(|| values.iter().sum::<f64>() / values.len() as f64),
    }
}

fn nearest_rank<T: Copy>(sorted: &[T], percentile: usize) -> Option<T> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (percentile * sorted.len()).div_ceil(100);
    sorted.get(rank.saturating_sub(1)).copied()
}

fn integer_mean(values: &[u128]) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    let count = values.len() as u128;
    let quotient_sum = values.iter().map(|value| value / count).sum::<u128>();
    let remainder_sum = values.iter().map(|value| value % count).sum::<u128>();
    Some(quotient_sum + remainder_sum / count)
}

fn command_identity(spec: &BenchmarkSpec, cwd: &Path) -> String {
    let mut digest = Sha256::new();
    hash_field(&mut digest, spec.program.as_bytes());
    for arg in &spec.args {
        hash_field(&mut digest, arg.as_bytes());
    }
    hash_field(&mut digest, cwd.as_os_str().to_string_lossy().as_bytes());
    let bytes = digest.finalize();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

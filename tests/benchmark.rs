use std::fs;
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::benchmark::{
    BenchmarkError, BenchmarkReport, BenchmarkSpec, RateDistribution, RunReceipt, run_benchmark,
};
use tempfile::TempDir;

const PRINTF: &str = "/usr/bin/printf";
const FALSE: &str = "/usr/bin/false";
const SLEEP: &str = "/bin/sleep";
const CAT: &str = "/bin/cat";

fn spec(program: &str, args: &[&str]) -> BenchmarkSpec {
    BenchmarkSpec {
        program: program.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: None,
        warmup_runs: 0,
        measured_runs: 1,
        timeout_ms: 5_000,
        workload_units: None,
        workload_bytes: None,
    }
}

fn all_measured(report: &BenchmarkReport) -> impl Iterator<Item = &RunReceipt> {
    std::iter::once(&report.first_measured_run).chain(report.warmed_samples.iter())
}

fn assert_no_judgment_fields(value: &Value) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("benchmark JSON root must be an object: {value}"));
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(
            !object.contains_key(forbidden),
            "observation report must not contain judgment field {forbidden:?}"
        );
    }
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .args(arguments)
        .output()
        .expect("seval benchmark CLI must start")
}

fn json_output(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "CLI stdout was not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn nearest_rank_u128(mut values: Vec<u128>, percentile: usize) -> Option<u128> {
    values.sort_unstable();
    let rank = (percentile * values.len()).div_ceil(100);
    rank.checked_sub(1)
        .and_then(|index| values.get(index))
        .copied()
}

fn expected_rates(samples: &[RunReceipt], numerator: u64) -> Vec<f64> {
    let mut rates = samples
        .iter()
        .map(|sample| numerator as f64 * 1_000_000_000.0 / sample.elapsed_ns.max(1) as f64)
        .collect::<Vec<_>>();
    rates.sort_by(f64::total_cmp);
    rates
}

fn assert_rate_distribution(
    distribution: &RateDistribution,
    samples: &[RunReceipt],
    numerator: u64,
) {
    // Precommitted oracle: every warmed sample contributes exactly one rate,
    // rate=numerator/elapsed-seconds, and nearest-rank k selects ceil(k*n/100).
    let expected = expected_rates(samples, numerator);
    let rank = |percentile: usize| {
        let index = (percentile * expected.len()).div_ceil(100) - 1;
        expected[index]
    };
    let mean = expected.iter().sum::<f64>() / expected.len() as f64;

    assert_eq!(distribution.sample_count, samples.len());
    assert_eq!(distribution.numerator_per_sample, numerator);
    assert_eq!(
        distribution.denominator,
        "elapsed_seconds_per_warmed_measured_sample"
    );
    assert_eq!(distribution.min_per_second, expected.first().copied());
    assert_eq!(distribution.p50_per_second, Some(rank(50)));
    assert_eq!(distribution.p95_per_second, Some(rank(95)));
    assert_eq!(distribution.p99_per_second, Some(rank(99)));
    assert_eq!(distribution.max_per_second, expected.last().copied());
    assert_eq!(distribution.mean_per_second, Some(mean));
}

#[test]
fn exact_argv_and_both_output_streams_are_captured_as_bytes() {
    let mut exact_argv = spec(PRINTF, &["%s|%s", "argument with spaces", "tail"]);
    exact_argv.measured_runs = 2;

    let report = run_benchmark(&exact_argv).expect("printf benchmark must run");

    assert!(report.successful);
    assert_eq!(report.command.program, PRINTF);
    assert_eq!(report.command.args, exact_argv.args);
    for receipt in all_measured(&report) {
        assert!(receipt.started);
        assert_eq!(receipt.exit_code, Some(0));
        assert_eq!(receipt.stdout, b"argument with spaces|tail");
        assert!(receipt.stderr.is_empty());
        assert!(receipt.spawn_error.is_none());
    }

    let directory = TempDir::new().expect("temporary cwd");
    let missing = "missing file with spaces";
    let reference = Command::new(CAT)
        .arg(missing)
        .current_dir(directory.path())
        .output()
        .expect("direct cat positive control must start");
    assert!(!reference.status.success());
    assert!(reference.stdout.is_empty());
    assert!(
        !reference.stderr.is_empty(),
        "positive control must prove the stderr probe produces findings"
    );

    let mut stderr_spec = spec(CAT, &[missing]);
    stderr_spec.cwd = Some(directory.path().to_path_buf());
    let stderr_report = run_benchmark(&stderr_spec).expect("cat benchmark must be observed");
    let receipt = &stderr_report.first_measured_run;
    assert!(receipt.started);
    assert_eq!(receipt.exit_code, reference.status.code());
    assert_eq!(receipt.stdout, reference.stdout);
    assert_eq!(receipt.stderr, reference.stderr);
    assert!(!receipt.success);
}

#[test]
fn failures_and_timeouts_preserve_requested_sample_denominators_and_terminal_evidence() {
    let mut failure_spec = spec(FALSE, &[]);
    failure_spec.warmup_runs = 2;
    failure_spec.measured_runs = 4;

    let failure = run_benchmark(&failure_spec).expect("nonzero runs are observations, not errors");

    assert!(!failure.successful);
    assert_eq!(failure.warmups.len(), 2);
    assert_eq!(failure.coverage.observed_warmup_runs, 2);
    assert_eq!(failure.coverage.observed_measured_runs, 4);
    assert_eq!(failure.coverage.warmed_distribution_denominator, 3);
    assert_eq!(failure.warmed_samples.len(), 3);
    assert_eq!(failure.warmed_latency_ns.sample_count, 3);
    for receipt in failure.warmups.iter().chain(all_measured(&failure)) {
        assert!(receipt.stdout.is_empty());
        assert!(receipt.stderr.is_empty());
        assert!(
            receipt.started && receipt.exit_code.is_some(),
            "empty output is evidence only when start and process exit are observed: {receipt:?}"
        );
        assert!(!receipt.success);
    }

    let mut timeout_spec = spec(SLEEP, &["1"]);
    timeout_spec.measured_runs = 2;
    timeout_spec.timeout_ms = 10;
    let timeout = run_benchmark(&timeout_spec).expect("timeouts must produce receipts");

    assert!(!timeout.successful);
    assert_eq!(timeout.coverage.observed_measured_runs, 2);
    assert_eq!(timeout.warmed_samples.len(), 1);
    assert_eq!(timeout.warmed_latency_ns.sample_count, 1);
    for receipt in all_measured(&timeout) {
        assert!(receipt.started);
        assert!(receipt.timed_out);
        assert!(receipt.termination_occurred);
        assert!(!receipt.success);
        assert!(receipt.spawn_error.is_none());
        assert!(receipt.exit_code.is_some() || receipt.termination_signal.is_some());
    }
}

#[test]
fn first_run_is_separate_and_warmed_distributions_use_nearest_rank_and_optional_rates() {
    let mut benchmark = spec(PRINTF, &[""]);
    benchmark.warmup_runs = 2;
    benchmark.measured_runs = 5;
    benchmark.workload_units = Some(40);
    benchmark.workload_bytes = Some(4_096);

    // Precommitted smallest-case relations before production runs:
    // measured=5 => one first + four warmed; ranks over four are p50=item 2,
    // while p95 and p99 are item 4. Warmups never enter either denominator.
    let expected_warmups = 2;
    let expected_measured = 5;
    let expected_warmed = 4;
    let report = run_benchmark(&benchmark).expect("distribution benchmark must run");

    assert_eq!(report.coverage.requested_warmup_runs, expected_warmups);
    assert_eq!(
        report.coverage.observed_warmup_runs,
        expected_warmups as usize
    );
    assert_eq!(report.coverage.requested_measured_runs, expected_measured);
    assert_eq!(
        report.coverage.observed_measured_runs,
        expected_measured as usize
    );
    assert_eq!(
        report.coverage.warmed_distribution_denominator,
        expected_warmed
    );
    assert_eq!(report.warmups.len(), expected_warmups as usize);
    assert_eq!(report.first_measured_run.ordinal, 0);
    assert_eq!(report.warmed_samples.len(), expected_warmed);
    assert_eq!(
        report
            .warmed_samples
            .iter()
            .map(|sample| sample.ordinal)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );

    let elapsed = report
        .warmed_samples
        .iter()
        .map(|sample| sample.elapsed_ns)
        .collect::<Vec<_>>();
    let latency = &report.warmed_latency_ns;
    assert_eq!(latency.sample_count, expected_warmed);
    assert_eq!(latency.min_ns, elapsed.iter().min().copied());
    assert_eq!(latency.p50_ns, nearest_rank_u128(elapsed.clone(), 50));
    assert_eq!(latency.p95_ns, nearest_rank_u128(elapsed.clone(), 95));
    assert_eq!(latency.p99_ns, nearest_rank_u128(elapsed.clone(), 99));
    assert_eq!(latency.max_ns, elapsed.iter().max().copied());

    assert_rate_distribution(
        report
            .warmed_units_per_second
            .as_ref()
            .expect("unit rate requested"),
        &report.warmed_samples,
        40,
    );
    assert_rate_distribution(
        report
            .warmed_bytes_per_second
            .as_ref()
            .expect("byte rate requested"),
        &report.warmed_samples,
        4_096,
    );
}

#[test]
fn command_identity_is_stable_for_the_same_argv_and_changes_with_argv_or_cwd() {
    let first_dir = TempDir::new().expect("first cwd");
    let second_dir = TempDir::new().expect("second cwd");
    let mut base = spec(PRINTF, &["%s", "same value"]);
    base.cwd = Some(first_dir.path().to_path_buf());

    let first = run_benchmark(&base).expect("first identity run");
    let repeated = run_benchmark(&base).expect("repeated identity run");
    let mut changed_arg = base.clone();
    changed_arg.args[1] = "different value".to_owned();
    let changed_arg = run_benchmark(&changed_arg).expect("changed argv run");
    let mut changed_cwd = base.clone();
    changed_cwd.cwd = Some(second_dir.path().to_path_buf());
    let changed_cwd = run_benchmark(&changed_cwd).expect("changed cwd run");

    assert_eq!(
        first.command.identity_sha256,
        repeated.command.identity_sha256
    );
    assert_ne!(
        first.command.identity_sha256,
        changed_arg.command.identity_sha256
    );
    assert_ne!(
        first.command.identity_sha256,
        changed_cwd.command.identity_sha256
    );
}

#[test]
fn invalid_counts_timeout_cwd_and_executable_are_rejected_before_measurement() {
    let invalid_counts = [
        (0, 0, "zero measured runs"),
        (10_001, 0, "measured runs above bound"),
        (u32::MAX, 0, "absurd measured runs"),
        (1, 10_001, "warmups above bound"),
        (1, u32::MAX, "absurd warmups"),
    ];
    for (measured, warmups, name) in invalid_counts {
        let mut invalid = spec(PRINTF, &[""]);
        invalid.measured_runs = measured;
        invalid.warmup_runs = warmups;
        assert!(
            matches!(
                run_benchmark(&invalid),
                Err(BenchmarkError::InvalidMeasuredRuns(_))
                    | Err(BenchmarkError::InvalidWarmupRuns(_))
            ),
            "{name} must be rejected"
        );
    }

    for timeout_ms in [0, 86_400_001, u64::MAX] {
        let mut invalid = spec(PRINTF, &[""]);
        invalid.timeout_ms = timeout_ms;
        assert!(matches!(
            run_benchmark(&invalid),
            Err(BenchmarkError::InvalidTimeout(value)) if value == timeout_ms
        ));
    }

    let directory = TempDir::new().expect("invalid cwd fixture");
    let file_cwd = directory.path().join("not-a-directory");
    fs::write(&file_cwd, b"file").expect("write invalid cwd fixture");
    for cwd in [file_cwd, directory.path().join("directory-does-not-exist")] {
        let mut invalid = spec(PRINTF, &[""]);
        invalid.cwd = Some(cwd);
        assert!(matches!(
            run_benchmark(&invalid),
            Err(BenchmarkError::InvalidWorkingDirectory { .. })
        ));
    }

    let missing = spec("seval-executable-that-cannot-exist-8a31f6", &[]);
    assert!(matches!(
        run_benchmark(&missing),
        Err(BenchmarkError::ExecutableNotFound { program })
            if program == "seval-executable-that-cannot-exist-8a31f6"
    ));
}

#[test]
fn cli_json_preserves_observations_rates_argv_and_success_exit_code() {
    let output = run_cli(&[
        "bench",
        "--warmup",
        "1",
        "--runs",
        "3",
        "--workload-units",
        "12",
        "--workload-bytes",
        "256",
        "--format",
        "json",
        "--",
        PRINTF,
        "%s",
        "CLI argument with spaces",
    ]);
    assert!(
        output.status.success(),
        "successful benchmark CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = json_output(&output);
    assert_no_judgment_fields(&value);
    assert_eq!(value["command"]["args"][1], "CLI argument with spaces");
    assert_eq!(value["coverage"]["observed_warmup_runs"], 1);
    assert_eq!(value["coverage"]["observed_measured_runs"], 3);
    assert_eq!(value["coverage"]["warmed_distribution_denominator"], 2);
    assert_eq!(value["warmed_samples"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["warmed_latency_ns"]["sample_count"], 2);
    assert_eq!(value["warmed_units_per_second"]["sample_count"], 2);
    assert_eq!(value["warmed_bytes_per_second"]["sample_count"], 2);
    assert_eq!(
        value["first_measured_run"]["stdout"],
        Value::Array(
            b"CLI argument with spaces"
                .iter()
                .map(|byte| Value::from(*byte))
                .collect()
        )
    );
}

#[test]
fn cli_text_reports_nonzero_run_receipts_and_returns_failure_exit_code() {
    let output = run_cli(&[
        "bench", "--warmup", "1", "--runs", "3", "--format", "text", "--", FALSE,
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).expect("text report must be UTF-8");
    assert!(text.contains("coverage: warmups=1/1 measured=3/3 warmed-denominator=2"));
    assert!(text.contains("successful: false"));
    assert!(text.contains("first measured #0: success=false exit=Some(1)"));
    assert_eq!(text.matches("sample #").count(), 2);
}

#[test]
fn cli_validation_errors_use_exit_two_without_emitting_reports() {
    let directory = TempDir::new().expect("CLI invalid cwd fixture");
    let file_cwd = directory.path().join("file cwd");
    fs::write(&file_cwd, b"not a directory").expect("write cwd file");
    let file_cwd = file_cwd.to_string_lossy().into_owned();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        (
            "zero measured runs",
            vec!["bench", "--runs", "0", "--", PRINTF, ""],
        ),
        (
            "absurd measured runs",
            vec!["bench", "--runs", "10001", "--", PRINTF, ""],
        ),
        (
            "zero timeout",
            vec!["bench", "--timeout-ms", "0", "--", PRINTF, ""],
        ),
        (
            "invalid cwd",
            vec!["bench", "--cwd", &file_cwd, "--", PRINTF, ""],
        ),
        (
            "missing executable",
            vec!["bench", "--", "seval-executable-that-cannot-exist-93bc62"],
        ),
    ];

    for (name, arguments) in cases {
        let output = run_cli(&arguments);
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(output.stdout.is_empty(), "{name} emitted a partial report");
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("seval: "),
            "{name} did not surface a benchmark validation error: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

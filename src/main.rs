mod analysis_output;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use analysis_output::{
    print_api, print_benchmark, print_dependencies, print_duplicates, print_tests,
};
use clap::{Parser, Subcommand, ValueEnum};
use software_evaluation::api_surface::analyze_api_surface;
use software_evaluation::audit::{AuditReport, Severity, audit_evaluation_dir};
use software_evaluation::benchmark::{BenchmarkSpec, run_benchmark};
use software_evaluation::compare::{CompareError, EvaluationComparison, compare_evaluation_runs};
use software_evaluation::deps::analyze_dependencies;
use software_evaluation::duplicates::{DuplicateConfig, analyze_duplicates};
use software_evaluation::info::{PlanReport, PlanSpec, plan};
use software_evaluation::kernel::{
    BeliefState, CriterionProgram, DecisionSpec, EvaluationRun, ResourceBudget, StopReason,
    evaluate_pipeline,
};
use software_evaluation::metrics::{
    FileIdentity, FileMetric, FunctionMetric, MatchedFileDifference, MetricSort, MetricsComparison,
    MetricsComparisonSide, MetricsReport, NumericDifference, analyze_path, compare_paths,
    rank_files, rank_functions,
};
use software_evaluation::repo::{
    GitChangeShapeProgram, RepoProfileConfig, StaticRepoShapeProgram, snapshot_git_repo,
};
use software_evaluation::tests_analysis::analyze_tests;

#[derive(Debug, Parser)]
#[command(name = "seval", version, about = "Evidence-first software evaluation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check an evaluation bundle's records and evidence closure.
    Audit {
        /// Directory containing report.md and records.jsonl.
        evaluation_dir: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Rank and select audit probes by information and decision value.
    Plan {
        /// JSON file containing claims, probes, budgets, and strategy.
        spec: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Measure committed repository structure and bounded change topology.
    RepoProfile {
        /// Clean Git repository to profile at HEAD.
        #[arg(default_value = ".")]
        repository: PathBuf,
        /// Maximum non-merge commits included in the change-shape program.
        #[arg(long, default_value_t = 200)]
        history_commits: usize,
        /// Per-repository wall-time budget.
        #[arg(long, default_value_t = 10)]
        max_seconds: u64,
        /// Maximum criterion-program invocations.
        #[arg(long, default_value_t = 2)]
        max_programs: u32,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Compare matched repository observations without choosing a winner.
    RepoCompare {
        /// Clean Git repository used as the left snapshot.
        left: PathBuf,
        /// Clean Git repository used as the right snapshot.
        right: PathBuf,
        /// Maximum non-merge commits included per repository.
        #[arg(long, default_value_t = 200)]
        history_commits: usize,
        /// Wall-time budget applied independently to each repository.
        #[arg(long, default_value_t = 10)]
        max_seconds: u64,
        /// Criterion-program limit applied independently to each repository.
        #[arg(long, default_value_t = 2)]
        max_programs: u32,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Analyze source files and print aggregate AST metrics.
    Metrics {
        /// File or directory to analyze.
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Compare two source trees or files without assigning a winner.
    MetricsCompare {
        /// File or directory used as the left baseline.
        left: PathBuf,
        /// File or directory used as the right candidate.
        right: PathBuf,
        /// Maximum matched-file difference rows to show; zero shows none.
        #[arg(long, default_value_t = 30)]
        top_files: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Rank functions by one AST metric.
    Functions {
        /// File or directory to analyze.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Metric used to rank rows (maintainability ranks lowest first).
        #[arg(long, value_enum, default_value_t = MetricSortArg::Cognitive)]
        sort: MetricSortArg,
        /// Maximum rows to show; zero shows no rows.
        #[arg(long, default_value_t = 30)]
        top: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Rank files by one AST metric.
    Files {
        /// File or directory to analyze.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Metric used to rank rows (maintainability ranks lowest first).
        #[arg(long, value_enum, default_value_t = MetricSortArg::Cognitive)]
        sort: MetricSortArg,
        /// Maximum rows to show; zero shows no rows.
        #[arg(long, default_value_t = 30)]
        top: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Inspect imports, manifests, dependency topology, and cycles.
    #[command(visible_alias = "dependencies")]
    Deps {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Maximum table rows shown in text output.
        #[arg(long, default_value_t = 30)]
        top: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Detect structurally duplicated source after AST token normalization.
    #[command(visible_alias = "clones")]
    Duplicates {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 40)]
        min_tokens: usize,
        #[arg(long, default_value_t = 5)]
        min_lines: usize,
        #[arg(long, default_value_t = 100)]
        max_groups: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Inventory the representable public API surface.
    Api {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Maximum symbol rows shown in text output.
        #[arg(long, default_value_t = 100)]
        top: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Inventory test files, cases, ignored cases, and assertion-like calls.
    #[command(visible_alias = "test-shape")]
    Tests {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Maximum file/path rows shown in each text section.
        #[arg(long, default_value_t = 100)]
        top: usize,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Benchmark an exact argv repeatedly and emit resource receipts.
    Bench {
        /// Warmup invocations, excluded from measured distributions.
        #[arg(long, default_value_t = 1)]
        warmup: u32,
        /// Measured invocations; the first is reported separately.
        #[arg(long, default_value_t = 10)]
        runs: u32,
        /// Per-invocation timeout in milliseconds.
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
        /// Child working directory; defaults to the current directory.
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Optional fixed logical operations completed per invocation.
        #[arg(long)]
        workload_units: Option<u64>,
        /// Optional fixed bytes processed per invocation.
        #[arg(long)]
        workload_bytes: Option<u64>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Exact command argv. Separate it from seval options with `--`.
        #[arg(last = true, required = true, num_args = 1.., allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[derive(Debug, serde::Serialize)]
struct RepoComparisonReport {
    left: EvaluationRun,
    right: EvaluationRun,
    comparison: Option<EvaluationComparison>,
    comparison_error: Option<CompareError>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum MetricSortArg {
    Cognitive,
    Cyclomatic,
    Sloc,
    Arguments,
    Exits,
    Maintainability,
    HalsteadEffort,
}

impl MetricSortArg {
    fn library(self) -> MetricSort {
        match self {
            Self::Cognitive => MetricSort::Cognitive,
            Self::Cyclomatic => MetricSort::Cyclomatic,
            Self::Sloc => MetricSort::Sloc,
            Self::Arguments => MetricSort::Arguments,
            Self::Exits => MetricSort::Exits,
            Self::Maintainability => MetricSort::Maintainability,
            Self::HalsteadEffort => MetricSort::HalsteadEffort,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cognitive => "cognitive",
            Self::Cyclomatic => "cyclomatic",
            Self::Sloc => "sloc",
            Self::Arguments => "arguments",
            Self::Exits => "exits",
            Self::Maintainability => "maintainability",
            Self::HalsteadEffort => "halstead-effort",
        }
    }
}

#[derive(serde::Serialize)]
struct RankedFunctions<'a> {
    root: &'a str,
    analyzer: &'a str,
    sort: MetricSortArg,
    shown: usize,
    total: usize,
    rows: Vec<&'a FunctionMetric>,
    limitations: &'a [String],
}

#[derive(serde::Serialize)]
struct RankedFiles<'a> {
    root: &'a str,
    analyzer: &'a str,
    sort: MetricSortArg,
    shown: usize,
    total: usize,
    rows: Vec<&'a FileMetric>,
    limitations: &'a [String],
}

#[derive(serde::Serialize)]
struct MetricsComparisonOutput<'a> {
    left: &'a MetricsComparisonSide,
    right: &'a MetricsComparisonSide,
    differences: &'a [NumericDifference],
    matched_files_shown: usize,
    matched_files_total: usize,
    matched_files: Vec<&'a MatchedFileDifference>,
    only_left: &'a [FileIdentity],
    only_right: &'a [FileIdentity],
    limitations: &'a [String],
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("seval: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Audit {
            evaluation_dir,
            format,
        } => {
            let report =
                audit_evaluation_dir(&evaluation_dir).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_audit(&report),
            }
            Ok(if report.passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Command::Plan { spec, format } => {
            let input = fs::read_to_string(&spec)
                .map_err(|error| format!("failed to read {}: {error}", spec.display()))?;
            if input.trim().is_empty() {
                return Err(format!("{} is empty", spec.display()));
            }
            let spec: PlanSpec = serde_json::from_str(&input)
                .map_err(|error| format!("invalid plan spec {}: {error}", spec.display()))?;
            let report = plan(&spec).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_plan(&report),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::RepoProfile {
            repository,
            history_commits,
            max_seconds,
            max_programs,
            format,
        } => {
            let report =
                evaluate_repository(&repository, history_commits, max_seconds, max_programs)?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_repo_run(&report)?,
            }
            Ok(if report.stopped_reason == StopReason::Complete {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Command::RepoCompare {
            left,
            right,
            history_commits,
            max_seconds,
            max_programs,
            format,
        } => {
            let left = evaluate_repository(&left, history_commits, max_seconds, max_programs)?;
            let right = evaluate_repository(&right, history_commits, max_seconds, max_programs)?;
            let (comparison, comparison_error) = match compare_evaluation_runs(&left, &right) {
                Ok(comparison) => (Some(comparison), None),
                Err(error) => (None, Some(error)),
            };
            let comparable = comparison.is_some();
            let report = RepoComparisonReport {
                left,
                right,
                comparison,
                comparison_error,
            };
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_repo_comparison(&report)?,
            }
            Ok(if comparable {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Command::Metrics { path, format } => {
            let report = analyze_path(&path).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_metrics(&report),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::MetricsCompare {
            left,
            right,
            top_files,
            format,
        } => {
            let comparison = compare_paths(&left, &right).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => {
                    print_json(&metrics_comparison_output(&comparison, top_files))?
                }
                OutputFormat::Text => print_metrics_comparison(&comparison, top_files),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Functions {
            path,
            sort,
            top,
            format,
        } => {
            let report = analyze_path(&path).map_err(|error| error.to_string())?;
            let functions = rank_functions(&report, sort.library(), top);
            match format {
                OutputFormat::Json => print_json(&RankedFunctions {
                    root: &report.root,
                    analyzer: &report.analyzer,
                    sort,
                    shown: functions.len(),
                    total: report.functions.len(),
                    rows: functions,
                    limitations: &report.limitations,
                })?,
                OutputFormat::Text => print_functions(&report, sort, top),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Files {
            path,
            sort,
            top,
            format,
        } => {
            let report = analyze_path(&path).map_err(|error| error.to_string())?;
            let files = rank_files(&report, sort.library(), top);
            match format {
                OutputFormat::Json => print_json(&RankedFiles {
                    root: &report.root,
                    analyzer: &report.analyzer,
                    sort,
                    shown: files.len(),
                    total: report.files.len(),
                    rows: files,
                    limitations: &report.limitations,
                })?,
                OutputFormat::Text => print_files(&report, sort, top),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Deps { path, top, format } => {
            let report = analyze_dependencies(&path).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_dependencies(&report, top),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Duplicates {
            path,
            min_tokens,
            min_lines,
            max_groups,
            format,
        } => {
            let config = DuplicateConfig {
                min_tokens,
                min_lines,
                max_groups,
            };
            let report = analyze_duplicates(&path, &config).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_duplicates(&report),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Api { path, top, format } => {
            let report = analyze_api_surface(&path).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_api(&report, top),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Tests { path, top, format } => {
            let report = analyze_tests(&path).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_tests(&report, top),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Bench {
            warmup,
            runs,
            timeout_ms,
            cwd,
            workload_units,
            workload_bytes,
            format,
            command,
        } => {
            let (program, args) = command
                .split_first()
                .ok_or_else(|| "benchmark command argv is empty".to_owned())?;
            let report = run_benchmark(&BenchmarkSpec {
                program: program.clone(),
                args: args.to_vec(),
                cwd,
                warmup_runs: warmup,
                measured_runs: runs,
                timeout_ms,
                workload_units,
                workload_bytes,
            })
            .map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Json => print_json(&report)?,
                OutputFormat::Text => print_benchmark(&report),
            }
            Ok(if report.successful {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

fn evaluate_repository(
    repository: &std::path::Path,
    history_commits: usize,
    max_seconds: u64,
    max_programs: u32,
) -> Result<EvaluationRun, String> {
    let artifact = snapshot_git_repo(repository).map_err(|error| error.to_string())?;
    let static_shape = StaticRepoShapeProgram::new();
    let change_shape = GitChangeShapeProgram::new(RepoProfileConfig { history_commits })
        .map_err(|error| error.to_string())?;
    let programs: [&dyn CriterionProgram; 2] = [&static_shape, &change_shape];
    let evidence = BeliefState {
        probabilities: BTreeMap::new(),
        observation_digests: Vec::new(),
    };
    let decision = DecisionSpec {
        id: "repository-review-routing".to_owned(),
        description: "Decide which deeper repository evaluations to run next from deterministic shape proxies".to_owned(),
        claim_ids: vec![
            "source-size-concentration".to_owned(),
            "change-mass-concentration".to_owned(),
            "cross-boundary-cochange".to_owned(),
        ],
    };
    let budget = ResourceBudget {
        max_usd: 0.0,
        max_wall_time_ms: max_seconds
            .checked_mul(1_000)
            .ok_or_else(|| "max-seconds is too large to represent in milliseconds".to_owned())?,
        max_programs,
    };

    evaluate_pipeline(&artifact, &programs, &evidence, &decision, &budget)
        .map_err(|error| error.to_string())
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    let output = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize output: {error}"))?;
    println!("{output}");
    Ok(())
}

fn print_repo_run(report: &EvaluationRun) -> Result<(), String> {
    println!("artifact: {}", report.artifact.id);
    println!("revision: {}", report.artifact.revision);
    println!("tree: {}", report.artifact.tree_digest);
    for step in &report.steps {
        println!(
            "program: {}@{} criterion={} status={:?} elapsed={}ms bytes_read={}",
            step.receipt.program.id,
            step.receipt.program.version,
            step.receipt.program.criterion,
            step.receipt.status,
            step.receipt.elapsed_ms,
            step.receipt.actual_resources.bytes_read,
        );
        if let Some(message) = &step.receipt.message {
            println!("  receipt: {message}");
        }
        for evidence in &step.evidence {
            println!(
                "  evidence: {} digest={} locator={}",
                evidence.kind,
                evidence.digest.as_deref().unwrap_or("none"),
                evidence.locator,
            );
        }
        if let Some(observation) = &step.observation {
            let rendered = serde_json::to_string_pretty(observation)
                .map_err(|error| format!("failed to serialize observation: {error}"))?;
            println!("  observation:\n{rendered}");
        }
    }
    println!("stop: {:?}", report.stopped_reason);
    println!(
        "remaining: ${:.6} {}ms {} programs",
        report.remaining.usd, report.remaining.wall_time_ms, report.remaining.programs
    );
    Ok(())
}

fn print_repo_comparison(report: &RepoComparisonReport) -> Result<(), String> {
    println!("left:");
    print_repo_run(&report.left)?;
    println!("right:");
    print_repo_run(&report.right)?;
    if let Some(comparison) = &report.comparison {
        println!("matched numeric differences (right - left; no quality direction):");
        for program in &comparison.programs {
            println!(
                "program: {}@{} criterion={}",
                program.program_id, program.program_version, program.criterion
            );
            for difference in &program.differences {
                let relative = difference
                    .relative_change_from_left
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned());
                println!(
                    "  {}: left={} right={} delta={} relative={}",
                    difference.path,
                    difference.left,
                    difference.right,
                    difference.right_minus_left,
                    relative,
                );
            }
        }
        for limitation in &comparison.limitations {
            println!("limitation: {limitation}");
        }
    } else if let Some(error) = &report.comparison_error {
        println!("comparison unavailable: {error}");
    }
    Ok(())
}

fn print_audit(report: &AuditReport) {
    println!("evaluation: {}", report.evaluation_dir);
    println!("records: {}", report.records_total);
    if report.instrument_counts.is_empty() {
        println!("instruments: none");
    } else {
        let counts = report
            .instrument_counts
            .iter()
            .map(|(instrument, count)| format!("{instrument}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("instruments: {counts}");
    }
    println!("referenced records: {}", report.referenced_record_ids.len());
    println!("result: {}", if report.passed() { "PASS" } else { "FAIL" });

    for issue in &report.issues {
        let severity = match issue.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN",
        };
        let location = match (&issue.path, issue.line) {
            (Some(path), Some(line)) => format!("{path}:{line}"),
            (Some(path), None) => path.clone(),
            (None, Some(line)) => format!("line {line}"),
            (None, None) => "evaluation".to_owned(),
        };
        println!("{severity} [{}] {location}: {}", issue.code, issue.message);
    }
}

fn print_plan(report: &PlanReport) {
    println!(
        "single-probe frontier: {}",
        report.single_probe_frontier.join(", ")
    );
    println!("selected probes: {}", report.selected.len());
    for step in &report.selected {
        println!(
            "{}. {} claim={} marginal={:.6} bits risk_delta={:.6} cumulative={:.6} bits ${:.6} {:.3}s",
            step.ordinal,
            step.probe_id,
            step.claim_id,
            step.marginal_information_bits,
            step.marginal_risk_reduction,
            step.cumulative_information_bits,
            step.cumulative_usd,
            step.cumulative_seconds,
        );
    }
    println!(
        "totals: {:.6} bits risk_delta={:.6} ${:.6} {:.3}s",
        report.total_information_bits,
        report.total_expected_risk_reduction,
        report.total_usd,
        report.total_seconds,
    );
    println!("stop: {}", report.stopped_reason);
    for assumption in &report.assumptions {
        println!("assumption: {assumption}");
    }
}

fn optional(value: Option<f64>) -> String {
    value
        .map(|number| format!("{number:.2}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn print_metrics(report: &MetricsReport) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "coverage: {} analyzed / {} enumerated, {} skipped ({}ms)",
        report.coverage.analyzed,
        report.coverage.enumerated,
        report.coverage.skipped,
        report.coverage.elapsed_ms
    );
    println!(
        "totals: {} files, {} functions, {} lines, {} SLOC, {} PLOC, {} LLOC, {} CLOC, {} blank",
        report.summary.files,
        report.summary.functions,
        report.summary.lines,
        report.summary.sloc,
        report.summary.ploc,
        report.summary.lloc,
        report.summary.cloc,
        report.summary.blank
    );
    println!(
        "complexity: cognitive={:.2} cyclomatic={:.2} modified={:.2} arguments={} exits={}",
        report.summary.cognitive,
        report.summary.cyclomatic,
        report.summary.modified_cyclomatic,
        report.summary.arguments,
        report.summary.exits
    );
    println!(
        "file means: maintainability={} halstead-volume={} halstead-difficulty={} halstead-effort={} | aggregate ABC={:.2}/{:.2}/{:.2} magnitude={:.2}",
        optional(report.summary.mean_maintainability),
        optional(report.summary.mean_halstead_volume),
        optional(report.summary.mean_halstead_difficulty),
        optional(report.summary.mean_halstead_effort),
        report.summary.abc_assignments,
        report.summary.abc_branches,
        report.summary.abc_conditions,
        report.summary.abc_magnitude
    );
    println!(
        "rates: functions/kSLOC={} cognitive/kSLOC={} cyclomatic/kSLOC={} arguments/function={} exits/function={} comments={} blanks={}",
        optional(report.rates.functions_per_ksloc),
        optional(report.rates.cognitive_per_ksloc),
        optional(report.rates.cyclomatic_per_ksloc),
        optional(report.rates.arguments_per_function),
        optional(report.rates.exits_per_function),
        optional(report.rates.comment_fraction),
        optional(report.rates.blank_fraction),
    );
    println!("function distributions (nearest-rank):");
    println!(
        "  {:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "METRIC", "MEAN", "P50", "P90", "P99", "MAX"
    );
    for (name, distribution) in [
        ("cognitive", &report.distributions.cognitive),
        ("cyclomatic", &report.distributions.cyclomatic),
        ("sloc", &report.distributions.sloc),
        ("arguments", &report.distributions.arguments),
        ("exits", &report.distributions.exits),
    ] {
        println!(
            "  {:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
            name,
            optional(distribution.mean),
            optional(distribution.p50),
            optional(distribution.p90),
            optional(distribution.p99),
            optional(distribution.max),
        );
    }
    println!("languages:");
    println!(
        "  {:<16} {:>7} {:>9} {:>9} {:>9} {:>11} {:>11}",
        "LANGUAGE", "FILES", "FUNCTIONS", "LINES", "SLOC", "COGNITIVE", "CYCLOMATIC"
    );
    for language in &report.languages {
        println!(
            "  {:<16} {:>7} {:>9} {:>9} {:>9} {:>11.2} {:>11.2}",
            language.language,
            language.files,
            language.functions,
            language.lines,
            language.sloc,
            language.cognitive,
            language.cyclomatic
        );
    }
    println!("top cognitive functions:");
    print_function_rows(&rank_functions(report, MetricSort::Cognitive, 10));
    print_limitations(&report.limitations);
}

fn metrics_comparison_output(
    comparison: &MetricsComparison,
    top_files: usize,
) -> MetricsComparisonOutput<'_> {
    let matched_files = comparison
        .matched_files
        .iter()
        .take(top_files)
        .collect::<Vec<_>>();
    MetricsComparisonOutput {
        left: &comparison.left,
        right: &comparison.right,
        differences: &comparison.differences,
        matched_files_shown: matched_files.len(),
        matched_files_total: comparison.matched_files.len(),
        matched_files,
        only_left: &comparison.only_left,
        only_right: &comparison.only_right,
        limitations: &comparison.limitations,
    }
}

fn print_metrics_comparison(comparison: &MetricsComparison, top_files: usize) {
    println!("left: {}", comparison.left.root);
    println!("right: {}", comparison.right.root);
    println!(
        "coverage: left={}/{} right={}/{} analyzed/enumerated",
        comparison.left.coverage.analyzed,
        comparison.left.coverage.enumerated,
        comparison.right.coverage.analyzed,
        comparison.right.coverage.enumerated,
    );
    println!("differences (right - left; no quality direction):");
    println!(
        "  {:<32} {:>14} {:>14} {:>14} {:>12}",
        "METRIC", "LEFT", "RIGHT", "DELTA", "RELATIVE"
    );
    for difference in &comparison.differences {
        let relative = difference
            .relative_change_from_left
            .map(|value| format!("{:+.2}%", value * 100.0))
            .unwrap_or_else(|| "n/a".to_owned());
        println!(
            "  {:<32} {:>14.4} {:>14.4} {:>+14.4} {:>12}",
            difference.metric,
            difference.left,
            difference.right,
            difference.right_minus_left,
            relative,
        );
    }
    let shown = comparison.matched_files.len().min(top_files);
    println!(
        "matched file deltas: {} / {} shown; only-left={} only-right={}",
        shown,
        comparison.matched_files.len(),
        comparison.only_left.len(),
        comparison.only_right.len(),
    );
    println!(
        "  {:<48} {:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "PATH", "LANGUAGE", "SLOC", "COG", "CYC", "ARGS", "EXITS"
    );
    for file in comparison.matched_files.iter().take(top_files) {
        println!(
            "  {:<48} {:<12} {:>+8} {:>+8.2} {:>+8.2} {:>+8} {:>+8}",
            file.path,
            file.language,
            file.right_minus_left.sloc,
            file.right_minus_left.cognitive,
            file.right_minus_left.cyclomatic,
            file.right_minus_left.arguments,
            file.right_minus_left.exits,
        );
    }
    print_file_identities("only left", &comparison.only_left, 10);
    print_file_identities("only right", &comparison.only_right, 10);
    print_limitations(&comparison.limitations);
}

fn print_file_identities(label: &str, files: &[FileIdentity], top: usize) {
    if files.is_empty() {
        return;
    }
    let shown = files.len().min(top);
    println!("{label}: {shown} / {} shown", files.len());
    for file in files.iter().take(top) {
        println!("  {} ({})", file.path, file.language);
    }
}

fn print_functions(report: &MetricsReport, sort: MetricSortArg, top: usize) {
    let rows = rank_functions(report, sort.library(), top);
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!("sort: {}", sort.name());
    println!(
        "shown: {} / {} functions",
        rows.len(),
        report.functions.len()
    );
    print_function_rows(&rows);
    print_limitations(&report.limitations);
}

fn print_function_rows(rows: &[&FunctionMetric]) {
    let location_width = rows
        .iter()
        .map(|row| row.path.len() + 1 + row.start_line.to_string().len())
        .max()
        .unwrap_or(8)
        .max("LOCATION".len());
    let name_width = rows
        .iter()
        .map(|row| row.name.len())
        .max()
        .unwrap_or(8)
        .max("FUNCTION".len());
    println!(
        "{:<location_width$}  {:<name_width$}  {:<12} {:>5} {:>5} {:>7} {:>7} {:>7} {:>5} {:>5} {:>8} {:>12} {:>8}",
        "LOCATION",
        "FUNCTION",
        "LANGUAGE",
        "LINES",
        "SLOC",
        "COG",
        "CYC",
        "MOD",
        "ARGS",
        "EXITS",
        "MI",
        "H-EFFORT",
        "ABC"
    );
    for row in rows {
        let location = format!("{}:{}", row.path, row.start_line);
        println!(
            "{location:<location_width$}  {:<name_width$}  {:<12} {:>5} {:>5} {:>7.2} {:>7.2} {:>7.2} {:>5} {:>5} {:>8} {:>12} {:>8.2}",
            row.name,
            row.language,
            row.lines,
            row.sloc,
            row.cognitive,
            row.cyclomatic,
            row.modified_cyclomatic,
            row.arguments,
            row.exits,
            optional(row.maintainability),
            optional(row.halstead_effort),
            row.abc_magnitude
        );
    }
}

fn print_files(report: &MetricsReport, sort: MetricSortArg, top: usize) {
    let rows = rank_files(report, sort.library(), top);
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!("sort: {}", sort.name());
    println!("shown: {} / {} files", rows.len(), report.files.len());
    let path_width = rows
        .iter()
        .map(|row| row.path.len())
        .max()
        .unwrap_or(4)
        .max("PATH".len());
    println!(
        "{:<path_width$}  {:<12} {:>5} {:>5} {:>5} {:>7} {:>7} {:>7} {:>5} {:>5} {:>8} {:>12} {:>8}",
        "PATH",
        "LANGUAGE",
        "FUNCS",
        "LINES",
        "SLOC",
        "COG",
        "CYC",
        "MOD",
        "ARGS",
        "EXITS",
        "MI",
        "H-EFFORT",
        "ABC"
    );
    for row in rows {
        println!(
            "{:<path_width$}  {:<12} {:>5} {:>5} {:>5} {:>7.2} {:>7.2} {:>7.2} {:>5} {:>5} {:>8} {:>12} {:>8.2}",
            row.path,
            row.language,
            row.functions,
            row.lines,
            row.sloc,
            row.cognitive,
            row.cyclomatic,
            row.modified_cyclomatic,
            row.arguments,
            row.exits,
            optional(row.maintainability),
            optional(row.halstead_effort),
            row.abc_magnitude
        );
    }
    print_limitations(&report.limitations);
}

fn print_limitations(limitations: &[String]) {
    if limitations.is_empty() {
        println!("limitations: none");
    } else {
        println!("limitations:");
        for limitation in limitations {
            println!("  - {limitation}");
        }
    }
}

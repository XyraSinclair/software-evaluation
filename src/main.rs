use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use software_evaluation::audit::{AuditReport, Severity, audit_evaluation_dir};
use software_evaluation::compare::{CompareError, EvaluationComparison, compare_evaluation_runs};
use software_evaluation::info::{PlanReport, PlanSpec, plan};
use software_evaluation::kernel::{
    BeliefState, CriterionProgram, DecisionSpec, EvaluationRun, ResourceBudget, StopReason,
    evaluate_pipeline,
};
use software_evaluation::repo::{
    GitChangeShapeProgram, RepoProfileConfig, StaticRepoShapeProgram, snapshot_git_repo,
};

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

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use software_evaluation::frontier::{
    AnalyzerStatus, FrontierComparison, FrontierConfig, FrontierProfile, PartialOrder,
    SignalOutcome, SignalStatus, compare_paths, profile_path,
};

#[derive(Debug, Parser)]
#[command(
    name = "seval-frontier",
    version,
    about = "Receipt-bearing Pareto surface for fast software-quality routing"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Project one source tree onto the six-signal mechanical frontier.
    Profile {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[command(flatten)]
        analysis: AnalysisArgs,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Compare two trees by strict Pareto order; never count favorable axes.
    Compare {
        left: PathBuf,
        right: PathBuf,
        #[command(flatten)]
        analysis: AnalysisArgs,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Args)]
struct AnalysisArgs {
    #[arg(long, default_value_t = 40)]
    duplicate_min_tokens: usize,
    #[arg(long, default_value_t = 5)]
    duplicate_min_lines: usize,
    #[arg(long, default_value_t = 100)]
    duplicate_max_groups: usize,
    #[arg(long, default_value_t = 0.50)]
    min_symbol_resolution_fraction: f64,
}

impl AnalysisArgs {
    fn config(&self) -> FrontierConfig {
        FrontierConfig {
            duplicate_min_tokens: self.duplicate_min_tokens,
            duplicate_min_lines: self.duplicate_min_lines,
            duplicate_max_groups: self.duplicate_max_groups,
            min_symbol_resolution_fraction: self.min_symbol_resolution_fraction,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("seval-frontier: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Profile {
            path,
            analysis,
            format,
        } => {
            let report =
                profile_path(&path, &analysis.config()).map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Text => print_profile(&report),
                OutputFormat::Json => print_json(&report)?,
            }
            Ok(if report.coverage.observed == 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Compare {
            left,
            right,
            analysis,
            format,
        } => {
            let report = compare_paths(&left, &right, &analysis.config())
                .map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Text => print_comparison(&report),
                OutputFormat::Json => print_json(&report)?,
            }
            Ok(if report.qualified_order.is_some() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value)
            .map_err(|error| format!("failed to serialize output: {error}"))?
    );
    Ok(())
}

fn print_profile(report: &FrontierProfile) {
    println!("artifact: {}", artifact_label(report));
    println!(
        "directional coverage: {}/{}",
        report.coverage.observed, report.coverage.declared
    );
    if let Some(error) = &report.artifact.identity_error {
        println!("identity: unpinned ({error})");
    }
    for family in &report.families {
        println!("family: {} — {}", family.label, family.rule);
        for signal in report
            .signals
            .iter()
            .filter(|signal| signal.family == family.id)
        {
            println!(
                "  {}: {} {} [{}; {}]",
                signal.id,
                render_number(signal.value),
                signal.unit,
                signal_status(signal.status),
                polarity(signal.polarity),
            );
            if let Some(reason) = &signal.unavailable_reason {
                println!("    unavailable: {reason}");
            }
        }
    }
    for analyzer in &report.analyzers {
        println!(
            "receipt: {} status={} elapsed={}ms digest={}",
            analyzer.id,
            analyzer_status(analyzer.status),
            analyzer.elapsed_ms,
            analyzer.payload_sha256.as_deref().unwrap_or("none"),
        );
        if let Some(error) = &analyzer.error {
            println!("  error: {error}");
        }
    }
    println!("elapsed: {}ms", report.elapsed_ms);
}

fn print_comparison(report: &FrontierComparison) {
    println!("left: {}", artifact_label(&report.left));
    println!("right: {}", artifact_label(&report.right));
    println!(
        "order on observed intersection: {}",
        partial_order(report.order_on_observed_intersection)
    );
    match report.qualified_order {
        Some(order) => println!("qualified order: {}", partial_order(order)),
        None => println!("qualified order: unavailable"),
    }
    println!(
        "readiness: schema={} config={} analyzers={} signals={} pinned={} qualified={}",
        report.readiness.schema_compatible,
        report.readiness.analysis_config_compatible,
        report.readiness.analyzer_implementations_compatible,
        report.readiness.directional_signals_complete,
        report.readiness.artifacts_commit_pinned,
        report.readiness.qualified,
    );
    for blocker in &report.readiness.blockers {
        println!("blocker: {blocker}");
    }
    for family in &report.families {
        println!(
            "family: {} complete={} order={}",
            family.label,
            family.comparison.complete,
            partial_order(family.comparison.order_on_observed_intersection),
        );
    }
    for signal in &report.signals {
        println!(
            "signal: {} left={} right={} delta={} outcome={}",
            signal.id,
            render_number(signal.left_value),
            render_number(signal.right_value),
            render_number(signal.right_minus_left),
            signal_outcome(signal.outcome),
        );
        if let Some(reason) = &signal.reason {
            println!("  reason: {reason}");
        }
    }
}

fn artifact_label(report: &FrontierProfile) -> String {
    report.artifact.git.as_ref().map_or_else(
        || report.artifact.input.clone(),
        |artifact| format!("{} tree={}", artifact.id, artifact.tree_digest),
    )
}

fn render_number(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn analyzer_status(status: AnalyzerStatus) -> &'static str {
    match status {
        AnalyzerStatus::Complete => "complete",
        AnalyzerStatus::Failed => "failed",
        AnalyzerStatus::Panicked => "panicked",
    }
}

fn signal_status(status: SignalStatus) -> &'static str {
    match status {
        SignalStatus::Observed => "observed",
        SignalStatus::Missing => "missing",
        SignalStatus::Censored => "censored",
        SignalStatus::InsufficientCoverage => "insufficient-coverage",
        SignalStatus::SourceFailed => "source-failed",
    }
}

fn polarity(polarity: software_evaluation::frontier::SignalPolarity) -> &'static str {
    match polarity {
        software_evaluation::frontier::SignalPolarity::LowerIsBetter => "lower-is-better",
        software_evaluation::frontier::SignalPolarity::HigherIsBetter => "higher-is-better",
    }
}

fn signal_outcome(outcome: SignalOutcome) -> &'static str {
    match outcome {
        SignalOutcome::RightBetter => "right-better",
        SignalOutcome::LeftBetter => "left-better",
        SignalOutcome::Equivalent => "equivalent",
        SignalOutcome::Unavailable => "unavailable",
        SignalOutcome::Incompatible => "incompatible",
    }
}

fn partial_order(order: PartialOrder) -> &'static str {
    match order {
        PartialOrder::RightDominates => "right-dominates",
        PartialOrder::LeftDominates => "left-dominates",
        PartialOrder::Tradeoff => "tradeoff",
        PartialOrder::Equivalent => "equivalent",
        PartialOrder::NoComparableSignals => "no-comparable-signals",
    }
}

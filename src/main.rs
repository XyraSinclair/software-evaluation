use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use software_evaluation::audit::{AuditReport, Severity, audit_evaluation_dir};
use software_evaluation::info::{PlanReport, PlanSpec, plan};

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
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    let output = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize output: {error}"))?;
    println!("{output}");
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

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use software_evaluation::frontier_probes::{ProbeAnalysis, ProbeModel, analyze_model};

#[derive(Debug, Parser)]
#[command(
    name = "seval-order-probes",
    version,
    about = "Blackwell and order-information frontiers over candidate quality probes"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze a declared world/probe model and emit both probe frontiers.
    Analyze {
        /// JSON file declaring worlds, optional priors, and probes.
        model: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyze { model, format } => {
            let raw = match fs::read_to_string(&model) {
                Ok(raw) => raw,
                Err(error) => {
                    eprintln!("cannot read {}: {error}", model.display());
                    return ExitCode::from(2);
                }
            };
            let parsed: ProbeModel = match serde_json::from_str(&raw) {
                Ok(parsed) => parsed,
                Err(error) => {
                    eprintln!("cannot parse {}: {error}", model.display());
                    return ExitCode::from(2);
                }
            };
            let analysis = match analyze_model(&parsed) {
                Ok(analysis) => analysis,
                Err(error) => {
                    eprintln!("invalid probe model: {error}");
                    return ExitCode::from(2);
                }
            };
            match format {
                OutputFormat::Json => match serde_json::to_string_pretty(&analysis) {
                    Ok(rendered) => println!("{rendered}"),
                    Err(error) => {
                        eprintln!("cannot serialize analysis: {error}");
                        return ExitCode::from(2);
                    }
                },
                OutputFormat::Text => render_text(&analysis),
            }
            ExitCode::SUCCESS
        }
    }
}

fn render_text(analysis: &ProbeAnalysis) {
    println!("worlds: {}", analysis.worlds.len());
    println!(
        "distinct induced orders: {}",
        analysis.distinct_orders.len()
    );
    if let Some(prior) = &analysis.prior {
        println!("prior: explicit (raw mass {})", prior.raw_mass);
    } else {
        println!("prior: none (worst-case analysis only)");
    }
    println!();
    for probe in &analysis.probes {
        println!("probe {}", probe.name);
        println!(
            "  cost: ${} / {} ms / {} invocations",
            probe.cost.dollars, probe.cost.latency_ms, probe.cost.invocations
        );
        println!(
            "  partition: {} block(s), sha256 {}",
            probe.partition.len(),
            &probe.partition_sha256[..16]
        );
        println!(
            "  worst-case remaining orders: {} (guaranteed {} eliminated, {:.3} bits)",
            probe.worst_case_remaining_orders,
            probe.guaranteed_eliminated_orders,
            probe.guaranteed_order_bits
        );
        if let (Some(expected), Some(information)) = (
            probe.expected_remaining_orders,
            probe.order_outcome_mutual_information_bits,
        ) {
            println!(
                "  expected remaining orders: {expected:.4}; I(order; outcome) = {information:.4} bits"
            );
        }
    }
    println!();
    println!(
        "Blackwell-cost frontier: {}",
        analysis.blackwell_frontier.join(", ")
    );
    for record in &analysis.blackwell_dominance {
        println!(
            "  dominated: {} by {} ({})",
            record.dominated, record.by, record.reason
        );
    }
    println!(
        "order-information frontier: {}",
        analysis.order_information_frontier.join(", ")
    );
    for record in &analysis.order_dominance {
        println!(
            "  dominated: {} by {} ({})",
            record.dominated, record.by, record.reason
        );
    }
}

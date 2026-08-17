use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use software_evaluation::frontier_policies::{
    PolicyAnalysis, PolicyTree, SolverLimits, solve_model,
};
use software_evaluation::frontier_probes::{CostVector, ProbeAnalysis, ProbeModel, analyze_model};

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
    /// Solve the exact robust adaptive policy frontier for a model.
    Plan {
        /// JSON file declaring worlds, optional priors, and probes.
        model: PathBuf,
        /// Hard per-path dollar budget; all three budget flags must be
        /// given together to declare a budget.
        #[arg(long, requires_all = ["budget_latency_ms", "budget_invocations"])]
        budget_dollars: Option<f64>,
        #[arg(long, requires_all = ["budget_dollars", "budget_invocations"])]
        budget_latency_ms: Option<f64>,
        #[arg(long, requires_all = ["budget_dollars", "budget_latency_ms"])]
        budget_invocations: Option<f64>,
        #[arg(long, default_value_t = SolverLimits::default().max_memo_states)]
        max_memo_states: usize,
        #[arg(long, default_value_t = SolverLimits::default().max_frontier_width)]
        max_frontier_width: usize,
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
        Command::Plan {
            model,
            budget_dollars,
            budget_latency_ms,
            budget_invocations,
            max_memo_states,
            max_frontier_width,
            format,
        } => {
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
            let budget = match (budget_dollars, budget_latency_ms, budget_invocations) {
                (Some(dollars), Some(latency_ms), Some(invocations)) => Some(CostVector {
                    dollars,
                    latency_ms,
                    invocations,
                }),
                _ => None,
            };
            let limits = SolverLimits {
                max_memo_states,
                max_frontier_width,
                ..SolverLimits::default()
            };
            let analysis = match solve_model(&parsed, budget, limits) {
                Ok(analysis) => analysis,
                Err(error) => {
                    eprintln!("cannot solve policy frontier: {error}");
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
                OutputFormat::Text => render_policy_text(&analysis),
            }
            ExitCode::SUCCESS
        }
    }
}

fn render_policy_text(analysis: &PolicyAnalysis) {
    println!(
        "initial possible orders: {}",
        analysis.initial_possible_orders.len()
    );
    if let Some(budget) = analysis.budget {
        println!(
            "hard per-path budget: ${} / {} ms / {} invocations",
            budget.dollars, budget.latency_ms, budget.invocations
        );
    } else {
        println!("hard per-path budget: none");
    }
    println!(
        "memoized states: {}; pruning certificates: {}",
        analysis.memoized_states,
        analysis.pruning_certificates.len()
    );
    println!(
        "policy frontier ({} policies):",
        analysis.policy_frontier.len()
    );
    for record in &analysis.policy_frontier {
        println!(
            "  worst-case orders {} / probes {} / ${} / {} ms / {} invocations",
            record.signature.worst_case_remaining_orders,
            record.signature.worst_case_additional_probes,
            record.signature.worst_case_cost.dollars,
            record.signature.worst_case_cost.latency_ms,
            record.signature.worst_case_cost.invocations,
        );
        render_policy_tree(&record.tree, 2);
    }
}

fn render_policy_tree(tree: &PolicyTree, indent: usize) {
    let pad = " ".repeat(indent);
    match tree {
        PolicyTree::Stop {
            status,
            possible_orders,
            ..
        } => {
            println!(
                "{pad}stop ({status:?}; {} order(s) possible)",
                possible_orders.len()
            );
        }
        PolicyTree::Probe { probe, outcomes } => {
            println!("{pad}probe {probe}");
            for (outcome, child) in outcomes {
                println!("{pad}  on {outcome}:");
                render_policy_tree(child, indent + 4);
            }
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

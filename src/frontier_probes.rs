//! Nonscalar comparison of candidate evidence-acquisition probes.
//!
//! A probe model declares a finite set of possible latent worlds, the Pareto
//! [`PartialOrder`] each world induces on the quality frontier, and a set of
//! deterministic candidate probes, each mapping every world to an outcome
//! label at a componentwise cost. This module ranks nothing. It computes two
//! different nondominated sets:
//!
//! - The **Blackwell-cost frontier** removes a probe only when another probe's
//!   outcome partition refines its partition (for deterministic finite
//!   experiments, partition refinement is exactly the prior-independent
//!   Blackwell "more informative than" order: the coarser experiment's result
//!   is a garbling of the finer one's) while costing no more in every declared
//!   cost dimension, with at least one strict advantage.
//! - The **order-information frontier** first quotients away distinctions
//!   between worlds that induce the same Pareto order, then compares
//!   worst-case remaining-order counts, optional prior-backed expected
//!   information, and componentwise cost. A probe that only separates worlds
//!   with identical induced orders earns no order information here, however
//!   fine its partition.
//!
//! No exchange rate between information and cost is ever introduced, and no
//! probe score is produced. Unresolved tradeoffs survive on both frontiers.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::frontier::PartialOrder;

pub const PROBE_SCHEMA_VERSION: &str = "seval.order-probes.v1";

/// Declared input model: worlds, optional priors, and candidate probes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeModel {
    pub worlds: Vec<WorldSpec>,
    /// Optional prior mass per world. Either every world receives a prior or
    /// none does; a partial prior is rejected rather than filled in.
    #[serde(default)]
    pub priors: Option<BTreeMap<String, f64>>,
    pub probes: Vec<ProbeSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpec {
    pub name: String,
    /// The Pareto order this latent world induces on the quality frontier.
    pub order: PartialOrder,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeSpec {
    pub name: String,
    #[serde(default)]
    pub cost: CostVector,
    /// Total observation function: every declared world must map to exactly
    /// one outcome label. Partial mappings are rejected.
    pub observations: BTreeMap<String, String>,
}

/// Componentwise cost. There is deliberately no combined magnitude.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CostVector {
    pub dollars: f64,
    pub latency_ms: f64,
    pub invocations: f64,
}

impl CostVector {
    fn components(self) -> [(&'static str, f64); 3] {
        [
            ("dollars", self.dollars),
            ("latency_ms", self.latency_ms),
            ("invocations", self.invocations),
        ]
    }

    /// Weakly no more expensive in every component.
    fn leq(self, other: Self) -> bool {
        self.dollars <= other.dollars
            && self.latency_ms <= other.latency_ms
            && self.invocations <= other.invocations
    }

    /// Strictly cheaper in at least one component.
    fn strictly_cheaper_somewhere(self, other: Self) -> bool {
        self.dollars < other.dollars
            || self.latency_ms < other.latency_ms
            || self.invocations < other.invocations
    }
}

#[derive(Debug, Error)]
pub enum ProbeModelError {
    #[error("the model declares no worlds")]
    EmptyWorlds,
    #[error("the model declares no probes")]
    EmptyProbes,
    #[error("duplicate world name: {0}")]
    DuplicateWorld(String),
    #[error("duplicate probe name: {0}")]
    DuplicateProbe(String),
    #[error("probe {probe} does not observe world {world}; observation functions must be total")]
    IncompleteObservations { probe: String, world: String },
    #[error("probe {probe} observes undeclared world {world}")]
    UnknownObservedWorld { probe: String, world: String },
    #[error(
        "priors are partial: world {world} has no prior; supply a prior for every world or none"
    )]
    PartialPriors { world: String },
    #[error("prior names undeclared world {world}")]
    UnknownPriorWorld { world: String },
    #[error("prior for world {world} is not a finite nonnegative number: {value}")]
    InvalidPrior { world: String, value: f64 },
    #[error("total prior mass is zero; at least one world needs positive mass")]
    ZeroPriorMass,
    #[error("probe {probe} cost component {component} is not a finite nonnegative number: {value}")]
    InvalidCost {
        probe: String,
        component: &'static str,
        value: f64,
    },
}

/// Normalization evidence for an explicitly supplied prior.
#[derive(Debug, Clone, Serialize)]
pub struct PriorReceipt {
    pub raw_mass: f64,
    /// Normalized weights in world order; they sum to one by construction.
    pub weights: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldReport {
    pub name: String,
    pub order: PartialOrder,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeReport {
    pub label: String,
    /// Worlds producing this outcome, sorted.
    pub worlds: Vec<String>,
    /// Distinct Pareto orders still possible after observing this outcome.
    pub possible_orders: Vec<PartialOrder>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    pub name: String,
    pub cost: CostVector,
    /// Outcome-preimage partition of the worlds: sorted blocks of sorted
    /// names. Outcome labels are deliberately excluded so relabeled but
    /// observationally identical probes are recognized as equivalent.
    pub partition: Vec<Vec<String>>,
    /// SHA-256 over the length-delimited canonical partition.
    pub partition_sha256: String,
    pub outcomes: Vec<OutcomeReport>,
    /// max over outcomes of the number of Pareto orders still possible.
    pub worst_case_remaining_orders: usize,
    /// Orders ruled out even in the least informative outcome.
    pub guaranteed_eliminated_orders: usize,
    /// Hartley information about the order guaranteed in the worst case:
    /// log2(total distinct orders) - log2(worst-case remaining orders).
    pub guaranteed_order_bits: f64,
    /// Sum over outcomes of P(outcome) * |possible orders|; requires a prior.
    pub expected_remaining_orders: Option<f64>,
    /// I(Order; Outcome) in bits under the normalized prior.
    pub order_outcome_mutual_information_bits: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DominanceRecord {
    pub dominated: String,
    pub by: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeAnalysis {
    pub schema_version: String,
    pub worlds: Vec<WorldReport>,
    /// Distinct induced Pareto orders across all declared worlds, sorted.
    pub distinct_orders: Vec<PartialOrder>,
    pub prior: Option<PriorReceipt>,
    pub probes: Vec<ProbeReport>,
    /// Probes not Blackwell-cost dominated, in declaration order.
    pub blackwell_frontier: Vec<String>,
    pub blackwell_dominance: Vec<DominanceRecord>,
    /// Probes not order-information dominated, in declaration order.
    pub order_information_frontier: Vec<String>,
    pub order_dominance: Vec<DominanceRecord>,
}

/// Validate a declared model and compute both probe frontiers.
pub fn analyze_model(model: &ProbeModel) -> Result<ProbeAnalysis, ProbeModelError> {
    let orders = validate_worlds(&model.worlds)?;
    let weights = validate_priors(model, &orders)?;
    validate_probes(model, &orders)?;

    let distinct_orders: BTreeSet<PartialOrder> = orders.values().copied().collect();
    let total_orders = distinct_orders.len();

    let mut probes = Vec::with_capacity(model.probes.len());
    let mut partitions: Vec<Vec<BTreeSet<String>>> = Vec::with_capacity(model.probes.len());
    for probe in &model.probes {
        let (report, partition) =
            analyze_probe(probe, &orders, total_orders, weights.as_deref(), model);
        probes.push(report);
        partitions.push(partition);
    }

    let (blackwell_frontier, blackwell_dominance) = blackwell_frontier(&probes, &partitions);
    let (order_information_frontier, order_dominance) =
        order_information_frontier(&probes, weights.is_some());

    Ok(ProbeAnalysis {
        schema_version: PROBE_SCHEMA_VERSION.to_owned(),
        worlds: model
            .worlds
            .iter()
            .map(|world| WorldReport {
                name: world.name.clone(),
                order: world.order,
            })
            .collect(),
        distinct_orders: distinct_orders.into_iter().collect(),
        prior: weights.as_ref().map(|weights| PriorReceipt {
            raw_mass: raw_prior_mass(model),
            weights: weights.clone(),
        }),
        probes,
        blackwell_frontier,
        blackwell_dominance,
        order_information_frontier,
        order_dominance,
    })
}

fn validate_worlds(
    worlds: &[WorldSpec],
) -> Result<BTreeMap<String, PartialOrder>, ProbeModelError> {
    if worlds.is_empty() {
        return Err(ProbeModelError::EmptyWorlds);
    }
    let mut orders = BTreeMap::new();
    for world in worlds {
        if orders.insert(world.name.clone(), world.order).is_some() {
            return Err(ProbeModelError::DuplicateWorld(world.name.clone()));
        }
    }
    Ok(orders)
}

fn validate_priors(
    model: &ProbeModel,
    orders: &BTreeMap<String, PartialOrder>,
) -> Result<Option<Vec<f64>>, ProbeModelError> {
    let Some(priors) = &model.priors else {
        return Ok(None);
    };
    for world in priors.keys() {
        if !orders.contains_key(world) {
            return Err(ProbeModelError::UnknownPriorWorld {
                world: world.clone(),
            });
        }
    }
    for world in orders.keys() {
        if !priors.contains_key(world) {
            return Err(ProbeModelError::PartialPriors {
                world: world.clone(),
            });
        }
    }
    let mut mass = 0.0_f64;
    for (world, value) in priors {
        if !value.is_finite() || *value < 0.0 {
            return Err(ProbeModelError::InvalidPrior {
                world: world.clone(),
                value: *value,
            });
        }
        mass += *value;
    }
    if mass <= 0.0 {
        return Err(ProbeModelError::ZeroPriorMass);
    }
    Ok(Some(
        model
            .worlds
            .iter()
            .map(|world| priors[&world.name] / mass)
            .collect(),
    ))
}

fn raw_prior_mass(model: &ProbeModel) -> f64 {
    model
        .priors
        .as_ref()
        .map(|priors| priors.values().sum())
        .unwrap_or(0.0)
}

fn validate_probes(
    model: &ProbeModel,
    orders: &BTreeMap<String, PartialOrder>,
) -> Result<(), ProbeModelError> {
    if model.probes.is_empty() {
        return Err(ProbeModelError::EmptyProbes);
    }
    let mut names = BTreeSet::new();
    for probe in &model.probes {
        if !names.insert(probe.name.clone()) {
            return Err(ProbeModelError::DuplicateProbe(probe.name.clone()));
        }
        for (component, value) in probe.cost.components() {
            if !value.is_finite() || value < 0.0 {
                return Err(ProbeModelError::InvalidCost {
                    probe: probe.name.clone(),
                    component,
                    value,
                });
            }
        }
        for world in probe.observations.keys() {
            if !orders.contains_key(world) {
                return Err(ProbeModelError::UnknownObservedWorld {
                    probe: probe.name.clone(),
                    world: world.clone(),
                });
            }
        }
        for world in orders.keys() {
            if !probe.observations.contains_key(world) {
                return Err(ProbeModelError::IncompleteObservations {
                    probe: probe.name.clone(),
                    world: world.clone(),
                });
            }
        }
    }
    Ok(())
}

fn analyze_probe(
    probe: &ProbeSpec,
    orders: &BTreeMap<String, PartialOrder>,
    total_orders: usize,
    weights: Option<&[f64]>,
    model: &ProbeModel,
) -> (ProbeReport, Vec<BTreeSet<String>>) {
    // Outcome label -> preimage worlds.
    let mut preimages: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (world, outcome) in &probe.observations {
        preimages
            .entry(outcome.as_str())
            .or_default()
            .insert(world.clone());
    }

    let mut outcomes = Vec::with_capacity(preimages.len());
    let mut worst_case_remaining_orders = 0;
    for (label, worlds) in &preimages {
        let possible: BTreeSet<PartialOrder> = worlds.iter().map(|world| orders[world]).collect();
        worst_case_remaining_orders = worst_case_remaining_orders.max(possible.len());
        outcomes.push(OutcomeReport {
            label: (*label).to_owned(),
            worlds: worlds.iter().cloned().collect(),
            possible_orders: possible.into_iter().collect(),
        });
    }

    // Label-free canonical partition, sorted by first member.
    let mut partition: Vec<BTreeSet<String>> = preimages.into_values().collect();
    partition.sort_by(|left, right| left.first().cmp(&right.first()));

    let (expected_remaining_orders, mutual_information) = weights
        .map(|weights| prior_statistics(&outcomes, orders, weights, model))
        .unzip();

    let report = ProbeReport {
        name: probe.name.clone(),
        cost: probe.cost,
        partition: partition
            .iter()
            .map(|block| block.iter().cloned().collect())
            .collect(),
        partition_sha256: partition_digest(&partition),
        outcomes,
        worst_case_remaining_orders,
        guaranteed_eliminated_orders: total_orders - worst_case_remaining_orders,
        guaranteed_order_bits: (total_orders as f64).log2()
            - (worst_case_remaining_orders as f64).log2(),
        expected_remaining_orders,
        order_outcome_mutual_information_bits: mutual_information,
    };
    (report, partition)
}

/// Expected remaining-order count and I(Order; Outcome) in bits under the
/// normalized prior. Zero-mass outcomes contribute nothing to either.
fn prior_statistics(
    outcomes: &[OutcomeReport],
    orders: &BTreeMap<String, PartialOrder>,
    weights: &[f64],
    model: &ProbeModel,
) -> (f64, f64) {
    let weight_of: BTreeMap<&str, f64> = model
        .worlds
        .iter()
        .zip(weights)
        .map(|(world, weight)| (world.name.as_str(), *weight))
        .collect();

    let mut order_mass: BTreeMap<PartialOrder, f64> = BTreeMap::new();
    for world in &model.worlds {
        *order_mass.entry(world.order).or_default() += weight_of[world.name.as_str()];
    }

    let mut expected_remaining = 0.0;
    let mut mutual_information = 0.0;
    for outcome in outcomes {
        let outcome_mass: f64 = outcome
            .worlds
            .iter()
            .map(|world| weight_of[world.as_str()])
            .sum();
        expected_remaining += outcome_mass * outcome.possible_orders.len() as f64;

        let mut joint: BTreeMap<PartialOrder, f64> = BTreeMap::new();
        for world in &outcome.worlds {
            *joint.entry(orders[world]).or_default() += weight_of[world.as_str()];
        }
        for (order, mass) in joint {
            if mass > 0.0 && outcome_mass > 0.0 {
                mutual_information += mass * (mass / (order_mass[&order] * outcome_mass)).log2();
            }
        }
    }
    (expected_remaining, mutual_information)
}

fn partition_digest(partition: &[BTreeSet<String>]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((partition.len() as u64).to_be_bytes());
    for block in partition {
        hasher.update((block.len() as u64).to_be_bytes());
        for world in block {
            hasher.update((world.len() as u64).to_be_bytes());
            hasher.update(world.as_bytes());
        }
    }
    let digest = hasher.finalize();
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut rendered, "{byte:02x}");
    }
    rendered
}

/// True when every block of `finer` is contained in some block of `coarser`.
fn refines(finer: &[BTreeSet<String>], coarser: &[BTreeSet<String>]) -> bool {
    finer
        .iter()
        .all(|block| coarser.iter().any(|candidate| block.is_subset(candidate)))
}

fn blackwell_frontier(
    probes: &[ProbeReport],
    partitions: &[Vec<BTreeSet<String>>],
) -> (Vec<String>, Vec<DominanceRecord>) {
    let mut dominance = Vec::new();
    let mut frontier = Vec::new();
    for (index, probe) in probes.iter().enumerate() {
        let mut dominated_by = None;
        for (other_index, other) in probes.iter().enumerate() {
            if index == other_index {
                continue;
            }
            let strictly_finer = refines(&partitions[other_index], &partitions[index])
                && partitions[other_index] != partitions[index];
            let equivalent = partitions[other_index] == partitions[index];
            let refines_this = strictly_finer || equivalent;
            if !refines_this || !other.cost.leq(probe.cost) {
                continue;
            }
            let strictly_cheaper = other.cost.strictly_cheaper_somewhere(probe.cost);
            if strictly_finer || strictly_cheaper {
                let mut reasons = Vec::new();
                if strictly_finer {
                    reasons.push("strictly finer outcome partition");
                } else {
                    reasons.push("observationally equivalent partition");
                }
                if strictly_cheaper {
                    reasons.push("strictly cheaper in at least one cost component");
                } else {
                    reasons.push("no more expensive in any cost component");
                }
                dominated_by = Some(DominanceRecord {
                    dominated: probe.name.clone(),
                    by: other.name.clone(),
                    reason: reasons.join("; "),
                });
                break;
            }
        }
        match dominated_by {
            Some(record) => dominance.push(record),
            None => frontier.push(probe.name.clone()),
        }
    }
    (frontier, dominance)
}

fn order_information_frontier(
    probes: &[ProbeReport],
    has_prior: bool,
) -> (Vec<String>, Vec<DominanceRecord>) {
    let mut dominance = Vec::new();
    let mut frontier = Vec::new();
    for (index, probe) in probes.iter().enumerate() {
        let mut dominated_by = None;
        for (other_index, other) in probes.iter().enumerate() {
            if index == other_index {
                continue;
            }
            if let Some(reason) = order_dominates(other, probe, has_prior) {
                dominated_by = Some(DominanceRecord {
                    dominated: probe.name.clone(),
                    by: other.name.clone(),
                    reason,
                });
                break;
            }
        }
        match dominated_by {
            Some(record) => dominance.push(record),
            None => frontier.push(probe.name.clone()),
        }
    }
    (frontier, dominance)
}

/// Reason `candidate` order-information dominates `probe`, if it does.
///
/// Axes: worst-case remaining orders (lower), each cost component (lower),
/// and with a prior also expected remaining orders (lower) and
/// I(Order; Outcome) (higher). Guaranteed bits are excluded as an axis: for a
/// fixed model they are a monotone function of the worst case.
fn order_dominates(
    candidate: &ProbeReport,
    probe: &ProbeReport,
    has_prior: bool,
) -> Option<String> {
    if candidate.worst_case_remaining_orders > probe.worst_case_remaining_orders
        || !candidate.cost.leq(probe.cost)
    {
        return None;
    }
    let mut strict = candidate.worst_case_remaining_orders < probe.worst_case_remaining_orders
        || candidate.cost.strictly_cheaper_somewhere(probe.cost);
    if has_prior {
        let candidate_expected = candidate.expected_remaining_orders.unwrap_or(f64::NAN);
        let probe_expected = probe.expected_remaining_orders.unwrap_or(f64::NAN);
        let candidate_information = candidate
            .order_outcome_mutual_information_bits
            .unwrap_or(f64::NAN);
        let probe_information = probe
            .order_outcome_mutual_information_bits
            .unwrap_or(f64::NAN);
        if !(candidate_expected <= probe_expected && candidate_information >= probe_information) {
            return None;
        }
        strict = strict
            || candidate_expected < probe_expected
            || candidate_information > probe_information;
    }
    strict.then(|| {
        let mut clauses = vec![format!(
            "worst-case remaining orders {} <= {}",
            candidate.worst_case_remaining_orders, probe.worst_case_remaining_orders
        )];
        if has_prior {
            clauses.push("weakly better expected remaining orders and order information".into());
        }
        clauses.push("cost weakly lower in every component".into());
        clauses.join("; ")
    })
}

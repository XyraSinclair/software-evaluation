//! Nonscalar comparison of candidate evidence-acquisition probes.
//!
//! A probe model declares a finite set of possible latent worlds, the Pareto
//! [`PartialOrder`] each world induces on the quality frontier, and a set of
//! deterministic candidate probes, each mapping every world to an outcome
//! label at an explicit componentwise cost. This module ranks nothing. It
//! computes two different nondominated sets:
//!
//! - The **Blackwell-cost frontier** removes a probe only when another probe's
//!   outcome partition refines its partition (for deterministic finite
//!   experiments, partition refinement is exactly the prior-independent
//!   Blackwell "more informative than" order: the coarser experiment's result
//!   is a garbling of the finer one's) while costing no more in every declared
//!   cost dimension, with at least one strict advantage.
//! - The **order-information frontier** first quotients away distinctions
//!   between worlds that induce the same Pareto order, then compares
//!   worst-case remaining-order counts, the exact prior-backed expected
//!   remaining-order count, and componentwise cost. A probe that only
//!   separates worlds with identical induced orders earns no order
//!   information here, however fine its partition.
//!
//! Dominance axes are exactly computable: counts, declared costs compared
//! as given, and expected remaining orders evaluated in exact rational
//! arithmetic from the declared prior masses. Mutual information in bits is
//! reported for context but is never a dominance axis, because a
//! transcendental quantity evaluated in floating point cannot honestly
//! license an eviction. No exchange rate between information and cost is
//! ever introduced, and no probe score is produced. Unresolved tradeoffs
//! survive on both frontiers.

use std::collections::{BTreeMap, BTreeSet};

use num_rational::BigRational;
use num_traits::{FromPrimitive, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::frontier::PartialOrder;
use crate::source::hex_digest;

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
    pub probes: Vec<OrderProbeSpec>,
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
pub struct OrderProbeSpec {
    pub name: String,
    /// Explicit componentwise cost. Every component is required; a missing
    /// or partial cost is rejected rather than filled in with zeros.
    pub cost: CostVector,
    /// Total observation function: every declared world must map to exactly
    /// one outcome label. Partial mappings are rejected.
    pub observations: BTreeMap<String, String>,
}

/// Componentwise cost. There is deliberately no combined magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
pub struct PriorEvidence {
    pub raw_mass: f64,
    /// Normalized weights in world order, rendered in floating point; they
    /// sum to one up to rounding. Dominance decisions never consume these:
    /// they are recomputed in exact rational arithmetic from the raw masses.
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
    /// Outcome-preimage partition of the worlds: canonically sorted blocks
    /// of sorted names. Outcome labels are deliberately excluded so
    /// relabeled but observationally identical probes are recognized as
    /// equivalent.
    pub partition: Vec<BTreeSet<String>>,
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
    /// Expected number of Pareto orders carried by positive-prior-mass
    /// worlds after observing the probe: sum over outcomes of P(outcome)
    /// times the count of orders with positive posterior mass. Rendered in
    /// floating point; dominance uses the exact rational value.
    pub expected_remaining_orders: Option<f64>,
    /// I(Order; Outcome) in bits under the normalized prior. Reported for
    /// context only; never a dominance axis.
    pub order_outcome_mutual_information_bits: Option<f64>,
    /// Exact rational expected remaining orders, used for dominance.
    #[serde(skip)]
    expected_remaining_orders_exact: Option<BigRational>,
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
    pub prior: Option<PriorEvidence>,
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
    let exact_weights = validate_priors(model, &orders)?;
    validate_probes(model, &orders)?;

    let distinct_orders: BTreeSet<PartialOrder> = orders.values().copied().collect();
    let total_orders = distinct_orders.len();

    let probes: Vec<ProbeReport> = model
        .probes
        .iter()
        .map(|probe| {
            analyze_probe(
                probe,
                &orders,
                total_orders,
                exact_weights.as_deref(),
                model,
            )
        })
        .collect();

    let (blackwell_frontier, blackwell_dominance) = frontier_by(&probes, blackwell_dominates);
    let (order_information_frontier, order_dominance) = frontier_by(&probes, order_dominates);

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
        prior: exact_weights.as_ref().map(|weights| PriorEvidence {
            raw_mass: raw_prior_mass(model),
            weights: weights
                .iter()
                .map(|weight| weight.to_f64().unwrap_or(f64::NAN))
                .collect(),
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

/// Validate priors and return the exact normalized weight per world, in
/// world declaration order. Every f64 is an exact rational, so normalizing
/// raw masses in [`BigRational`] arithmetic is exact end to end.
fn validate_priors(
    model: &ProbeModel,
    orders: &BTreeMap<String, PartialOrder>,
) -> Result<Option<Vec<BigRational>>, ProbeModelError> {
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
    for (world, value) in priors {
        if !value.is_finite() || *value < 0.0 {
            return Err(ProbeModelError::InvalidPrior {
                world: world.clone(),
                value: *value,
            });
        }
    }
    let raw: Vec<BigRational> = model
        .worlds
        .iter()
        .map(|world| {
            BigRational::from_f64(priors[&world.name]).expect("finite f64 is an exact rational")
        })
        .collect();
    let mass: BigRational = raw.iter().sum();
    if mass.is_zero() {
        return Err(ProbeModelError::ZeroPriorMass);
    }
    Ok(Some(raw.into_iter().map(|value| value / &mass).collect()))
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
    probe: &OrderProbeSpec,
    orders: &BTreeMap<String, PartialOrder>,
    total_orders: usize,
    exact_weights: Option<&[BigRational]>,
    model: &ProbeModel,
) -> ProbeReport {
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

    let (expected_exact, mutual_information) = match exact_weights {
        Some(weights) => {
            let (expected, information) = prior_statistics(&outcomes, orders, weights, model);
            (Some(expected), Some(information))
        }
        None => (None, None),
    };

    ProbeReport {
        name: probe.name.clone(),
        cost: probe.cost,
        partition_sha256: partition_digest(&partition),
        partition,
        outcomes,
        worst_case_remaining_orders,
        guaranteed_eliminated_orders: total_orders - worst_case_remaining_orders,
        guaranteed_order_bits: (total_orders as f64).log2()
            - (worst_case_remaining_orders as f64).log2(),
        expected_remaining_orders: expected_exact
            .as_ref()
            .map(|expected| expected.to_f64().unwrap_or(f64::NAN)),
        order_outcome_mutual_information_bits: mutual_information,
        expected_remaining_orders_exact: expected_exact,
    }
}

/// Exact expected remaining-order count and I(Order; Outcome) in bits under
/// the normalized prior.
///
/// Both quantities are measure-consistent: an order carried only by
/// zero-prior-mass worlds contributes to neither, so a measure-zero
/// distinction can never decide dominance. The purely possibilistic view of
/// the same probe lives in `worst_case_remaining_orders`.
fn prior_statistics(
    outcomes: &[OutcomeReport],
    orders: &BTreeMap<String, PartialOrder>,
    exact_weights: &[BigRational],
    model: &ProbeModel,
) -> (BigRational, f64) {
    let exact_weight_of: BTreeMap<&str, &BigRational> = model
        .worlds
        .iter()
        .zip(exact_weights)
        .map(|(world, weight)| (world.name.as_str(), weight))
        .collect();
    let float_weight_of: BTreeMap<&str, f64> = model
        .worlds
        .iter()
        .zip(exact_weights)
        .map(|(world, weight)| (world.name.as_str(), weight.to_f64().unwrap_or(f64::NAN)))
        .collect();

    let mut order_mass: BTreeMap<PartialOrder, f64> = BTreeMap::new();
    for world in &model.worlds {
        *order_mass.entry(world.order).or_default() += float_weight_of[world.name.as_str()];
    }

    let mut expected_remaining = BigRational::zero();
    let mut mutual_information = 0.0;
    for outcome in outcomes {
        let outcome_mass_exact: BigRational = outcome
            .worlds
            .iter()
            .map(|world| exact_weight_of[world.as_str()].clone())
            .sum();
        let positive_mass_orders: BTreeSet<PartialOrder> = outcome
            .worlds
            .iter()
            .filter(|world| !exact_weight_of[world.as_str()].is_zero())
            .map(|world| orders[world.as_str()])
            .collect();
        expected_remaining += outcome_mass_exact
            * BigRational::from_usize(positive_mass_orders.len())
                .expect("usize converts to a rational");

        let outcome_mass: f64 = outcome
            .worlds
            .iter()
            .map(|world| float_weight_of[world.as_str()])
            .sum();
        let mut joint: BTreeMap<PartialOrder, f64> = BTreeMap::new();
        for world in &outcome.worlds {
            *joint.entry(orders[world]).or_default() += float_weight_of[world.as_str()];
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
    hex_digest(&hasher.finalize())
}

/// True when every block of `finer` is contained in some block of `coarser`.
fn refines(finer: &[BTreeSet<String>], coarser: &[BTreeSet<String>]) -> bool {
    finer
        .iter()
        .all(|block| coarser.iter().any(|candidate| block.is_subset(candidate)))
}

/// Shared nondominated-set scan: a probe survives unless some other probe
/// dominates it under the given predicate. First dominator wins for the
/// dominance record; declaration order is preserved.
fn frontier_by(
    probes: &[ProbeReport],
    dominates: impl Fn(&ProbeReport, &ProbeReport) -> Option<String>,
) -> (Vec<String>, Vec<DominanceRecord>) {
    let mut dominance = Vec::new();
    let mut frontier = Vec::new();
    for (index, probe) in probes.iter().enumerate() {
        let record = probes
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .find_map(|(_, other)| {
                dominates(other, probe).map(|reason| DominanceRecord {
                    dominated: probe.name.clone(),
                    by: other.name.clone(),
                    reason,
                })
            });
        match record {
            Some(record) => dominance.push(record),
            None => frontier.push(probe.name.clone()),
        }
    }
    (frontier, dominance)
}

/// Reason `candidate` Blackwell-cost dominates `probe`, if it does.
fn blackwell_dominates(candidate: &ProbeReport, probe: &ProbeReport) -> Option<String> {
    let equivalent = candidate.partition == probe.partition;
    let strictly_finer = !equivalent && refines(&candidate.partition, &probe.partition);
    if (!equivalent && !strictly_finer) || !candidate.cost.leq(probe.cost) {
        return None;
    }
    let strictly_cheaper = candidate.cost.strictly_cheaper_somewhere(probe.cost);
    if !strictly_finer && !strictly_cheaper {
        return None;
    }
    let partition_clause = if strictly_finer {
        "strictly finer outcome partition"
    } else {
        "observationally equivalent partition"
    };
    let cost_clause = if strictly_cheaper {
        "strictly cheaper in at least one cost component"
    } else {
        "no more expensive in any cost component"
    };
    Some(format!("{partition_clause}; {cost_clause}"))
}

/// Reason `candidate` order-information dominates `probe`, if it does.
///
/// Axes: worst-case remaining orders (lower), each cost component (lower),
/// and, when a prior exists, the exact rational expected remaining-order
/// count (lower). Guaranteed bits are excluded as an axis (for a fixed
/// model they are a monotone function of the worst case), and mutual
/// information is excluded because it is not exactly computable.
fn order_dominates(candidate: &ProbeReport, probe: &ProbeReport) -> Option<String> {
    if candidate.worst_case_remaining_orders > probe.worst_case_remaining_orders
        || !candidate.cost.leq(probe.cost)
    {
        return None;
    }
    let mut strict = candidate.worst_case_remaining_orders < probe.worst_case_remaining_orders
        || candidate.cost.strictly_cheaper_somewhere(probe.cost);
    let mut expected_clause = None;
    if let (Some(candidate_expected), Some(probe_expected)) = (
        &candidate.expected_remaining_orders_exact,
        &probe.expected_remaining_orders_exact,
    ) {
        if candidate_expected > probe_expected {
            return None;
        }
        strict = strict || candidate_expected < probe_expected;
        expected_clause = Some("exactly weakly better expected remaining orders");
    }
    strict.then(|| {
        let mut clauses = vec![format!(
            "worst-case remaining orders {} <= {}",
            candidate.worst_case_remaining_orders, probe.worst_case_remaining_orders
        )];
        if let Some(clause) = expected_clause {
            clauses.push(clause.to_owned());
        }
        clauses.push("cost weakly lower in every component".to_owned());
        clauses.join("; ")
    })
}

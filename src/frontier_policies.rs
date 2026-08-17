//! Exact contingent policies for frontier order-region identification.
//!
//! The probe model from [`crate::frontier_probes`] fixes finite latent
//! worlds (each inducing one Pareto [`PartialOrder`]), deterministic total
//! probes, and componentwise costs. This module solves the induced
//! sequential decision problem exactly:
//!
//! ```text
//! surviving worlds -> choose an admissible probe -> observe an outcome
//!   -> restrict worlds -> stop when one order remains or nothing helps
//! ```
//!
//! The target is order-region identification, not latent-world
//! identification: policies are judged only by the order sets they can
//! terminate with. Probes that split survivors without immediately
//! separating orders are still expanded — restricting the surviving set can
//! make a later probe decisive — but probes constant on the survivors are
//! dropped, exactly: any policy using one maps to a weakly better policy
//! that skips it.
//!
//! For every reachable state — a surviving-world bitset plus the set of
//! still-unused probes — the dynamic program computes the exact
//! nondominated set of adaptive policies under three axes, none of them
//! scalarized:
//!
//! - worst-case number of Pareto orders still possible at termination;
//! - worst-case number of additional probe invocations;
//! - worst-case componentwise cost along any outcome path.
//!
//! Ties between incomparable policies are preserved, never broken by an
//! undeclared utility. Probes are one-shot per path. An optional hard
//! budget vector is enforced per path: no policy may schedule a probe whose
//! cost exceeds the remaining budget on any component, and expected-value
//! reasoning never launders a branch past a hard budget (this solver is
//! purely worst-case; the Bayesian frontier is a separate objective and a
//! separate implementation).
//!
//! At every state, probe `B` is pruned when another admissible probe `A`
//! restricted to the surviving worlds refines `B`'s partition, costs no
//! more in every component, and is strictly finer or strictly cheaper.
//! Restricted to survivors, `B`'s outcome is a function of `A`'s, and any
//! policy opening with `B` transforms into one opening with `A` that is
//! weakly better on all three axes (the later `A` node, if any, becomes
//! deterministic and collapses), so pruning preserves exactness. Every
//! pruning event is recorded as a certificate.
//!
//! The solver fails closed: declared limits on worlds, probes, memoized
//! states, and policy-frontier width abort the computation with an explicit
//! truncation reason rather than presenting a partial frontier as complete.

use std::collections::BTreeMap;

use serde::Serialize;
use thiserror::Error;

use crate::frontier::PartialOrder;
use crate::frontier_probes::{CostVector, ProbeModel, ProbeModelError, analyze_model};

pub const POLICY_SCHEMA_VERSION: &str = "seval.order-policies.v1";

/// Hard caps that keep the exact solver from unbounded exponential work.
/// Exceeding any cap is an explicit error, never a silently partial answer.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SolverLimits {
    pub max_worlds: usize,
    pub max_probes: usize,
    pub max_memo_states: usize,
    pub max_frontier_width: usize,
}

impl Default for SolverLimits {
    fn default() -> Self {
        Self {
            max_worlds: 20,
            max_probes: 12,
            max_memo_states: 200_000,
            max_frontier_width: 64,
        }
    }
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error(transparent)]
    Model(#[from] ProbeModelError),
    #[error("model declares {worlds} worlds; the exact solver caps at {limit}")]
    TooManyWorlds { worlds: usize, limit: usize },
    #[error("model declares {probes} probes; the exact solver caps at {limit}")]
    TooManyProbes { probes: usize, limit: usize },
    #[error(
        "memoized state cap {limit} exceeded; raise max_memo_states or shrink the model \
         (the completed prefix is not reported because it would misrepresent the frontier)"
    )]
    Truncated { limit: usize },
    #[error(
        "policy frontier width cap {limit} exceeded at an interior state; raise \
         max_frontier_width or shrink the model"
    )]
    FrontierWidthExceeded { limit: usize },
    #[error("budget component {component} is not a finite nonnegative number: {value}")]
    InvalidBudget { component: &'static str, value: f64 },
}

/// Why a policy leaf stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopStatus {
    /// Exactly one Pareto order remains.
    Identified,
    /// Multiple orders remain and every unused probe is constant on the
    /// surviving worlds, so no observation can help.
    ObservationallyIndistinguishable,
    /// Informative probes exist but every one violates the remaining hard
    /// budget on some component.
    BudgetExhausted,
    /// Informative admissible probes exist, but stopping here is a
    /// nondominated choice (cheaper or shorter than continuing).
    ElectiveStop,
}

/// An adaptive policy: stop, or run a probe and continue per outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum PolicyTree {
    Stop {
        status: StopStatus,
        /// Pareto orders still possible at this leaf, sorted.
        possible_orders: Vec<PartialOrder>,
        /// Worlds still surviving at this leaf, sorted by name.
        surviving_worlds: Vec<String>,
    },
    Probe {
        probe: String,
        /// Continuations keyed by the probe outcomes reachable from the
        /// surviving worlds.
        outcomes: BTreeMap<String, PolicyTree>,
    },
}

/// Worst-case guarantees of one policy. Never scalarized.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PolicySignature {
    pub worst_case_remaining_orders: usize,
    pub worst_case_additional_probes: usize,
    pub worst_case_cost: CostVector,
}

impl PolicySignature {
    fn dominates(&self, other: &Self) -> bool {
        let leq = self.worst_case_remaining_orders <= other.worst_case_remaining_orders
            && self.worst_case_additional_probes <= other.worst_case_additional_probes
            && cost_leq(self.worst_case_cost, other.worst_case_cost);
        let strict = self.worst_case_remaining_orders < other.worst_case_remaining_orders
            || self.worst_case_additional_probes < other.worst_case_additional_probes
            || cost_strictly_less_somewhere(self.worst_case_cost, other.worst_case_cost);
        leq && strict
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyRecord {
    pub signature: PolicySignature,
    pub tree: PolicyTree,
}

/// Evidence that a probe was exactly dominated at a specific state.
#[derive(Debug, Clone, Serialize)]
pub struct PruningCertificate {
    /// Surviving worlds at the state where pruning applied, sorted.
    pub surviving_worlds: Vec<String>,
    pub pruned: String,
    pub by: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyAnalysis {
    pub schema_version: String,
    pub limits: SolverLimits,
    /// Declared hard per-path budget, if any.
    pub budget: Option<CostVector>,
    /// Distinct Pareto orders across all declared worlds.
    pub initial_possible_orders: Vec<PartialOrder>,
    /// Exact nondominated adaptive policies from the initial state.
    pub policy_frontier: Vec<PolicyRecord>,
    pub pruning_certificates: Vec<PruningCertificate>,
    pub memoized_states: usize,
    /// Always true when this struct exists: the solver fails closed instead
    /// of emitting partial frontiers. Serialized so downstream consumers
    /// never have to guess.
    pub complete: bool,
}

/// Solve the exact robust policy frontier for a declared model.
pub fn solve_model(
    model: &ProbeModel,
    budget: Option<CostVector>,
    limits: SolverLimits,
) -> Result<PolicyAnalysis, PolicyError> {
    let analysis = analyze_model(model)?;
    if let Some(budget) = budget {
        for (component, value) in [
            ("dollars", budget.dollars),
            ("latency_ms", budget.latency_ms),
            ("invocations", budget.invocations),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PolicyError::InvalidBudget { component, value });
            }
        }
    }
    let world_count = analysis.worlds.len();
    if world_count > limits.max_worlds {
        return Err(PolicyError::TooManyWorlds {
            worlds: world_count,
            limit: limits.max_worlds,
        });
    }
    if analysis.probes.len() > limits.max_probes {
        return Err(PolicyError::TooManyProbes {
            probes: analysis.probes.len(),
            limit: limits.max_probes,
        });
    }

    let world_names: Vec<String> = analysis
        .worlds
        .iter()
        .map(|world| world.name.clone())
        .collect();
    let world_orders: Vec<PartialOrder> = analysis.worlds.iter().map(|world| world.order).collect();
    let world_index: BTreeMap<&str, usize> = world_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index))
        .collect();

    // Per probe: outcome label per world index, plus name and cost.
    let probes: Vec<ProbeTable> = model
        .probes
        .iter()
        .map(|probe| {
            let mut outcome_of_world = vec![0usize; world_count];
            let mut labels: Vec<String> = Vec::new();
            for (world, outcome) in &probe.observations {
                let label_index = labels
                    .iter()
                    .position(|label| label == outcome)
                    .unwrap_or_else(|| {
                        labels.push(outcome.clone());
                        labels.len() - 1
                    });
                outcome_of_world[world_index[world.as_str()]] = label_index;
            }
            ProbeTable {
                name: probe.name.clone(),
                cost: probe.cost,
                outcome_of_world,
                labels,
            }
        })
        .collect();

    let full_mask: u32 = if world_count == 32 {
        u32::MAX
    } else {
        (1u32 << world_count) - 1
    };
    let all_probes_mask: u32 = if probes.len() == 32 {
        u32::MAX
    } else {
        (1u32 << probes.len()) - 1
    };

    let mut solver = Solver {
        world_names: &world_names,
        world_orders: &world_orders,
        probes: &probes,
        budget,
        limits,
        memo: BTreeMap::new(),
        certificates: Vec::new(),
    };
    let frontier = solver.solve(full_mask, all_probes_mask)?;

    Ok(PolicyAnalysis {
        schema_version: POLICY_SCHEMA_VERSION.to_owned(),
        limits,
        budget,
        initial_possible_orders: analysis.distinct_orders.clone(),
        policy_frontier: frontier,
        pruning_certificates: solver.certificates,
        memoized_states: solver.memo.len(),
        complete: true,
    })
}

struct ProbeTable {
    name: String,
    cost: CostVector,
    /// Outcome label index per world index.
    outcome_of_world: Vec<usize>,
    labels: Vec<String>,
}

struct Solver<'a> {
    world_names: &'a [String],
    world_orders: &'a [PartialOrder],
    probes: &'a [ProbeTable],
    budget: Option<CostVector>,
    limits: SolverLimits,
    memo: BTreeMap<(u32, u32), Vec<PolicyRecord>>,
    certificates: Vec<PruningCertificate>,
}

impl Solver<'_> {
    /// Exact nondominated policy set for the state (surviving worlds,
    /// unused probes). The remaining budget is a function of the unused
    /// probe set, so the memo key needs nothing more.
    fn solve(&mut self, survivors: u32, unused: u32) -> Result<Vec<PolicyRecord>, PolicyError> {
        if let Some(cached) = self.memo.get(&(survivors, unused)) {
            return Ok(cached.clone());
        }
        if self.memo.len() >= self.limits.max_memo_states {
            return Err(PolicyError::Truncated {
                limit: self.limits.max_memo_states,
            });
        }

        let remaining_budget = self.remaining_budget(unused);
        let orders_here = self.possible_orders(survivors);
        let informative = self.informative_probes(survivors, unused);
        let admissible: Vec<usize> = informative
            .iter()
            .copied()
            .filter(|&probe| self.within_budget(probe, remaining_budget))
            .collect();
        let admissible = self.blackwell_prune(survivors, &admissible);

        let stop_status = if orders_here.len() == 1 {
            StopStatus::Identified
        } else if informative.is_empty() {
            StopStatus::ObservationallyIndistinguishable
        } else if admissible.is_empty() {
            StopStatus::BudgetExhausted
        } else {
            StopStatus::ElectiveStop
        };
        let mut records = vec![PolicyRecord {
            signature: PolicySignature {
                worst_case_remaining_orders: orders_here.len(),
                worst_case_additional_probes: 0,
                worst_case_cost: ZERO_COST,
            },
            tree: PolicyTree::Stop {
                status: stop_status,
                possible_orders: orders_here.clone(),
                surviving_worlds: self.world_subset(survivors),
            },
        }];

        for probe_index in admissible {
            let cells = self.outcome_cells(probe_index, survivors);
            // Solve each reachable outcome cell, then take the Cartesian
            // product of child frontiers: a policy commits one continuation
            // per outcome before observing which outcome occurs.
            let mut child_frontiers: Vec<(String, Vec<PolicyRecord>)> = Vec::new();
            for (label, cell) in cells {
                child_frontiers.push((label, self.solve(cell, unused & !(1u32 << probe_index))?));
            }
            let cost = self.probes[probe_index].cost;
            let mut combos: Vec<(PolicySignature, BTreeMap<String, PolicyTree>)> = vec![(
                PolicySignature {
                    worst_case_remaining_orders: 0,
                    worst_case_additional_probes: 0,
                    worst_case_cost: ZERO_COST,
                },
                BTreeMap::new(),
            )];
            for (label, frontier) in &child_frontiers {
                let mut next: Vec<(PolicySignature, BTreeMap<String, PolicyTree>)> = Vec::new();
                for (signature, outcomes) in &combos {
                    for child in frontier {
                        let mut outcomes = outcomes.clone();
                        outcomes.insert(label.clone(), child.tree.clone());
                        next.push((
                            PolicySignature {
                                worst_case_remaining_orders: signature
                                    .worst_case_remaining_orders
                                    .max(child.signature.worst_case_remaining_orders),
                                worst_case_additional_probes: signature
                                    .worst_case_additional_probes
                                    .max(child.signature.worst_case_additional_probes),
                                worst_case_cost: cost_max(
                                    signature.worst_case_cost,
                                    child.signature.worst_case_cost,
                                ),
                            },
                            outcomes,
                        ));
                    }
                }
                // Prune the partial product eagerly; dominated partial
                // combinations can never yield nondominated completions
                // because every axis composes monotonically (max / max /
                // componentwise max).
                next = pareto_prune_combos(next);
                if next.len() > self.limits.max_frontier_width {
                    return Err(PolicyError::FrontierWidthExceeded {
                        limit: self.limits.max_frontier_width,
                    });
                }
                combos = next;
            }
            for (signature, outcomes) in combos {
                records.push(PolicyRecord {
                    signature: PolicySignature {
                        worst_case_remaining_orders: signature.worst_case_remaining_orders,
                        worst_case_additional_probes: signature.worst_case_additional_probes + 1,
                        worst_case_cost: cost_add(cost, signature.worst_case_cost),
                    },
                    tree: PolicyTree::Probe {
                        probe: self.probes[probe_index].name.clone(),
                        outcomes,
                    },
                });
            }
        }

        let records = pareto_prune(records);
        if records.len() > self.limits.max_frontier_width {
            return Err(PolicyError::FrontierWidthExceeded {
                limit: self.limits.max_frontier_width,
            });
        }
        self.memo.insert((survivors, unused), records.clone());
        Ok(records)
    }

    /// Total spend so far is determined by which probes have been used, so
    /// the remaining budget is a pure function of the unused mask.
    fn remaining_budget(&self, unused: u32) -> Option<CostVector> {
        let budget = self.budget?;
        let mut remaining = budget;
        for (index, probe) in self.probes.iter().enumerate() {
            if unused & (1u32 << index) == 0 {
                remaining = CostVector {
                    dollars: remaining.dollars - probe.cost.dollars,
                    latency_ms: remaining.latency_ms - probe.cost.latency_ms,
                    invocations: remaining.invocations - probe.cost.invocations,
                };
            }
        }
        Some(remaining)
    }

    fn within_budget(&self, probe: usize, remaining: Option<CostVector>) -> bool {
        match remaining {
            None => true,
            Some(remaining) => cost_leq(self.probes[probe].cost, remaining),
        }
    }

    /// Probes that split the surviving worlds into at least two cells whose
    /// order sets differ from certainty — i.e. probes that can change the
    /// possible order set. A probe separating only same-order worlds is
    /// informative about worlds but not about the decision; it still counts
    /// as informative here only if it splits survivors at all, because a
    /// split can enable later probes. Constant probes never count.
    fn informative_probes(&self, survivors: u32, unused: u32) -> Vec<usize> {
        (0..self.probes.len())
            .filter(|&index| unused & (1u32 << index) != 0)
            .filter(|&index| self.outcome_cells(index, survivors).len() >= 2)
            .collect()
    }

    /// Reachable outcome cells of a probe restricted to survivors, keyed by
    /// outcome label.
    fn outcome_cells(&self, probe: usize, survivors: u32) -> Vec<(String, u32)> {
        let table = &self.probes[probe];
        let mut cells: BTreeMap<usize, u32> = BTreeMap::new();
        let mut mask = survivors;
        while mask != 0 {
            let world = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            *cells.entry(table.outcome_of_world[world]).or_default() |= 1u32 << world;
        }
        cells
            .into_iter()
            .map(|(label_index, cell)| (table.labels[label_index].clone(), cell))
            .collect()
    }

    fn possible_orders(&self, survivors: u32) -> Vec<PartialOrder> {
        let mut orders: Vec<PartialOrder> = Vec::new();
        let mut mask = survivors;
        while mask != 0 {
            let world = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let order = self.world_orders[world];
            if !orders.contains(&order) {
                orders.push(order);
            }
        }
        orders.sort();
        orders
    }

    fn world_subset(&self, survivors: u32) -> Vec<String> {
        let mut names = Vec::new();
        let mut mask = survivors;
        while mask != 0 {
            let world = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            names.push(self.world_names[world].clone());
        }
        names
    }

    /// Exact state-local Blackwell pruning over admissible probes. Restricted
    /// to survivors, if probe A's cells refine probe B's cells and A costs no
    /// more everywhere (strictly finer or strictly cheaper), any policy
    /// opening with B transforms into a weakly better one opening with A, so
    /// B never needs expansion at this state.
    fn blackwell_prune(&mut self, survivors: u32, admissible: &[usize]) -> Vec<usize> {
        let cells: BTreeMap<usize, Vec<u32>> = admissible
            .iter()
            .map(|&probe| {
                (
                    probe,
                    self.outcome_cells(probe, survivors)
                        .into_iter()
                        .map(|(_, cell)| cell)
                        .collect(),
                )
            })
            .collect();
        let mut kept = Vec::new();
        for &probe in admissible {
            let dominated = admissible.iter().copied().find(|&other| {
                if other == probe {
                    return false;
                }
                let finer = &cells[&other];
                let coarser = &cells[&probe];
                let refines = finer
                    .iter()
                    .all(|cell| coarser.iter().any(|target| cell & !target == 0));
                if !refines {
                    return false;
                }
                let strictly_finer = finer.len() > coarser.len();
                let cost_ok = cost_leq(self.probes[other].cost, self.probes[probe].cost);
                let strictly_cheaper =
                    cost_strictly_less_somewhere(self.probes[other].cost, self.probes[probe].cost);
                cost_ok && (strictly_finer || strictly_cheaper)
            });
            match dominated {
                Some(by) => self.certificates.push(PruningCertificate {
                    surviving_worlds: self.world_subset(survivors),
                    pruned: self.probes[probe].name.clone(),
                    by: self.probes[by].name.clone(),
                    reason: "refines the pruned probe's surviving-world partition at \
                             weakly lower componentwise cost with a strict advantage"
                        .to_owned(),
                }),
                None => kept.push(probe),
            }
        }
        kept
    }
}

const ZERO_COST: CostVector = CostVector {
    dollars: 0.0,
    latency_ms: 0.0,
    invocations: 0.0,
};

fn cost_leq(left: CostVector, right: CostVector) -> bool {
    left.dollars <= right.dollars
        && left.latency_ms <= right.latency_ms
        && left.invocations <= right.invocations
}

fn cost_strictly_less_somewhere(left: CostVector, right: CostVector) -> bool {
    left.dollars < right.dollars
        || left.latency_ms < right.latency_ms
        || left.invocations < right.invocations
}

fn cost_add(left: CostVector, right: CostVector) -> CostVector {
    CostVector {
        dollars: left.dollars + right.dollars,
        latency_ms: left.latency_ms + right.latency_ms,
        invocations: left.invocations + right.invocations,
    }
}

fn cost_max(left: CostVector, right: CostVector) -> CostVector {
    CostVector {
        dollars: left.dollars.max(right.dollars),
        latency_ms: left.latency_ms.max(right.latency_ms),
        invocations: left.invocations.max(right.invocations),
    }
}

fn pareto_prune(records: Vec<PolicyRecord>) -> Vec<PolicyRecord> {
    let mut kept: Vec<PolicyRecord> = Vec::new();
    for record in records {
        if kept
            .iter()
            .any(|existing| existing.signature.dominates(&record.signature))
            || kept
                .iter()
                .any(|existing| existing.signature == record.signature)
        {
            continue;
        }
        kept.retain(|existing| !record.signature.dominates(&existing.signature));
        kept.push(record);
    }
    kept
}

fn pareto_prune_combos(
    combos: Vec<(PolicySignature, BTreeMap<String, PolicyTree>)>,
) -> Vec<(PolicySignature, BTreeMap<String, PolicyTree>)> {
    let mut kept: Vec<(PolicySignature, BTreeMap<String, PolicyTree>)> = Vec::new();
    for (signature, outcomes) in combos {
        if kept
            .iter()
            .any(|(existing, _)| existing.dominates(&signature))
            || kept.iter().any(|(existing, _)| *existing == signature)
        {
            continue;
        }
        kept.retain(|(existing, _)| !signature.dominates(existing));
        kept.push((signature, outcomes));
    }
    kept
}

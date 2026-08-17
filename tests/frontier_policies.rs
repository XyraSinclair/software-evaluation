//! Verification of the exact robust policy solver: an independently written
//! exhaustive decision-tree oracle over randomized tiny models, metamorphic
//! invariances, budget semantics, and fail-closed truncation.

use std::collections::BTreeMap;

use software_evaluation::frontier::PartialOrder;
use software_evaluation::frontier_policies::{
    PolicyError, PolicyTree, SolverLimits, StopStatus, solve_model,
};
use software_evaluation::frontier_probes::{CostVector, OrderProbeSpec, ProbeModel, WorldSpec};

fn world(name: &str, order: PartialOrder) -> WorldSpec {
    WorldSpec {
        name: name.to_owned(),
        order,
    }
}

fn probe(name: &str, cost: CostVector, observations: &[(&str, &str)]) -> OrderProbeSpec {
    OrderProbeSpec {
        name: name.to_owned(),
        cost,
        observations: observations
            .iter()
            .map(|(world, outcome)| ((*world).to_owned(), (*outcome).to_owned()))
            .collect(),
    }
}

fn cost(dollars: f64, latency_ms: f64, invocations: f64) -> CostVector {
    CostVector {
        dollars,
        latency_ms,
        invocations,
    }
}

/// (worst orders, worst probes, worst cost triple) — an exact signature.
type Sig = (usize, usize, (f64, f64, f64));

fn solver_signatures(model: &ProbeModel, budget: Option<CostVector>) -> Vec<Sig> {
    let analysis = solve_model(model, budget, SolverLimits::default()).expect("solvable model");
    let mut signatures: Vec<Sig> = analysis
        .policy_frontier
        .iter()
        .map(|record| {
            (
                record.signature.worst_case_remaining_orders,
                record.signature.worst_case_additional_probes,
                (
                    record.signature.worst_case_cost.dollars,
                    record.signature.worst_case_cost.latency_ms,
                    record.signature.worst_case_cost.invocations,
                ),
            )
        })
        .collect();
    signatures.sort_by(|a, b| a.partial_cmp(b).expect("finite signatures"));
    signatures
}

/// Independently written exhaustive oracle: enumerate EVERY adaptive
/// decision tree (including ones using constant probes and Blackwell-
/// dominated probes), collect exact worst-case signatures, and keep the
/// nondominated set. No memoization, no pruning, no eager filtering.
mod oracle {
    use super::Sig;

    #[derive(Clone)]
    pub struct TinyModel {
        /// Order index per world.
        pub orders: Vec<usize>,
        /// Per probe: outcome index per world.
        pub probes: Vec<Vec<usize>>,
        /// Per probe: (dollars, latency, invocations).
        pub costs: Vec<(f64, f64, f64)>,
    }

    pub fn frontier(model: &TinyModel, budget: Option<(f64, f64, f64)>) -> Vec<Sig> {
        let all_worlds: Vec<usize> = (0..model.orders.len()).collect();
        let unused: Vec<usize> = (0..model.probes.len()).collect();
        let mut signatures = Vec::new();
        collect(model, &all_worlds, &unused, budget, &mut signatures);
        let mut nondominated: Vec<Sig> = Vec::new();
        for candidate in &signatures {
            if signatures.iter().any(|other| dominates(other, candidate))
                || nondominated.contains(candidate)
            {
                continue;
            }
            nondominated.push(*candidate);
        }
        nondominated.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        nondominated
    }

    fn dominates(a: &Sig, b: &Sig) -> bool {
        let leq = a.0 <= b.0 && a.1 <= b.1 && a.2.0 <= b.2.0 && a.2.1 <= b.2.1 && a.2.2 <= b.2.2;
        let strict = a.0 < b.0 || a.1 < b.1 || a.2.0 < b.2.0 || a.2.1 < b.2.1 || a.2.2 < b.2.2;
        leq && strict
    }

    fn remaining_orders(model: &TinyModel, worlds: &[usize]) -> usize {
        let mut orders: Vec<usize> = worlds.iter().map(|&world| model.orders[world]).collect();
        orders.sort_unstable();
        orders.dedup();
        orders.len()
    }

    /// Enumerate every tree from this state and push its signature.
    fn collect(
        model: &TinyModel,
        worlds: &[usize],
        unused: &[usize],
        budget: Option<(f64, f64, f64)>,
        signatures: &mut Vec<Sig>,
    ) {
        // Stopping here is always a tree.
        signatures.push((remaining_orders(model, worlds), 0, (0.0, 0.0, 0.0)));

        for (position, &probe) in unused.iter().enumerate() {
            let cost = model.costs[probe];
            if let Some(remaining) = budget {
                if cost.0 > remaining.0 || cost.1 > remaining.1 || cost.2 > remaining.2 {
                    continue;
                }
            }
            let next_budget = budget.map(|b| (b.0 - cost.0, b.1 - cost.1, b.2 - cost.2));
            // Partition survivors by outcome.
            let mut cells: Vec<Vec<usize>> = Vec::new();
            let mut labels: Vec<usize> = Vec::new();
            for &world in worlds {
                let outcome = model.probes[probe][world];
                match labels.iter().position(|&label| label == outcome) {
                    Some(index) => cells[index].push(world),
                    None => {
                        labels.push(outcome);
                        cells.push(vec![world]);
                    }
                }
            }
            let next_unused: Vec<usize> = unused
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != position)
                .map(|(_, &p)| p)
                .collect();
            // All signature sets per cell, then every combination.
            let mut per_cell: Vec<Vec<Sig>> = Vec::new();
            for cell in &cells {
                let mut cell_signatures = Vec::new();
                collect(model, cell, &next_unused, next_budget, &mut cell_signatures);
                per_cell.push(cell_signatures);
            }
            let mut combos: Vec<Sig> = vec![(0, 0, (0.0, 0.0, 0.0))];
            for cell_signatures in &per_cell {
                let mut next: Vec<Sig> = Vec::new();
                for combo in &combos {
                    for child in cell_signatures {
                        next.push((
                            combo.0.max(child.0),
                            combo.1.max(child.1),
                            (
                                combo.2.0.max(child.2.0),
                                combo.2.1.max(child.2.1),
                                combo.2.2.max(child.2.2),
                            ),
                        ));
                    }
                }
                combos = next;
            }
            for combo in combos {
                signatures.push((
                    combo.0,
                    combo.1 + 1,
                    (combo.2.0 + cost.0, combo.2.1 + cost.1, combo.2.2 + cost.2),
                ));
            }
        }
    }
}

const ORDER_VARIANTS: [PartialOrder; 3] = [
    PartialOrder::RightDominates,
    PartialOrder::LeftDominates,
    PartialOrder::Tradeoff,
];

fn tiny_to_model(tiny: &oracle::TinyModel) -> ProbeModel {
    ProbeModel {
        worlds: tiny
            .orders
            .iter()
            .enumerate()
            .map(|(index, &order)| world(&format!("w{index}"), ORDER_VARIANTS[order]))
            .collect(),
        priors: None,
        probes: tiny
            .probes
            .iter()
            .zip(&tiny.costs)
            .enumerate()
            .map(
                |(probe_index, (outcomes, &(dollars, latency, invocations)))| OrderProbeSpec {
                    name: format!("p{probe_index}"),
                    cost: cost(dollars, latency, invocations),
                    observations: outcomes
                        .iter()
                        .enumerate()
                        .map(|(world_index, &outcome)| {
                            (format!("w{world_index}"), format!("o{outcome}"))
                        })
                        .collect(),
                },
            )
            .collect(),
    }
}

#[test]
fn solver_matches_exhaustive_oracle_on_randomized_tiny_models() {
    // Deterministic LCG; no clock, no external randomness.
    let mut state = 0x2545F4914F6CDD1Du64;
    let mut next = move |bound: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % bound
    };
    for case in 0..40 {
        let world_count = 2 + next(3) as usize; // 2..=4
        let probe_count = 1 + next(3) as usize; // 1..=3
        let tiny = oracle::TinyModel {
            orders: (0..world_count).map(|_| next(3) as usize).collect(),
            probes: (0..probe_count)
                .map(|_| (0..world_count).map(|_| next(3) as usize).collect())
                .collect(),
            costs: (0..probe_count)
                .map(|_| (next(4) as f64, next(4) as f64, next(3) as f64))
                .collect(),
        };
        let budget = if next(2) == 0 {
            None
        } else {
            Some((next(6) as f64, next(6) as f64, next(4) as f64))
        };
        let expected = oracle::frontier(&tiny, budget);
        let model = tiny_to_model(&tiny);
        let actual = solver_signatures(
            &model,
            budget.map(|(dollars, latency_ms, invocations)| cost(dollars, latency_ms, invocations)),
        );
        assert_eq!(
            actual, expected,
            "case {case}: solver frontier diverged from the exhaustive oracle \
             (worlds={world_count}, probes={probe_count}, budget={budget:?})"
        );
    }
}

#[test]
fn renaming_worlds_probes_and_outcomes_changes_no_signatures() {
    let base = ProbeModel {
        worlds: vec![
            world("alpha", PartialOrder::RightDominates),
            world("beta", PartialOrder::LeftDominates),
            world("gamma", PartialOrder::Tradeoff),
        ],
        priors: None,
        probes: vec![
            probe(
                "first",
                cost(2.0, 10.0, 1.0),
                &[("alpha", "x"), ("beta", "y"), ("gamma", "y")],
            ),
            probe(
                "second",
                cost(1.0, 5.0, 1.0),
                &[("alpha", "m"), ("beta", "m"), ("gamma", "n")],
            ),
        ],
    };
    let renamed = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
            world("w-c", PartialOrder::Tradeoff),
        ],
        priors: None,
        probes: vec![
            probe(
                "renamed-first",
                cost(2.0, 10.0, 1.0),
                &[("w-a", "up"), ("w-b", "down"), ("w-c", "down")],
            ),
            probe(
                "renamed-second",
                cost(1.0, 5.0, 1.0),
                &[("w-a", "same"), ("w-b", "same"), ("w-c", "other")],
            ),
        ],
    };
    assert_eq!(
        solver_signatures(&base, None),
        solver_signatures(&renamed, None)
    );
}

#[test]
fn adding_a_constant_probe_changes_no_signatures() {
    let mut model = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![probe(
            "decisive",
            cost(3.0, 20.0, 1.0),
            &[("w-a", "up"), ("w-b", "down")],
        )],
    };
    let baseline = solver_signatures(&model, None);
    model.probes.push(probe(
        "constant",
        cost(0.0, 0.0, 1.0),
        &[("w-a", "same"), ("w-b", "same")],
    ));
    assert_eq!(solver_signatures(&model, None), baseline);
}

#[test]
fn duplicating_a_world_with_identical_observations_changes_no_signatures() {
    let base = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![probe(
            "split",
            cost(1.0, 1.0, 1.0),
            &[("w-a", "up"), ("w-b", "down")],
        )],
    };
    let doubled = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-a2", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![probe(
            "split",
            cost(1.0, 1.0, 1.0),
            &[("w-a", "up"), ("w-a2", "up"), ("w-b", "down")],
        )],
    };
    assert_eq!(
        solver_signatures(&base, None),
        solver_signatures(&doubled, None)
    );
}

#[test]
fn hard_budget_produces_budget_exhausted_leaves_not_expected_value_laundering() {
    let model = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![probe(
            "expensive",
            cost(5.0, 100.0, 1.0),
            &[("w-a", "up"), ("w-b", "down")],
        )],
    };
    let analysis = solve_model(
        &model,
        Some(cost(4.0, 1000.0, 10.0)),
        SolverLimits::default(),
    )
    .expect("solvable");
    assert_eq!(analysis.policy_frontier.len(), 1);
    match &analysis.policy_frontier[0].tree {
        PolicyTree::Stop { status, .. } => assert_eq!(*status, StopStatus::BudgetExhausted),
        other => panic!("expected a budget-exhausted stop, got {other:?}"),
    }
}

#[test]
fn decisive_probe_yields_identified_leaves_and_blackwell_pruning_certificates() {
    let model = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![
            probe(
                "decisive-cheap",
                cost(1.0, 1.0, 1.0),
                &[("w-a", "up"), ("w-b", "down")],
            ),
            probe(
                "decisive-expensive",
                cost(2.0, 2.0, 1.0),
                &[("w-a", "hot"), ("w-b", "cold")],
            ),
        ],
    };
    let analysis = solve_model(&model, None, SolverLimits::default()).expect("solvable");
    let identified = analysis
        .policy_frontier
        .iter()
        .find(|record| record.signature.worst_case_remaining_orders == 1)
        .expect("an identifying policy must be on the frontier");
    assert_eq!(identified.signature.worst_case_additional_probes, 1);
    assert_eq!(identified.signature.worst_case_cost.dollars, 1.0);
    match &identified.tree {
        PolicyTree::Probe { probe, outcomes } => {
            assert_eq!(probe, "decisive-cheap");
            for child in outcomes.values() {
                match child {
                    PolicyTree::Stop {
                        status,
                        possible_orders,
                        ..
                    } => {
                        assert_eq!(*status, StopStatus::Identified);
                        assert_eq!(possible_orders.len(), 1);
                    }
                    other => panic!("expected identified leaves, got {other:?}"),
                }
            }
        }
        other => panic!("expected a probing policy, got {other:?}"),
    }
    assert!(
        analysis
            .pruning_certificates
            .iter()
            .any(|certificate| certificate.pruned == "decisive-expensive"
                && certificate.by == "decisive-cheap"),
        "the strictly costlier equivalent probe must carry a pruning certificate"
    );
}

#[test]
fn state_cap_fails_closed_instead_of_reporting_a_partial_frontier() {
    let model = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::LeftDominates),
            world("w-c", PartialOrder::Tradeoff),
        ],
        priors: None,
        probes: vec![
            probe(
                "p1",
                cost(1.0, 1.0, 1.0),
                &[("w-a", "x"), ("w-b", "y"), ("w-c", "y")],
            ),
            probe(
                "p2",
                cost(1.0, 2.0, 1.0),
                &[("w-a", "m"), ("w-b", "m"), ("w-c", "n")],
            ),
        ],
    };
    let limits = SolverLimits {
        max_memo_states: 1,
        ..SolverLimits::default()
    };
    match solve_model(&model, None, limits) {
        Err(PolicyError::Truncated { limit }) => assert_eq!(limit, 1),
        other => panic!("expected fail-closed truncation, got {other:?}"),
    }
}

#[test]
fn order_useless_splits_are_kept_when_they_enable_later_identification() {
    // Probe "presort" separates only same-order pairs (each cell still
    // carries both orders), yet enables "finisher", whose cells are mixed
    // on the full world set but decisive on each presorted half.
    let worlds = vec![
        world("a1", PartialOrder::RightDominates),
        world("b1", PartialOrder::LeftDominates),
        world("a2", PartialOrder::RightDominates),
        world("b2", PartialOrder::LeftDominates),
    ];
    let model = ProbeModel {
        worlds,
        priors: None,
        probes: vec![
            probe(
                "presort",
                cost(1.0, 1.0, 1.0),
                &[("a1", "one"), ("b1", "one"), ("a2", "two"), ("b2", "two")],
            ),
            probe(
                "finisher",
                cost(1.0, 1.0, 1.0),
                &[("a1", "x"), ("b1", "y"), ("a2", "y"), ("b2", "x")],
            ),
        ],
    };
    let analysis = solve_model(&model, None, SolverLimits::default()).expect("solvable");
    let identified = analysis
        .policy_frontier
        .iter()
        .find(|record| record.signature.worst_case_remaining_orders == 1)
        .expect("identification is achievable and must appear on the frontier");
    assert_eq!(identified.signature.worst_case_additional_probes, 2);

    // A BTreeMap-based sanity check that both probes appear on the path.
    fn probes_used(tree: &PolicyTree, used: &mut BTreeMap<String, usize>) {
        if let PolicyTree::Probe { probe, outcomes } = tree {
            *used.entry(probe.clone()).or_default() += 1;
            for child in outcomes.values() {
                probes_used(child, used);
            }
        }
    }
    let mut used = BTreeMap::new();
    probes_used(&identified.tree, &mut used);
    assert!(used.contains_key("presort") && used.contains_key("finisher"));
}

//! Adversarial coverage of the Blackwell and order-information probe
//! frontiers: refinement dominance, relabeled equivalence, order-irrelevant
//! distinctions, exact information accounting, unresolved tradeoffs, and
//! model-validation rejections.

use std::collections::BTreeMap;

use software_evaluation::frontier::PartialOrder;
use software_evaluation::frontier_probes::{
    CostVector, ProbeModel, ProbeModelError, ProbeSpec, WorldSpec, analyze_model,
};

fn world(name: &str, order: PartialOrder) -> WorldSpec {
    WorldSpec {
        name: name.to_owned(),
        order,
    }
}

fn probe(name: &str, cost: CostVector, observations: &[(&str, &str)]) -> ProbeSpec {
    ProbeSpec {
        name: name.to_owned(),
        cost,
        observations: observations
            .iter()
            .map(|(world, outcome)| ((*world).to_owned(), (*outcome).to_owned()))
            .collect(),
    }
}

fn dollars(amount: f64) -> CostVector {
    CostVector {
        dollars: amount,
        latency_ms: 0.0,
        invocations: 0.0,
    }
}

fn four_worlds() -> Vec<WorldSpec> {
    vec![
        world("w-right", PartialOrder::RightDominates),
        world("w-left", PartialOrder::LeftDominates),
        world("w-trade", PartialOrder::Tradeoff),
        world("w-equal", PartialOrder::Equivalent),
    ]
}

#[test]
fn strictly_finer_partition_at_equal_cost_blackwell_dominates() {
    let model = ProbeModel {
        worlds: four_worlds(),
        priors: None,
        probes: vec![
            probe(
                "coarse",
                dollars(1.0),
                &[
                    ("w-right", "a"),
                    ("w-left", "a"),
                    ("w-trade", "b"),
                    ("w-equal", "b"),
                ],
            ),
            probe(
                "fine",
                dollars(1.0),
                &[
                    ("w-right", "p"),
                    ("w-left", "q"),
                    ("w-trade", "r"),
                    ("w-equal", "s"),
                ],
            ),
        ],
    };
    let analysis = analyze_model(&model).expect("valid model");
    assert_eq!(analysis.blackwell_frontier, vec!["fine"]);
    assert_eq!(analysis.blackwell_dominance.len(), 1);
    assert_eq!(analysis.blackwell_dominance[0].dominated, "coarse");
    assert_eq!(analysis.blackwell_dominance[0].by, "fine");
}

#[test]
fn cheaper_relabeled_equivalent_probe_dominates_and_shares_digest() {
    let model = ProbeModel {
        worlds: four_worlds(),
        priors: None,
        probes: vec![
            probe(
                "expensive-labels",
                dollars(5.0),
                &[
                    ("w-right", "alpha"),
                    ("w-left", "alpha"),
                    ("w-trade", "beta"),
                    ("w-equal", "beta"),
                ],
            ),
            probe(
                "cheap-labels",
                dollars(1.0),
                &[
                    ("w-right", "x"),
                    ("w-left", "x"),
                    ("w-trade", "y"),
                    ("w-equal", "y"),
                ],
            ),
        ],
    };
    let analysis = analyze_model(&model).expect("valid model");
    assert_eq!(
        analysis.probes[0].partition_sha256, analysis.probes[1].partition_sha256,
        "relabeled but observationally identical probes share one partition digest"
    );
    assert_eq!(analysis.blackwell_frontier, vec!["cheap-labels"]);
    assert_eq!(
        analysis.blackwell_dominance[0].dominated,
        "expensive-labels"
    );
}

#[test]
fn order_irrelevant_distinction_earns_no_order_information() {
    // Both worlds induce the same Pareto order; separating them is
    // Blackwell-informative but order-useless.
    let model = ProbeModel {
        worlds: vec![
            world("w-a", PartialOrder::RightDominates),
            world("w-b", PartialOrder::RightDominates),
        ],
        priors: None,
        probes: vec![
            probe("separator", dollars(3.0), &[("w-a", "a"), ("w-b", "b")]),
            probe("null", dollars(0.0), &[("w-a", "same"), ("w-b", "same")]),
        ],
    };
    let analysis = analyze_model(&model).expect("valid model");
    let separator = &analysis.probes[0];
    assert_eq!(separator.worst_case_remaining_orders, 1);
    assert_eq!(separator.guaranteed_order_bits, 0.0);
    // Blackwell keeps the separator (strictly finer), but the order frontier
    // rejects it: the cheap null probe is order-equivalent and cheaper.
    assert!(
        analysis
            .blackwell_frontier
            .contains(&"separator".to_owned())
    );
    assert_eq!(analysis.order_information_frontier, vec!["null"]);
    assert_eq!(analysis.order_dominance[0].dominated, "separator");
}

#[test]
fn perfect_probe_on_two_equiprobable_orders_carries_exactly_one_bit() {
    let mut priors = BTreeMap::new();
    priors.insert("w-right".to_owned(), 1.0);
    priors.insert("w-left".to_owned(), 1.0);
    let model = ProbeModel {
        worlds: vec![
            world("w-right", PartialOrder::RightDominates),
            world("w-left", PartialOrder::LeftDominates),
        ],
        priors: Some(priors),
        probes: vec![probe(
            "perfect",
            dollars(1.0),
            &[("w-right", "improved"), ("w-left", "regressed")],
        )],
    };
    let analysis = analyze_model(&model).expect("valid model");
    let report = &analysis.probes[0];
    assert_eq!(
        report.order_outcome_mutual_information_bits,
        Some(1.0),
        "I(order; outcome) for a perfect binary probe under a uniform prior is exactly one bit"
    );
    assert_eq!(report.expected_remaining_orders, Some(1.0));
    let prior = analysis.prior.expect("normalization evidence");
    assert_eq!(prior.raw_mass, 2.0);
    assert_eq!(prior.weights, vec![0.5, 0.5]);
}

#[test]
fn unresolved_information_cost_tradeoff_keeps_both_probes() {
    let model = ProbeModel {
        worlds: vec![
            world("w-right", PartialOrder::RightDominates),
            world("w-left", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![
            probe(
                "informative-expensive",
                dollars(10.0),
                &[("w-right", "up"), ("w-left", "down")],
            ),
            probe(
                "blind-cheap",
                dollars(0.0),
                &[("w-right", "same"), ("w-left", "same")],
            ),
        ],
    };
    let analysis = analyze_model(&model).expect("valid model");
    assert_eq!(
        analysis.blackwell_frontier,
        vec!["informative-expensive", "blind-cheap"],
        "no exchange rate: information cannot buy cost and cost cannot buy information"
    );
    assert_eq!(
        analysis.order_information_frontier,
        vec!["informative-expensive", "blind-cheap"]
    );
    assert!(analysis.blackwell_dominance.is_empty());
    assert!(analysis.order_dominance.is_empty());
}

#[test]
fn partial_priors_are_rejected_not_filled_in() {
    let mut priors = BTreeMap::new();
    priors.insert("w-right".to_owned(), 1.0);
    let model = ProbeModel {
        worlds: vec![
            world("w-right", PartialOrder::RightDominates),
            world("w-left", PartialOrder::LeftDominates),
        ],
        priors: Some(priors),
        probes: vec![probe(
            "any",
            dollars(1.0),
            &[("w-right", "a"), ("w-left", "b")],
        )],
    };
    match analyze_model(&model) {
        Err(ProbeModelError::PartialPriors { world }) => assert_eq!(world, "w-left"),
        other => panic!("expected partial-prior rejection, got {other:?}"),
    }
}

#[test]
fn incomplete_observation_functions_are_rejected() {
    let model = ProbeModel {
        worlds: vec![
            world("w-right", PartialOrder::RightDominates),
            world("w-left", PartialOrder::LeftDominates),
        ],
        priors: None,
        probes: vec![probe("partial", dollars(1.0), &[("w-right", "a")])],
    };
    match analyze_model(&model) {
        Err(ProbeModelError::IncompleteObservations { probe, world }) => {
            assert_eq!(probe, "partial");
            assert_eq!(world, "w-left");
        }
        other => panic!("expected incomplete-observation rejection, got {other:?}"),
    }
}

#[test]
fn invalid_costs_and_unknown_worlds_are_rejected() {
    let negative_cost = ProbeModel {
        worlds: vec![world("w", PartialOrder::Equivalent)],
        priors: None,
        probes: vec![probe("bad", dollars(-1.0), &[("w", "a")])],
    };
    assert!(matches!(
        analyze_model(&negative_cost),
        Err(ProbeModelError::InvalidCost {
            component: "dollars",
            ..
        })
    ));

    let unknown_world = ProbeModel {
        worlds: vec![world("w", PartialOrder::Equivalent)],
        priors: None,
        probes: vec![probe(
            "ghost",
            dollars(1.0),
            &[("w", "a"), ("phantom", "b")],
        )],
    };
    assert!(matches!(
        analyze_model(&unknown_world),
        Err(ProbeModelError::UnknownObservedWorld { .. })
    ));
}

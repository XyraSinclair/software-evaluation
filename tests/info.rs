use approx::assert_abs_diff_eq;
use software_evaluation::info::{
    Budget, ClaimSpec, PlanError, PlanSpec, ProbeSpec, SelectionStrategy, binary_entropy_bits,
    metrics_for_probe, plan,
};

fn balanced_claim(id: &str) -> ClaimSpec {
    ClaimSpec {
        id: id.to_owned(),
        prior_probability: 0.5,
        false_positive_loss: 1.0,
        false_negative_loss: 1.0,
        importance: 1.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn probe(
    id: &str,
    claim_id: &str,
    sensitivity: f64,
    specificity: f64,
    expected_usd: f64,
    expected_seconds: f64,
    dependence_group: &str,
) -> ProbeSpec {
    ProbeSpec {
        id: id.to_owned(),
        claim_id: claim_id.to_owned(),
        sensitivity,
        specificity,
        expected_usd,
        expected_seconds,
        dependence_group: dependence_group.to_owned(),
    }
}

fn generous_budget(max_probes: usize) -> Budget {
    Budget {
        max_usd: 1_000.0,
        max_seconds: 1_000.0,
        max_probes,
    }
}

fn spec(claims: Vec<ClaimSpec>, probes: Vec<ProbeSpec>, budget: Budget) -> PlanSpec {
    PlanSpec {
        claims,
        probes,
        budget,
        strategy: SelectionStrategy::MaxInformation,
        conditional_independence_assumed: true,
    }
}

fn invalid_input_message(error: PlanError) -> String {
    match error {
        PlanError::InvalidInput(message) => message,
        other => panic!("expected invalid input, got {other}"),
    }
}

#[test]
fn binary_entropy_has_continuous_boundaries_and_one_bit_maximum() {
    assert_eq!(binary_entropy_bits(0.0), 0.0);
    assert_eq!(binary_entropy_bits(1.0), 0.0);
    assert_eq!(binary_entropy_bits(0.5), 1.0);
    assert_abs_diff_eq!(
        binary_entropy_bits(0.2),
        binary_entropy_bits(0.8),
        epsilon = 1e-15
    );
}

#[test]
fn binary_entropy_marks_out_of_domain_probabilities_as_nan() {
    for probability in [-f64::EPSILON, 1.0 + f64::EPSILON, f64::NAN, f64::INFINITY] {
        assert!(
            binary_entropy_bits(probability).is_nan(),
            "{probability:?} must not produce a plausible entropy"
        );
    }
}

#[test]
fn balanced_perfect_probe_matches_the_precommitted_oracle() {
    let claim = balanced_claim("claim");
    let probe = probe("perfect", "claim", 1.0, 1.0, 2.0, 4.0, "perfect");

    let metrics = metrics_for_probe(&claim, &probe).expect("valid calibrated probe");

    assert_eq!(metrics.probability_positive, 0.5);
    assert_eq!(metrics.posterior_if_positive, 1.0);
    assert_eq!(metrics.posterior_if_negative, 0.0);
    assert_eq!(metrics.information_gain_bits, 1.0);
    assert_eq!(metrics.expected_bayes_risk_reduction, 0.5);
    assert_eq!(metrics.decision_flip_probability, 0.5);
    assert_eq!(metrics.bits_per_usd, Some(0.5));
    assert_eq!(metrics.bits_per_second, Some(0.25));
}

#[test]
fn uninformative_probe_has_no_value_and_loses_to_the_no_probe_baseline() {
    let claim = balanced_claim("claim");
    let probe = probe("coin-flip", "claim", 0.5, 0.5, 1.0, 1.0, "coin");

    let metrics = metrics_for_probe(&claim, &probe).expect("valid calibrated probe");
    assert_eq!(metrics.posterior_if_positive, 0.5);
    assert_eq!(metrics.posterior_if_negative, 0.5);
    assert_eq!(metrics.information_gain_bits, 0.0);
    assert_eq!(metrics.expected_bayes_risk_reduction, 0.0);
    assert_eq!(metrics.decision_flip_probability, 0.0);

    let report = plan(&spec(vec![claim], vec![probe], generous_budget(1))).expect("valid plan");

    assert!(report.single_probe_frontier.is_empty());
    assert!(report.selected.is_empty());
    assert_eq!(report.stopped_reason, "no positive marginal value");
}

#[test]
fn inverted_perfect_probe_is_still_fully_informative() {
    let claim = balanced_claim("claim");
    let probe = probe("inverted", "claim", 0.0, 0.0, 1.0, 1.0, "inverted");

    let metrics = metrics_for_probe(&claim, &probe).expect("valid calibrated probe");

    assert_eq!(metrics.posterior_if_positive, 0.0);
    assert_eq!(metrics.posterior_if_negative, 1.0);
    assert_eq!(metrics.information_gain_bits, 1.0);
    assert_eq!(metrics.expected_bayes_risk_reduction, 0.5);
    assert_eq!(metrics.decision_flip_probability, 0.5);
}

#[test]
fn metric_inputs_reject_invalid_probabilities() {
    let cases = [
        {
            let mut claim = balanced_claim("claim");
            claim.prior_probability = -0.01;
            (
                "negative prior",
                claim,
                probe("p", "claim", 0.8, 0.8, 1.0, 1.0, "g"),
            )
        },
        {
            let mut claim = balanced_claim("claim");
            claim.prior_probability = f64::NAN;
            (
                "non-finite prior",
                claim,
                probe("p", "claim", 0.8, 0.8, 1.0, 1.0, "g"),
            )
        },
        (
            "sensitivity above one",
            balanced_claim("claim"),
            probe("p", "claim", 1.01, 0.8, 1.0, 1.0, "g"),
        ),
        (
            "non-finite specificity",
            balanced_claim("claim"),
            probe("p", "claim", 0.8, f64::INFINITY, 1.0, 1.0, "g"),
        ),
    ];

    for (name, claim, probe) in cases {
        let error = match metrics_for_probe(&claim, &probe) {
            Ok(_) => panic!("{name} unexpectedly passed"),
            Err(error) => error,
        };
        let message = invalid_input_message(error);
        assert!(
            message.contains("finite and in [0, 1]"),
            "{name} surfaced the wrong contract: {message}"
        );
    }
}

#[test]
fn planner_rejects_duplicate_claim_and_probe_ids() {
    let duplicate_claims = spec(
        vec![balanced_claim("same"), balanced_claim("same")],
        vec![probe("p", "same", 0.8, 0.8, 1.0, 1.0, "g")],
        generous_budget(1),
    );
    let message = invalid_input_message(plan(&duplicate_claims).unwrap_err());
    assert!(message.contains("duplicate claim id"));

    let duplicate_probes = spec(
        vec![balanced_claim("claim")],
        vec![
            probe("same", "claim", 0.8, 0.8, 1.0, 1.0, "g1"),
            probe("same", "claim", 0.7, 0.7, 1.0, 1.0, "g2"),
        ],
        generous_budget(2),
    );
    let message = invalid_input_message(plan(&duplicate_probes).unwrap_err());
    assert!(message.contains("duplicate probe id"));
}

#[test]
fn planner_rejects_unknown_claims_and_empty_model_sides() {
    let unknown_claim = spec(
        vec![balanced_claim("known")],
        vec![probe("p", "missing", 0.8, 0.8, 1.0, 1.0, "g")],
        generous_budget(1),
    );
    let message = invalid_input_message(plan(&unknown_claim).unwrap_err());
    assert!(message.contains("unknown claim"));

    let no_claims = spec(
        vec![],
        vec![probe("p", "missing", 0.8, 0.8, 1.0, 1.0, "g")],
        generous_budget(1),
    );
    let message = invalid_input_message(plan(&no_claims).unwrap_err());
    assert!(message.contains("claims must contain at least one claim"));

    let no_probes = spec(vec![balanced_claim("claim")], vec![], generous_budget(1));
    let message = invalid_input_message(plan(&no_probes).unwrap_err());
    assert!(message.contains("probes must contain at least one probe"));
}

#[test]
fn zero_cost_probe_has_no_bits_per_dollar_but_remains_selectable() {
    let claim = balanced_claim("claim");
    let free = probe("free", "claim", 1.0, 1.0, 0.0, 1.0, "free");
    let metrics = metrics_for_probe(&claim, &free).expect("valid free probe");
    assert_eq!(metrics.bits_per_usd, None);
    assert_eq!(metrics.information_gain_bits, 1.0);

    let mut plan_spec = spec(
        vec![claim],
        vec![free],
        Budget {
            max_usd: 0.0,
            max_seconds: 1.0,
            max_probes: 1,
        },
    );
    plan_spec.strategy = SelectionStrategy::InformationPerUsd;

    let report = plan(&plan_spec).expect("free probe fits a zero-dollar budget");
    assert_eq!(report.selected.len(), 1);
    assert_eq!(report.selected[0].probe_id, "free");
    assert_eq!(report.total_usd, 0.0);
}

#[test]
fn hard_dollar_time_and_probe_count_budgets_are_never_exceeded() {
    struct Case {
        name: &'static str,
        budget: Budget,
        expected_stop: &'static str,
    }

    let cases = [
        Case {
            name: "dollar ceiling",
            budget: Budget {
                max_usd: 2.0,
                max_seconds: 10.0,
                max_probes: 3,
            },
            expected_stop: "budget exhausted",
        },
        Case {
            name: "time ceiling",
            budget: Budget {
                max_usd: 10.0,
                max_seconds: 1.0,
                max_probes: 3,
            },
            expected_stop: "budget exhausted",
        },
        Case {
            name: "probe-count ceiling",
            budget: Budget {
                max_usd: 10.0,
                max_seconds: 10.0,
                max_probes: 1,
            },
            expected_stop: "max probes reached",
        },
    ];

    for case in cases {
        let claims = ["a", "b", "c"]
            .into_iter()
            .map(balanced_claim)
            .collect::<Vec<_>>();
        let probes = ["a", "b", "c"]
            .into_iter()
            .map(|id| probe(id, id, 1.0, 1.0, 2.0, 1.0, id))
            .collect::<Vec<_>>();
        let limits = case.budget.clone();
        let report = plan(&spec(claims, probes, case.budget))
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));

        assert!(report.total_usd <= limits.max_usd, "{}", case.name);
        assert!(report.total_seconds <= limits.max_seconds, "{}", case.name);
        assert!(report.selected.len() <= limits.max_probes, "{}", case.name);
        assert_eq!(report.selected.len(), 1, "{}", case.name);
        assert_eq!(report.stopped_reason, case.expected_stop, "{}", case.name);
    }
}

#[test]
fn probes_in_the_same_dependence_group_cannot_both_be_selected() {
    let report = plan(&spec(
        vec![balanced_claim("claim")],
        vec![
            probe("a", "claim", 0.75, 0.75, 0.0, 0.0, "shared"),
            probe("b", "claim", 0.75, 0.75, 0.0, 0.0, "shared"),
        ],
        generous_budget(2),
    ))
    .expect("valid dependent probes");

    assert_eq!(report.selected.len(), 1);
    assert_eq!(report.stopped_reason, "dependence constraint");
}

#[test]
fn declining_conditional_independence_limits_the_entire_plan_to_one_probe() {
    let mut plan_spec = spec(
        vec![balanced_claim("a"), balanced_claim("b")],
        vec![
            probe("probe-a", "a", 0.75, 0.75, 0.0, 0.0, "group-a"),
            probe("probe-b", "b", 0.75, 0.75, 0.0, 0.0, "group-b"),
        ],
        generous_budget(2),
    );
    plan_spec.conditional_independence_assumed = false;

    let report = plan(&plan_spec).expect("valid conservative plan");

    assert_eq!(report.selected.len(), 1);
    assert_eq!(report.stopped_reason, "dependence constraint");
    assert!(report.assumptions.iter().any(|assumption| {
        assumption.contains("Conditional independence is not assumed")
            && assumption.contains("at most one probe")
    }));
    assert!(
        !report
            .assumptions
            .iter()
            .any(|assumption| assumption.contains("conditionally independent given each claim"))
    );
}

#[test]
fn repeated_independent_probes_have_diminishing_marginal_information() {
    let report = plan(&spec(
        vec![balanced_claim("claim")],
        vec![
            probe("first", "claim", 0.75, 0.75, 0.0, 0.0, "group-1"),
            probe("second", "claim", 0.75, 0.75, 0.0, 0.0, "group-2"),
        ],
        generous_budget(2),
    ))
    .expect("valid independent probes");

    assert_eq!(report.selected.len(), 2);
    let first = report.selected[0].marginal_information_bits;
    let second = report.selected[1].marginal_information_bits;
    assert!(
        first > second,
        "expected diminishing information: {first} then {second}"
    );
    assert!(
        second > 0.0,
        "the independent repeat must still add information"
    );
    assert_abs_diff_eq!(
        report.total_information_bits,
        first + second,
        epsilon = 1e-14
    );
    assert!(report.total_information_bits < 2.0 * first);
    assert!(report.assumptions.iter().any(|assumption| {
        assumption.contains("conditionally independent given each claim")
            && assumption.contains("exact joint-information calculations")
    }));
    assert_eq!(report.stopped_reason, "max probes reached");
}

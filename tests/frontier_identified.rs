use std::path::PathBuf;

use software_evaluation::frontier::{
    AnalyzerEvidence, AnalyzerStatus, DirectionalCoverage, FrontierArtifact, FrontierConfig,
    FrontierProfile, FrontierSignal, PartialOrder, SignalFamily, SignalPolarity, SignalStatus,
};
use software_evaluation::frontier_identified::compare_profiles;
use software_evaluation::kernel::ArtifactSnapshot;

#[derive(Debug, Clone, Copy)]
struct SignalFixture {
    id: &'static str,
    family: &'static str,
    polarity: SignalPolarity,
    analyzer_id: &'static str,
    unit: &'static str,
    json_pointers: &'static [&'static str],
}

const SIGNALS: [SignalFixture; 6] = [
    SignalFixture {
        id: "reader.local-cognitive-p90",
        family: "reader-load",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: "shape",
        unit: "AST cognitive-complexity units",
        json_pointers: &[
            "/distributions/cognitive/p90",
            "/coverage/functions_analyzed",
        ],
    },
    SignalFixture {
        id: "reader.symbol-working-set-p90-fraction",
        family: "reader-load",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: "symbols",
        unit: "fraction of other resolved symbols",
        json_pointers: &[
            "/working_set_reachability/p90",
            "/working_set_reachability/nodes_in_distribution",
            "/graph/node_count",
            "/resolution/resolution_fraction",
        ],
    },
    SignalFixture {
        id: "interface.shallow-function-fraction",
        family: "interface-depth",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: "shape",
        unit: "fraction of analyzed functions",
        json_pointers: &[
            "/coverage/shallow_functions",
            "/coverage/shallow_denominator",
        ],
    },
    SignalFixture {
        id: "effects.syntactic-pure-fraction",
        family: "effect-locality",
        polarity: SignalPolarity::HigherIsBetter,
        analyzer_id: "discipline",
        unit: "fraction of analyzed functions",
        json_pointers: &["/coverage/pure_fraction", "/coverage/functions_total"],
    },
    SignalFixture {
        id: "effects.mutable-live-range-p90-lines",
        family: "effect-locality",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: "discipline",
        unit: "source lines",
        json_pointers: &[
            "/coverage/tails/mutable_live_range_lines_given_mutable/p90",
            "/coverage/functions_with_mutable_bindings",
        ],
    },
    SignalFixture {
        id: "uniformity.reported-clone-token-density",
        family: "uniformity",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: "duplicates",
        unit: "reported duplicated token mass / considered token",
        json_pointers: &[
            "/totals/duplicated_tokens",
            "/coverage/considered_tokens",
            "/totals/clone_groups",
            "/config/max_groups",
        ],
    },
];

#[test]
fn exact_intervals_recover_the_qualified_pareto_order() {
    let report = compare_profiles(profile("left", 0.6), profile("right", 0.5));

    assert_eq!(
        report.base.qualified_order,
        Some(PartialOrder::RightDominates)
    );
    assert!(report.readiness.qualified_identified_set);
    assert!(report.sharp_order_set.complete);
    assert_eq!(
        report.sharp_order_set.possible_orders,
        vec![PartialOrder::RightDominates]
    );
    assert_eq!(
        report.sharp_order_set.necessary_order,
        Some(PartialOrder::RightDominates)
    );
    assert_eq!(
        report.qualified_necessary_order,
        Some(PartialOrder::RightDominates)
    );
    assert!(report.sharp_order_set.right_necessarily_not_worse);
    assert!(!report.sharp_order_set.left_necessarily_not_worse);
}

#[test]
fn approximate_point_equivalence_matches_the_exact_comparator() {
    let left = profile("left", 0.5);
    let mut right = profile("right", 0.5);
    right.signals[0].value = Some(0.5 + 1e-12);

    let report = compare_profiles(left, right);

    assert_eq!(report.base.qualified_order, Some(PartialOrder::Equivalent));
    assert_eq!(
        report.sharp_order_set.possible_orders,
        vec![PartialOrder::Equivalent]
    );
    assert_eq!(
        report.qualified_necessary_order,
        Some(PartialOrder::Equivalent)
    );
}

#[test]
fn one_exact_regression_makes_tradeoff_necessary() {
    let left = profile("left", 0.6);
    let mut right = profile("right", 0.5);
    right.signals[0].value = Some(0.7);

    let report = compare_profiles(left, right);

    assert_eq!(
        report.sharp_order_set.possible_orders,
        vec![PartialOrder::Tradeoff]
    );
    assert_eq!(
        report.sharp_order_set.necessary_order,
        Some(PartialOrder::Tradeoff)
    );
    assert_eq!(
        report.qualified_necessary_order,
        Some(PartialOrder::Tradeoff)
    );
}

#[test]
fn a_censored_lower_bound_can_prove_a_regression() {
    let mut left = profile("left", 0.5);
    let mut right = profile("right", 0.5);
    left.signals[5].value = Some(0.1);
    right.signals[5].value = Some(0.2);
    mark_unusable(
        &mut right,
        5,
        SignalStatus::Censored,
        "fixture clone cap reached",
    );

    let report = compare_profiles(left, right);

    assert_eq!(report.base.qualified_order, None);
    assert!(report.readiness.qualified_identified_set);
    assert!(report.sharp_order_set.complete);
    assert_eq!(
        report.sharp_order_set.possible_orders,
        vec![PartialOrder::LeftDominates]
    );
    assert_eq!(
        report.sharp_order_set.necessary_order,
        Some(PartialOrder::LeftDominates)
    );
    assert_eq!(
        report.qualified_necessary_order,
        Some(PartialOrder::LeftDominates)
    );
    assert!(report.sharp_order_set.left_necessarily_not_worse);
}

#[test]
fn a_promising_censored_lower_bound_does_not_invent_improvement() {
    let mut left = profile("left", 0.5);
    let mut right = profile("right", 0.5);
    left.signals[5].value = Some(0.1);
    right.signals[5].value = Some(0.05);
    mark_unusable(
        &mut right,
        5,
        SignalStatus::Censored,
        "fixture clone cap reached",
    );

    let report = compare_profiles(left, right);

    assert!(report.readiness.qualified_identified_set);
    assert_eq!(
        report.sharp_order_set.possible_orders,
        vec![
            PartialOrder::RightDominates,
            PartialOrder::LeftDominates,
            PartialOrder::Equivalent,
        ]
    );
    assert_eq!(report.sharp_order_set.necessary_order, None);
    assert_eq!(report.qualified_necessary_order, None);
    assert!(!report.sharp_order_set.right_necessarily_not_worse);
    assert!(!report.sharp_order_set.left_necessarily_not_worse);
}

#[test]
fn low_symbol_coverage_yields_a_bounded_but_ambiguous_order_set() {
    let mut left = profile("left", 0.5);
    let mut right = profile("right", 0.5);
    left.signals[1].value = Some(0.4);
    right.signals[1].value = Some(0.3);
    mark_unusable(
        &mut left,
        1,
        SignalStatus::InsufficientCoverage,
        "fixture resolution gate",
    );
    mark_unusable(
        &mut right,
        1,
        SignalStatus::InsufficientCoverage,
        "fixture resolution gate",
    );

    let report = compare_profiles(left, right);

    assert!(report.readiness.qualified_identified_set);
    assert!(report.sharp_order_set.complete);
    assert_eq!(
        report.sharp_order_set.possible_orders,
        vec![
            PartialOrder::RightDominates,
            PartialOrder::LeftDominates,
            PartialOrder::Equivalent,
        ]
    );
    assert_eq!(report.sharp_order_set.necessary_order, None);
    assert_eq!(report.qualified_necessary_order, None);
}

#[test]
fn missing_or_malformed_registries_have_no_complete_identified_set() {
    let left = profile("left", 0.6);

    let mut missing = profile("right", 0.5);
    mark_unusable(
        &mut missing,
        0,
        SignalStatus::Missing,
        "fixture observation missing",
    );
    missing.signals[0].value = None;
    let report = compare_profiles(left.clone(), missing);
    assert!(!report.readiness.interval_surface_complete);
    assert!(!report.readiness.qualified_identified_set);
    assert!(!report.sharp_order_set.complete);
    assert!(report.sharp_order_set.possible_orders.is_empty());
    assert_eq!(report.qualified_necessary_order, None);

    let mut duplicate = profile("right", 0.5);
    duplicate.signals.push(duplicate.signals[0].clone());
    let report = compare_profiles(left, duplicate);
    assert!(!report.readiness.signal_registries_valid);
    assert!(!report.sharp_order_set.registry_valid);
    assert!(!report.sharp_order_set.complete);
    assert!(report.sharp_order_set.possible_orders.is_empty());
    assert_eq!(report.qualified_necessary_order, None);
}

#[test]
fn canonical_metadata_forgery_invalidates_the_identified_set() {
    let mut left = profile("left", 0.6);
    let mut right = profile("right", 0.5);
    left.signals[0].polarity = SignalPolarity::HigherIsBetter;
    right.signals[0].polarity = SignalPolarity::HigherIsBetter;

    let report = compare_profiles(left, right);

    assert!(!report.readiness.signal_registries_valid);
    assert!(!report.readiness.qualified_identified_set);
    assert!(!report.sharp_order_set.registry_valid);
    assert!(report.sharp_order_set.possible_orders.is_empty());
    assert_eq!(report.qualified_necessary_order, None);
}

#[test]
fn unique_interval_order_is_not_qualified_across_provenance_drift() {
    let left = profile("left", 0.6);

    let mut config_drift = profile("right", 0.5);
    config_drift.config.duplicate_min_tokens += 1;
    let report = compare_profiles(left.clone(), config_drift);
    assert_eq!(
        report.sharp_order_set.necessary_order,
        Some(PartialOrder::RightDominates)
    );
    assert!(!report.readiness.analysis_config_compatible);
    assert!(!report.readiness.qualified_identified_set);
    assert_eq!(report.qualified_necessary_order, None);

    let mut unpinned = profile("right", 0.5);
    unpinned.artifact.git = None;
    unpinned.artifact.identity_error = Some("fixture unpinned".to_owned());
    let report = compare_profiles(left, unpinned);
    assert_eq!(
        report.sharp_order_set.necessary_order,
        Some(PartialOrder::RightDominates)
    );
    assert!(!report.readiness.artifacts_commit_pinned);
    assert!(!report.readiness.qualified_identified_set);
    assert_eq!(report.qualified_necessary_order, None);
}

fn mark_unusable(profile: &mut FrontierProfile, index: usize, status: SignalStatus, reason: &str) {
    let signal = &mut profile.signals[index];
    signal.status = status;
    signal.unavailable_reason = Some(reason.to_owned());
    profile.coverage.observed = profile
        .signals
        .iter()
        .filter(|signal| signal.status == SignalStatus::Observed)
        .count();
    profile.coverage.unusable_signal_ids = profile
        .signals
        .iter()
        .filter(|signal| signal.status != SignalStatus::Observed)
        .map(|signal| signal.id.clone())
        .collect();
}

fn profile(name: &str, lower_value: f64) -> FrontierProfile {
    let signals = SIGNALS
        .iter()
        .map(|signal| FrontierSignal {
            id: signal.id.to_owned(),
            family: signal.family.to_owned(),
            label: signal.id.to_owned(),
            polarity: signal.polarity,
            status: SignalStatus::Observed,
            value: Some(match signal.polarity {
                SignalPolarity::LowerIsBetter => lower_value,
                SignalPolarity::HigherIsBetter => 1.0 - lower_value,
            }),
            numerator: None,
            denominator: Some(1.0),
            unit: signal.unit.to_owned(),
            analyzer_id: signal.analyzer_id.to_owned(),
            json_pointers: signal
                .json_pointers
                .iter()
                .map(|pointer| (*pointer).to_owned())
                .collect(),
            note: "fixture".to_owned(),
            unavailable_reason: None,
        })
        .collect();
    let (revision_digit, tree_digit) = if name == "left" {
        ('1', '3')
    } else {
        ('2', '4')
    };

    FrontierProfile {
        schema_version: "seval.frontier.v1".to_owned(),
        artifact: FrontierArtifact {
            input: name.to_owned(),
            git: Some(ArtifactSnapshot {
                id: format!("fixture:{name}"),
                root: PathBuf::from(format!("/{name}")),
                revision: revision_digit.to_string().repeat(40),
                tree_digest: tree_digit.to_string().repeat(40),
                kind: "git-repository".to_owned(),
            }),
            identity_error: None,
        },
        config: FrontierConfig::default(),
        elapsed_ms: 0,
        analyzers: ["shape", "symbols", "discipline", "duplicates"]
            .into_iter()
            .map(|id| AnalyzerEvidence {
                id: id.to_owned(),
                status: AnalyzerStatus::Complete,
                implementation: Some(format!("{id}.v1")),
                elapsed_ms: 0,
                payload_sha256: Some("00".repeat(32)),
                coverage: Some(serde_json::json!({"fixture": true})),
                limitations: Vec::new(),
                error: None,
            })
            .collect(),
        families: vec![
            family("reader-load", &SIGNALS[..2]),
            family("interface-depth", &SIGNALS[2..3]),
            family("effect-locality", &SIGNALS[3..5]),
            family("uniformity", &SIGNALS[5..]),
        ],
        coverage: DirectionalCoverage {
            declared: SIGNALS.len(),
            observed: SIGNALS.len(),
            unusable_signal_ids: Vec::new(),
        },
        signals,
        limitations: Vec::new(),
    }
}

fn family(id: &str, signals: &[SignalFixture]) -> SignalFamily {
    SignalFamily {
        id: id.to_owned(),
        label: id.to_owned(),
        signal_ids: signals.iter().map(|signal| signal.id.to_owned()).collect(),
        rule: "fixture".to_owned(),
    }
}

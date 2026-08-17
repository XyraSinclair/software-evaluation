use std::path::PathBuf;

use software_evaluation::frontier::{
    AnalyzerEvidence, AnalyzerStatus, DirectionalCoverage, FrontierArtifact, FrontierConfig,
    FrontierProfile, FrontierSignal, PartialOrder, SignalFamily, SignalOutcome, SignalPolarity,
    SignalStatus, compare_profiles,
};
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
            "/coverage/tails/max_mutable_live_range_lines/p90",
            "/coverage/functions_total",
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
fn complete_compatible_profiles_emit_a_qualified_pareto_order() {
    let left = profile("left", 0.6);
    let right = profile("right", 0.5);

    let comparison = compare_profiles(left, right);

    assert!(comparison.readiness.qualified);
    assert_eq!(
        comparison.qualified_order,
        Some(PartialOrder::RightDominates)
    );
    assert_eq!(
        comparison.order_on_observed_intersection,
        PartialOrder::RightDominates
    );
    assert!(comparison.readiness.blockers.is_empty());
}

#[test]
fn one_regression_forces_tradeoff_instead_of_being_outvoted() {
    let left = profile("left", 0.6);
    let mut right = profile("right", 0.5);
    right.signals[0].value = Some(0.7);

    let comparison = compare_profiles(left, right);

    assert_eq!(comparison.qualified_order, Some(PartialOrder::Tradeoff));
    assert_eq!(
        comparison.order_on_observed_intersection,
        PartialOrder::Tradeoff
    );
}

#[test]
fn missing_evidence_fails_closed_without_hiding_observed_deltas() {
    let left = profile("left", 0.6);

    let mut unpinned = profile("right", 0.5);
    unpinned.artifact.git = None;
    unpinned.artifact.identity_error = Some("fixture is unpinned".to_owned());
    let comparison = compare_profiles(left.clone(), unpinned);
    assert!(!comparison.readiness.qualified);
    assert_eq!(comparison.qualified_order, None);
    assert_eq!(
        comparison.order_on_observed_intersection,
        PartialOrder::RightDominates
    );
    assert!(!comparison.readiness.artifacts_commit_pinned);

    let mut unstable_identity = profile("right", 0.5);
    unstable_identity.artifact.identity_error = Some("fixture changed during scan".to_owned());
    let comparison = compare_profiles(left.clone(), unstable_identity);
    assert!(!comparison.readiness.artifacts_commit_pinned);
    assert_eq!(comparison.qualified_order, None);

    let mut malformed_identity = profile("right", 0.5);
    malformed_identity.artifact.git.as_mut().unwrap().kind = "git".to_owned();
    let comparison = compare_profiles(left.clone(), malformed_identity);
    assert!(!comparison.readiness.artifacts_commit_pinned);
    assert_eq!(comparison.qualified_order, None);

    let mut censored = profile("right", 0.5);
    censored.signals[5].status = SignalStatus::Censored;
    censored.signals[5].unavailable_reason = Some("fixture cap reached".to_owned());
    censored.coverage.observed -= 1;
    censored.coverage.unusable_signal_ids = vec![censored.signals[5].id.clone()];
    let comparison = compare_profiles(left.clone(), censored);
    assert!(!comparison.readiness.directional_signals_complete);
    assert_eq!(comparison.qualified_order, None);
    assert_eq!(
        comparison.order_on_observed_intersection,
        PartialOrder::RightDominates
    );

    let mut non_finite = profile("right", 0.5);
    non_finite.signals[0].value = Some(f64::NAN);
    let comparison = compare_profiles(left, non_finite);
    assert!(!comparison.readiness.directional_signals_complete);
    assert_eq!(comparison.qualified_order, None);
    assert_eq!(
        comparison
            .signals
            .iter()
            .find(|signal| signal.id == "reader.local-cognitive-p90")
            .map(|signal| signal.outcome),
        Some(SignalOutcome::Unavailable)
    );
}

#[test]
fn malformed_evidence_registries_and_configurations_never_qualify() {
    let left = profile("left", 0.6);

    let mut implementation_drift = profile("right", 0.5);
    implementation_drift.analyzers[0].implementation = Some("shape.v2".to_owned());
    let comparison = compare_profiles(left.clone(), implementation_drift);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut failed_evidence = profile("right", 0.5);
    failed_evidence.analyzers[0].status = AnalyzerStatus::Failed;
    failed_evidence.analyzers[0].error = Some("fixture analyzer failure".to_owned());
    let comparison = compare_profiles(left.clone(), failed_evidence);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut invalid_digest = profile("right", 0.5);
    invalid_digest.analyzers[0].payload_sha256 = Some("not-a-digest".to_owned());
    let comparison = compare_profiles(left.clone(), invalid_digest);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut absent_coverage = profile("right", 0.5);
    absent_coverage.analyzers[0].coverage = None;
    let comparison = compare_profiles(left.clone(), absent_coverage);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut duplicate_evidence = profile("right", 0.5);
    duplicate_evidence
        .analyzers
        .push(duplicate_evidence.analyzers[0].clone());
    let comparison = compare_profiles(left.clone(), duplicate_evidence);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut duplicate_signal = profile("right", 0.5);
    duplicate_signal
        .signals
        .push(duplicate_signal.signals[0].clone());
    let comparison = compare_profiles(left.clone(), duplicate_signal);
    assert!(!comparison.readiness.directional_signals_complete);
    assert_eq!(comparison.qualified_order, None);

    let mut config_drift = profile("right", 0.5);
    config_drift.config.min_symbol_resolution_fraction =
        f64::from_bits(config_drift.config.min_symbol_resolution_fraction.to_bits() + 1);
    let comparison = compare_profiles(left.clone(), config_drift);
    assert!(!comparison.readiness.analysis_config_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut invalid_config = profile("right", 0.5);
    invalid_config.config.duplicate_max_groups = 0;
    let mut invalid_left = left;
    invalid_left.config.duplicate_max_groups = 0;
    let comparison = compare_profiles(invalid_left, invalid_config);
    assert!(!comparison.readiness.analysis_config_compatible);
    assert_eq!(comparison.qualified_order, None);
}

#[test]
fn mutually_consistent_contract_forgery_still_fails_qualification() {
    let mut left = profile("left", 0.6);
    let mut right = profile("right", 0.5);
    left.signals[0].polarity = SignalPolarity::HigherIsBetter;
    right.signals[0].polarity = SignalPolarity::HigherIsBetter;

    let comparison = compare_profiles(left, right);

    assert!(!comparison.readiness.directional_signals_complete);
    assert_eq!(comparison.qualified_order, None);
    assert!(
        comparison
            .readiness
            .blockers
            .iter()
            .any(|blocker| blocker.contains("preregistered signal"))
    );
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

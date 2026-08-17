use std::path::PathBuf;

use software_evaluation::frontier::{
    AnalyzerReceipt, AnalyzerStatus, DirectionalCoverage, FrontierArtifact, FrontierConfig,
    FrontierProfile, FrontierSignal, PartialOrder, SignalFamily, SignalOutcome, SignalPolarity,
    SignalStatus, compare_profiles,
};
use software_evaluation::kernel::ArtifactSnapshot;

const SIGNALS: [(&str, &str, SignalPolarity); 6] = [
    (
        "reader.local-cognitive-p90",
        "reader-load",
        SignalPolarity::LowerIsBetter,
    ),
    (
        "reader.symbol-working-set-p90-fraction",
        "reader-load",
        SignalPolarity::LowerIsBetter,
    ),
    (
        "interface.shallow-function-fraction",
        "interface-depth",
        SignalPolarity::LowerIsBetter,
    ),
    (
        "effects.syntactic-pure-fraction",
        "effect-locality",
        SignalPolarity::HigherIsBetter,
    ),
    (
        "effects.mutable-live-range-p90-lines",
        "effect-locality",
        SignalPolarity::LowerIsBetter,
    ),
    (
        "uniformity.reported-clone-token-density",
        "uniformity",
        SignalPolarity::LowerIsBetter,
    ),
];

#[test]
fn complete_compatible_profiles_emit_a_qualified_pareto_order() {
    let left = profile("left", 1.0);
    let right = profile("right", 0.9);

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
    let left = profile("left", 1.0);
    let mut right = profile("right", 0.9);
    right.signals[0].value = Some(1.1);

    let comparison = compare_profiles(left, right);

    assert_eq!(comparison.qualified_order, Some(PartialOrder::Tradeoff));
    assert_eq!(
        comparison.order_on_observed_intersection,
        PartialOrder::Tradeoff
    );
}

#[test]
fn missing_evidence_fails_closed_without_hiding_observed_deltas() {
    let left = profile("left", 1.0);

    let mut unpinned = profile("right", 0.9);
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

    let mut unstable_identity = profile("right", 0.9);
    unstable_identity.artifact.identity_error = Some("fixture changed during scan".to_owned());
    let comparison = compare_profiles(left.clone(), unstable_identity);
    assert!(!comparison.readiness.artifacts_commit_pinned);
    assert_eq!(comparison.qualified_order, None);

    let mut censored = profile("right", 0.9);
    censored.signals[5].status = SignalStatus::Censored;
    censored.signals[5].unavailable_reason = Some("fixture cap reached".to_owned());
    let comparison = compare_profiles(left.clone(), censored);
    assert!(!comparison.readiness.directional_signals_complete);
    assert_eq!(comparison.qualified_order, None);
    assert_eq!(
        comparison.order_on_observed_intersection,
        PartialOrder::RightDominates
    );

    let mut non_finite = profile("right", 0.9);
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
fn malformed_receipts_registries_and_configurations_never_qualify() {
    let left = profile("left", 1.0);

    let mut implementation_drift = profile("right", 0.9);
    implementation_drift.analyzers[0].implementation = Some("shape.v2".to_owned());
    let comparison = compare_profiles(left.clone(), implementation_drift);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut failed_receipt = profile("right", 0.9);
    failed_receipt.analyzers[0].status = AnalyzerStatus::Failed;
    failed_receipt.analyzers[0].error = Some("fixture analyzer failure".to_owned());
    let comparison = compare_profiles(left.clone(), failed_receipt);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut invalid_digest = profile("right", 0.9);
    invalid_digest.analyzers[0].payload_sha256 = Some("not-a-digest".to_owned());
    let comparison = compare_profiles(left.clone(), invalid_digest);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut duplicate_receipt = profile("right", 0.9);
    duplicate_receipt
        .analyzers
        .push(duplicate_receipt.analyzers[0].clone());
    let comparison = compare_profiles(left.clone(), duplicate_receipt);
    assert!(!comparison.readiness.analyzer_implementations_compatible);
    assert_eq!(comparison.qualified_order, None);

    let mut duplicate_signal = profile("right", 0.9);
    duplicate_signal
        .signals
        .push(duplicate_signal.signals[0].clone());
    let comparison = compare_profiles(left.clone(), duplicate_signal);
    assert!(!comparison.readiness.directional_signals_complete);
    assert_eq!(comparison.qualified_order, None);

    let mut config_drift = profile("right", 0.9);
    config_drift.config.min_symbol_resolution_fraction =
        f64::from_bits(config_drift.config.min_symbol_resolution_fraction.to_bits() + 1);
    let comparison = compare_profiles(left, config_drift);
    assert!(!comparison.readiness.analysis_config_compatible);
    assert_eq!(comparison.qualified_order, None);
}

fn profile(name: &str, lower_value: f64) -> FrontierProfile {
    let signals = SIGNALS
        .iter()
        .map(|(id, family, polarity)| FrontierSignal {
            id: (*id).to_owned(),
            family: (*family).to_owned(),
            label: (*id).to_owned(),
            polarity: *polarity,
            status: SignalStatus::Observed,
            value: Some(match polarity {
                SignalPolarity::LowerIsBetter => lower_value,
                SignalPolarity::HigherIsBetter => 2.0 - lower_value,
            }),
            numerator: None,
            denominator: Some(1.0),
            unit: "fixture-unit".to_owned(),
            analyzer_id: "fixture".to_owned(),
            json_pointers: vec!["/fixture".to_owned()],
            note: "fixture".to_owned(),
            unavailable_reason: None,
        })
        .collect();

    FrontierProfile {
        schema_version: "seval.frontier.v1".to_owned(),
        artifact: FrontierArtifact {
            input: name.to_owned(),
            git: Some(ArtifactSnapshot {
                id: format!("fixture:{name}"),
                root: PathBuf::from(format!("/{name}")),
                revision: format!("{name:0<40}"),
                tree_digest: format!("{name:0<64}"),
                kind: "git".to_owned(),
            }),
            identity_error: None,
        },
        config: FrontierConfig::default(),
        elapsed_ms: 0,
        analyzers: ["shape", "symbols", "discipline", "duplicates"]
            .into_iter()
            .map(|id| AnalyzerReceipt {
                id: id.to_owned(),
                status: AnalyzerStatus::Complete,
                implementation: Some(format!("{id}.v1")),
                elapsed_ms: 0,
                payload_sha256: Some("00".repeat(32)),
                coverage: None,
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

fn family(id: &str, signals: &[(&str, &str, SignalPolarity)]) -> SignalFamily {
    SignalFamily {
        id: id.to_owned(),
        label: id.to_owned(),
        signal_ids: signals
            .iter()
            .map(|(signal_id, _, _)| (*signal_id).to_owned())
            .collect(),
        rule: "fixture".to_owned(),
    }
}

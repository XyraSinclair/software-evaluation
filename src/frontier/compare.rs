use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;

struct SignalContract {
    id: &'static str,
    family: &'static str,
    polarity: SignalPolarity,
    analyzer_id: &'static str,
    unit: &'static str,
    json_pointers: &'static [&'static str],
    bounded_by_one: bool,
}

const SIGNAL_CONTRACTS: [SignalContract; 6] = [
    SignalContract {
        id: "reader.local-cognitive-p90",
        family: "reader-load",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: SHAPE,
        unit: "AST cognitive-complexity units",
        json_pointers: &[
            "/distributions/cognitive/p90",
            "/coverage/functions_analyzed",
        ],
        bounded_by_one: false,
    },
    SignalContract {
        id: "reader.symbol-working-set-p90-fraction",
        family: "reader-load",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: SYMBOLS,
        unit: "fraction of other resolved symbols",
        json_pointers: &[
            "/working_set_reachability/p90",
            "/working_set_reachability/nodes_in_distribution",
            "/graph/node_count",
            "/resolution/resolution_fraction",
        ],
        bounded_by_one: true,
    },
    SignalContract {
        id: "interface.shallow-function-fraction",
        family: "interface-depth",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: SHAPE,
        unit: "fraction of analyzed functions",
        json_pointers: &[
            "/coverage/shallow_functions",
            "/coverage/shallow_denominator",
        ],
        bounded_by_one: true,
    },
    SignalContract {
        id: "effects.syntactic-pure-fraction",
        family: "effect-locality",
        polarity: SignalPolarity::HigherIsBetter,
        analyzer_id: DISCIPLINE,
        unit: "fraction of analyzed functions",
        json_pointers: &["/coverage/pure_fraction", "/coverage/functions_total"],
        bounded_by_one: true,
    },
    SignalContract {
        id: "effects.mutable-live-range-p90-lines",
        family: "effect-locality",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: DISCIPLINE,
        unit: "source lines",
        json_pointers: &[
            "/coverage/tails/max_mutable_live_range_lines/p90",
            "/coverage/functions_total",
        ],
        bounded_by_one: false,
    },
    SignalContract {
        id: "uniformity.reported-clone-token-density",
        family: "uniformity",
        polarity: SignalPolarity::LowerIsBetter,
        analyzer_id: DUPLICATES,
        unit: "reported duplicated token mass / considered token",
        json_pointers: &[
            "/totals/duplicated_tokens",
            "/coverage/considered_tokens",
            "/totals/clone_groups",
            "/config/max_groups",
        ],
        bounded_by_one: false,
    },
];

const FAMILY_CONTRACTS: [(&str, &[&str]); 4] = [
    (
        "reader-load",
        &[
            "reader.local-cognitive-p90",
            "reader.symbol-working-set-p90-fraction",
        ],
    ),
    (
        "interface-depth",
        &["interface.shallow-function-fraction"],
    ),
    (
        "effect-locality",
        &[
            "effects.syntactic-pure-fraction",
            "effects.mutable-live-range-p90-lines",
        ],
    ),
    (
        "uniformity",
        &["uniformity.reported-clone-token-density"],
    ),
];

pub fn compare_paths(
    left: &Path,
    right: &Path,
    config: &FrontierConfig,
) -> Result<FrontierComparison, FrontierError> {
    Ok(compare_profiles(
        profile_path(left, config)?,
        profile_path(right, config)?,
    ))
}

#[must_use]
pub fn compare_profiles(left: FrontierProfile, right: FrontierProfile) -> FrontierComparison {
    let signals = compare_signal_sets(&left.signals, &right.signals);
    let overall = comparison_slice(&signals, &SIGNAL_IDS);
    let schema_compatible = left.schema_version == right.schema_version
        && left.schema_version == FRONTIER_SCHEMA_VERSION;
    let analysis_config_compatible = compatible_config(&left.config, &right.config);
    let analyzer_implementations_compatible = compatible_implementations(&left, &right);
    let signal_registries_valid = valid_signal_registry(&left) && valid_signal_registry(&right);
    let directional_signals_complete = overall.complete && signal_registries_valid;
    let artifacts_commit_pinned = artifact_is_pinned(&left) && artifact_is_pinned(&right);
    let qualified = schema_compatible
        && analysis_config_compatible
        && analyzer_implementations_compatible
        && directional_signals_complete
        && artifacts_commit_pinned;
    let mut blockers = Vec::new();
    if !schema_compatible {
        blockers.push("frontier schema versions differ or are unsupported".to_owned());
    }
    if !analysis_config_compatible {
        blockers.push("analysis-affecting configurations differ".to_owned());
    }
    if !analyzer_implementations_compatible {
        blockers.push(
            "underlying analyzer receipts are missing, duplicated, failed, undigested, unnamed, or implementation-incompatible"
                .to_owned(),
        );
    }
    if !overall.complete {
        blockers.push(format!(
            "directional surface is incomplete: {}",
            overall.unusable_signal_ids.join(", ")
        ));
    }
    if !signal_registries_valid {
        blockers.push(
            "each profile must exactly satisfy the preregistered signal, family, polarity, unit, analyzer, projection, status, value-domain, and coverage-ledger contract"
                .to_owned(),
        );
    }
    if !artifacts_commit_pinned {
        blockers.push("both scans must remain clean and commit-pinned".to_owned());
    }
    let families = left
        .families
        .iter()
        .map(|family| {
            let ids = family
                .signal_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            FamilyComparison {
                id: family.id.clone(),
                label: family.label.clone(),
                comparison: comparison_slice(&signals, &ids),
            }
        })
        .collect();

    FrontierComparison {
        schema_version: FRONTIER_SCHEMA_VERSION.to_owned(),
        left,
        right,
        readiness: ComparisonReadiness {
            schema_compatible,
            analysis_config_compatible,
            analyzer_implementations_compatible,
            directional_signals_complete,
            artifacts_commit_pinned,
            qualified,
            blockers,
        },
        order_on_observed_intersection: overall.order_on_observed_intersection,
        qualified_order: qualified.then_some(overall.order_on_observed_intersection),
        families,
        signals,
        limitations: vec![
            "Dominance is strict Pareto dominance over six mechanical proxies, not proof of correctness, security, fitness, maintainability, or overall value.".to_owned(),
            "Equivalent means numerically equal within floating-point noise, not practically equivalent.".to_owned(),
            "Any missing, non-finite, censored, failed, or coverage-gated signal removes the qualified order rather than shrinking the denominator silently.".to_owned(),
        ],
    }
}

fn artifact_is_pinned(profile: &FrontierProfile) -> bool {
    profile.artifact.git.is_some() && profile.artifact.identity_error.is_none()
}

fn valid_signal_registry(profile: &FrontierProfile) -> bool {
    profile.signals.len() == SIGNAL_CONTRACTS.len()
        && SIGNAL_CONTRACTS.iter().all(|contract| {
            let mut matches = profile
                .signals
                .iter()
                .filter(|signal| signal.id == contract.id);
            let Some(signal) = matches.next() else {
                return false;
            };
            matches.next().is_none() && valid_signal_contract(signal, contract)
        })
        && valid_family_registry(profile)
        && valid_coverage_ledger(profile)
}

fn valid_signal_contract(signal: &FrontierSignal, contract: &SignalContract) -> bool {
    if signal.id != contract.id
        || signal.family != contract.family
        || signal.polarity != contract.polarity
        || signal.analyzer_id != contract.analyzer_id
        || signal.unit != contract.unit
        || !signal
            .json_pointers
            .iter()
            .map(String::as_str)
            .eq(contract.json_pointers.iter().copied())
    {
        return false;
    }
    if [signal.value, signal.numerator, signal.denominator]
        .into_iter()
        .flatten()
        .any(|value| !value.is_finite() || value < 0.0)
    {
        return false;
    }
    if contract.bounded_by_one && signal.value.is_some_and(|value| value > 1.0) {
        return false;
    }

    let has_reason = signal
        .unavailable_reason
        .as_deref()
        .is_some_and(|reason| !reason.trim().is_empty());
    match signal.status {
        SignalStatus::Observed => {
            signal.value.is_some_and(f64::is_finite)
                && signal.denominator.is_some_and(|value| value > 0.0)
                && signal.unavailable_reason.is_none()
        }
        SignalStatus::Censored => {
            signal.id == "uniformity.reported-clone-token-density"
                && signal.value.is_some()
                && signal.denominator.is_some_and(|value| value > 0.0)
                && has_reason
        }
        SignalStatus::InsufficientCoverage => {
            signal.id == "reader.symbol-working-set-p90-fraction"
                && signal.value.is_some()
                && signal.denominator.is_some_and(|value| value > 0.0)
                && has_reason
        }
        SignalStatus::Missing => has_reason,
        SignalStatus::SourceFailed => {
            signal.value.is_none()
                && signal.numerator.is_none()
                && signal.denominator.is_none()
                && has_reason
        }
    }
}

fn valid_family_registry(profile: &FrontierProfile) -> bool {
    profile.families.len() == FAMILY_CONTRACTS.len()
        && FAMILY_CONTRACTS.iter().all(|(id, expected_signals)| {
            let mut matches = profile.families.iter().filter(|family| family.id == *id);
            let Some(family) = matches.next() else {
                return false;
            };
            matches.next().is_none()
                && family
                    .signal_ids
                    .iter()
                    .map(String::as_str)
                    .eq(expected_signals.iter().copied())
        })
}

fn valid_coverage_ledger(profile: &FrontierProfile) -> bool {
    let observed = profile
        .signals
        .iter()
        .filter(|signal| signal.status == SignalStatus::Observed)
        .count();
    let unusable = profile
        .signals
        .iter()
        .filter(|signal| signal.status != SignalStatus::Observed)
        .map(|signal| signal.id.as_str())
        .collect::<BTreeSet<_>>();
    let reported_unusable = profile
        .coverage
        .unusable_signal_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    profile.coverage.declared == SIGNAL_CONTRACTS.len()
        && profile.coverage.observed == observed
        && unusable == reported_unusable
}

fn compatible_implementations(left: &FrontierProfile, right: &FrontierProfile) -> bool {
    [SHAPE, SYMBOLS, DISCIPLINE, DUPLICATES]
        .into_iter()
        .all(|analyzer_id| {
            let left = unique_complete_implementation(left, analyzer_id);
            let right = unique_complete_implementation(right, analyzer_id);
            matches!((left, right), (Some(left), Some(right)) if left == right)
        })
}

fn unique_complete_implementation<'a>(
    profile: &'a FrontierProfile,
    analyzer_id: &str,
) -> Option<&'a str> {
    let mut matches = profile
        .analyzers
        .iter()
        .filter(|receipt| receipt.id == analyzer_id);
    let receipt = matches.next()?;
    if matches.next().is_some()
        || receipt.status != AnalyzerStatus::Complete
        || receipt.error.is_some()
        || !receipt
            .payload_sha256
            .as_deref()
            .is_some_and(valid_sha256_hex)
    {
        return None;
    }
    receipt.implementation.as_deref()
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn compatible_config(left: &FrontierConfig, right: &FrontierConfig) -> bool {
    left == right
}

fn compare_signal_sets(left: &[FrontierSignal], right: &[FrontierSignal]) -> Vec<SignalComparison> {
    let left = left
        .iter()
        .map(|signal| (signal.id.as_str(), signal))
        .collect::<BTreeMap<_, _>>();
    let right = right
        .iter()
        .map(|signal| (signal.id.as_str(), signal))
        .collect::<BTreeMap<_, _>>();
    left.keys()
        .chain(right.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| match (left.get(id), right.get(id)) {
            (Some(left), Some(right)) => compare_signal(left, right),
            (Some(signal), None) => missing_side(signal, true),
            (None, Some(signal)) => missing_side(signal, false),
            (None, None) => unreachable!("id came from at least one profile"),
        })
        .collect()
}

fn compare_signal(left: &FrontierSignal, right: &FrontierSignal) -> SignalComparison {
    let compatible = left.family == right.family
        && left.polarity == right.polarity
        && left.unit == right.unit
        && left.analyzer_id == right.analyzer_id
        && left.json_pointers == right.json_pointers;
    let usable = left.status == SignalStatus::Observed
        && right.status == SignalStatus::Observed
        && left.value.is_some_and(f64::is_finite)
        && right.value.is_some_and(f64::is_finite);
    let delta = if usable {
        left.value
            .zip(right.value)
            .map(|(left, right)| right - left)
    } else {
        None
    };
    let (outcome, reason) = if !compatible {
        (
            SignalOutcome::Incompatible,
            Some("signal family, polarity, unit, analyzer, or projection differs".to_owned()),
        )
    } else if !usable {
        (
            SignalOutcome::Unavailable,
            Some("one or both observations are absent, non-finite, or unusable".to_owned()),
        )
    } else {
        let (Some(left_value), Some(right_value)) = (left.value, right.value) else {
            unreachable!("usable values were checked above");
        };
        let outcome = if approximately_equal(left_value, right_value) {
            SignalOutcome::Equivalent
        } else {
            match left.polarity {
                SignalPolarity::LowerIsBetter if right_value < left_value => {
                    SignalOutcome::RightBetter
                }
                SignalPolarity::LowerIsBetter => SignalOutcome::LeftBetter,
                SignalPolarity::HigherIsBetter if right_value > left_value => {
                    SignalOutcome::RightBetter
                }
                SignalPolarity::HigherIsBetter => SignalOutcome::LeftBetter,
            }
        };
        (outcome, None)
    };
    SignalComparison {
        id: left.id.clone(),
        family: left.family.clone(),
        label: left.label.clone(),
        polarity: left.polarity,
        left_status: left.status,
        right_status: right.status,
        left_value: left.value,
        right_value: right.value,
        right_minus_left: delta,
        outcome,
        reason,
    }
}

fn missing_side(signal: &FrontierSignal, missing_right: bool) -> SignalComparison {
    SignalComparison {
        id: signal.id.clone(),
        family: signal.family.clone(),
        label: signal.label.clone(),
        polarity: signal.polarity,
        left_status: if missing_right {
            signal.status
        } else {
            SignalStatus::Missing
        },
        right_status: if missing_right {
            SignalStatus::Missing
        } else {
            signal.status
        },
        left_value: if missing_right { signal.value } else { None },
        right_value: if missing_right { None } else { signal.value },
        right_minus_left: None,
        outcome: SignalOutcome::Incompatible,
        reason: Some("signal is absent from one profile".to_owned()),
    }
}

fn comparison_slice(comparisons: &[SignalComparison], ids: &[&str]) -> ComparisonSlice {
    let by_id = comparisons
        .iter()
        .map(|comparison| (comparison.id.as_str(), comparison))
        .collect::<BTreeMap<_, _>>();
    let mut outcomes = Vec::new();
    let mut unusable = Vec::new();
    for id in ids {
        match by_id.get(id).map(|comparison| comparison.outcome) {
            Some(
                outcome @ (SignalOutcome::RightBetter
                | SignalOutcome::LeftBetter
                | SignalOutcome::Equivalent),
            ) => outcomes.push(outcome),
            _ => unusable.push((*id).to_owned()),
        }
    }
    ComparisonSlice {
        complete: unusable.is_empty(),
        comparable_signals: outcomes.len(),
        unusable_signal_ids: unusable,
        order_on_observed_intersection: partial_order(&outcomes),
    }
}

fn partial_order(outcomes: &[SignalOutcome]) -> PartialOrder {
    let right = outcomes.contains(&SignalOutcome::RightBetter);
    let left = outcomes.contains(&SignalOutcome::LeftBetter);
    match (outcomes.is_empty(), right, left) {
        (true, _, _) => PartialOrder::NoComparableSignals,
        (false, true, true) => PartialOrder::Tradeoff,
        (false, true, false) => PartialOrder::RightDominates,
        (false, false, true) => PartialOrder::LeftDominates,
        (false, false, false) => PartialOrder::Equivalent,
    }
}

fn approximately_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-12 + 1e-9 * left.abs().max(right.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pareto_order_never_counts_axes() {
        assert_eq!(
            partial_order(&[
                SignalOutcome::RightBetter,
                SignalOutcome::Equivalent,
                SignalOutcome::RightBetter,
            ]),
            PartialOrder::RightDominates
        );
        assert_eq!(
            partial_order(&[
                SignalOutcome::RightBetter,
                SignalOutcome::LeftBetter,
                SignalOutcome::RightBetter,
            ]),
            PartialOrder::Tradeoff
        );
        assert_eq!(partial_order(&[]), PartialOrder::NoComparableSignals);
    }
}

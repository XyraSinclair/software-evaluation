use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;

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
            "each profile must contain exactly one instance of every declared directional signal and no undeclared directional signals"
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
    profile.signals.len() == SIGNAL_IDS.len()
        && SIGNAL_IDS.iter().all(|id| {
            profile
                .signals
                .iter()
                .filter(|signal| signal.id == *id)
                .count()
                == 1
        })
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

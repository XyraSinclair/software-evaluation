//! Partial-identification semantics for the mechanical quality frontier.
//!
//! Point estimates remain the primary mechanical observations. This module adds
//! a second, conservative layer for the two cases where the existing frontier
//! already carries mathematically meaningful bounds:
//!
//! - capped clone output is a lower bound on clone-token density;
//! - a low-coverage resolved symbol graph is a lower bound on working-set
//!   reachability, whose normalized fraction is at most one.
//!
//! The result is the sharp set of Pareto orders attainable in the Cartesian
//! product of those intervals. A unique order is therefore necessary under the
//! declared bounds. Qualification still requires compatible configurations,
//! evidence, schemas, canonical signal registries, and stable Git identities.
//! No probability distribution, criterion weight, or hidden scalarization is
//! introduced.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::frontier::{
    self, FrontierComparison, FrontierConfig, FrontierError, FrontierProfile, FrontierSignal,
    PartialOrder, SignalOutcome, SignalPolarity, SignalStatus,
};
use crate::frontier_contract::{signal_contract, signal_ids, valid_signal_registry};

pub const IDENTIFIED_ORDER_SCHEMA_VERSION: &str = "seval.frontier.identified.v1";

const SYMBOL_WORKING_SET: &str = "reader.symbol-working-set-p90-fraction";
const CLONE_DENSITY: &str = "uniformity.reported-clone-token-density";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct IdentifiedInterval {
    pub lower: f64,
    /// `None` denotes no finite upper bound.
    pub upper: Option<f64>,
}

impl IdentifiedInterval {
    fn exact(value: f64) -> Result<Self, String> {
        if value.is_finite() {
            Ok(Self {
                lower: value,
                upper: Some(value),
            })
        } else {
            Err("point observation is non-finite".to_owned())
        }
    }

    fn lower_bounded(lower: f64) -> Result<Self, String> {
        if lower.is_finite() {
            Ok(Self { lower, upper: None })
        } else {
            Err("lower bound is non-finite".to_owned())
        }
    }

    fn bounded(lower: f64, upper: f64) -> Result<Self, String> {
        if !lower.is_finite() || !upper.is_finite() {
            return Err("interval endpoint is non-finite".to_owned());
        }
        if lower > upper {
            return Err("interval lower endpoint exceeds its upper endpoint".to_owned());
        }
        Ok(Self {
            lower,
            upper: Some(upper),
        })
    }

    fn upper_value(self) -> f64 {
        self.upper.unwrap_or(f64::INFINITY)
    }

    fn overlaps(self, other: Self) -> bool {
        self.lower <= other.upper_value() && other.lower <= self.upper_value()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IntervalBasis {
    ExactObservation,
    CoverageLowerBound,
    CensoringLowerBound,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntervalEvidence {
    pub interval: IdentifiedInterval,
    pub basis: IntervalBasis,
    pub source_status: SignalStatus,
    pub interpretation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentifiedSignalComparison {
    pub id: String,
    pub family: String,
    pub label: String,
    pub polarity: SignalPolarity,
    pub compatible: bool,
    pub left: Option<IntervalEvidence>,
    pub right: Option<IntervalEvidence>,
    /// Every local order attainable by values inside the two intervals.
    pub possible_outcomes: Vec<SignalOutcome>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharpOrderSet {
    pub registry_valid: bool,
    pub complete: bool,
    pub comparable_signals: usize,
    pub unusable_signal_ids: Vec<String>,
    /// Exactly the Pareto orders attainable under the Cartesian-product bound
    /// model, in deterministic display order.
    pub possible_orders: Vec<PartialOrder>,
    /// Present when every compatible assignment of bounded values induces the
    /// same Pareto order. This is a mathematical property of the interval box,
    /// not by itself a provenance-qualified routing fact.
    pub necessary_order: Option<PartialOrder>,
    /// True when no compatible assignment makes the right artifact worse on
    /// any admitted coordinate. Strict improvement need not be necessary.
    pub right_necessarily_not_worse: bool,
    /// Symmetric weak relation for the left artifact.
    pub left_necessarily_not_worse: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentifiedReadiness {
    pub schema_compatible: bool,
    pub analysis_config_compatible: bool,
    pub analyzer_implementations_compatible: bool,
    pub artifacts_commit_pinned: bool,
    pub signal_registries_valid: bool,
    pub interval_surface_complete: bool,
    pub qualified_identified_set: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentifiedComparison {
    pub schema_version: String,
    pub base: FrontierComparison,
    pub readiness: IdentifiedReadiness,
    pub sharp_order_set: SharpOrderSet,
    /// A unique interval-box order promoted to a routing fact only after the
    /// full schema/configuration/evidence/artifact contract passes.
    pub qualified_necessary_order: Option<PartialOrder>,
    pub signals: Vec<IdentifiedSignalComparison>,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub fn compare_paths(
    left: &Path,
    right: &Path,
    config: &FrontierConfig,
) -> Result<IdentifiedComparison, FrontierError> {
    Ok(compare_profiles(
        frontier::profile_path(left, config)?,
        frontier::profile_path(right, config)?,
    ))
}

#[must_use]
pub fn compare_profiles(left: FrontierProfile, right: FrontierProfile) -> IdentifiedComparison {
    let signal_registries_valid = valid_signal_registry(&left) && valid_signal_registry(&right);
    let signals = compare_signal_sets(&left, &right);
    let sharp_order_set = derive_sharp_order_set(&signals, signal_registries_valid);
    let base = frontier::compare_profiles(left, right);
    let readiness = identified_readiness(&base, signal_registries_valid, sharp_order_set.complete);
    let qualified_necessary_order = if readiness.qualified_identified_set {
        sharp_order_set.necessary_order
    } else {
        None
    };

    IdentifiedComparison {
        schema_version: IDENTIFIED_ORDER_SCHEMA_VERSION.to_owned(),
        base,
        readiness,
        sharp_order_set,
        qualified_necessary_order,
        signals,
        assumptions: vec![
            "Observed finite values are singleton identified intervals.".to_owned(),
            "A capped clone-density observation identifies [reported density, +infinity)."
                .to_owned(),
            "A coverage-gated symbol working-set fraction identifies [resolved-graph fraction, 1]."
                .to_owned(),
            "Joint feasibility is conservatively outer-approximated by the Cartesian product of coordinate intervals."
                .to_owned(),
            "Pareto order uses only declared polarity; no probability distribution, weights, or scalar utility are assumed."
                .to_owned(),
        ],
        limitations: vec![
            "Intervals model identified measurement bounds, not semantic error in the underlying software-quality proxies."
                .to_owned(),
            "Ignoring dependence between coordinate bounds can enlarge the possible-order set; a unique order over the larger box remains a sound necessary order."
                .to_owned(),
            "Missing, failed, or structurally invalid signals have no declared interval and prevent a complete identified order set."
                .to_owned(),
            "A qualified necessary order is still only a routing statement over the six mechanical proxies, not proof of correctness, security, fitness, or maintainability."
                .to_owned(),
        ],
    }
}

fn identified_readiness(
    base: &FrontierComparison,
    signal_registries_valid: bool,
    interval_surface_complete: bool,
) -> IdentifiedReadiness {
    let schema_compatible = base.readiness.schema_compatible;
    let analysis_config_compatible = base.readiness.analysis_config_compatible;
    let analyzer_implementations_compatible = base.readiness.analyzer_implementations_compatible;
    let artifacts_commit_pinned = base.readiness.artifacts_commit_pinned;
    let qualified_identified_set = schema_compatible
        && analysis_config_compatible
        && analyzer_implementations_compatible
        && artifacts_commit_pinned
        && signal_registries_valid
        && interval_surface_complete;
    let mut blockers = Vec::new();
    if !schema_compatible {
        blockers.push("frontier schema versions differ or are unsupported".to_owned());
    }
    if !analysis_config_compatible {
        blockers.push("analysis-affecting configurations differ or are invalid".to_owned());
    }
    if !analyzer_implementations_compatible {
        blockers
            .push("analyzer evidence registries or implementations are incompatible".to_owned());
    }
    if !artifacts_commit_pinned {
        blockers.push("both artifacts require valid stable Git snapshot identities".to_owned());
    }
    if !signal_registries_valid {
        blockers.push("one or both signal registries violate the canonical contract".to_owned());
    }
    if !interval_surface_complete {
        blockers.push(
            "one or more directional signals lack a preregistered identified interval".to_owned(),
        );
    }
    IdentifiedReadiness {
        schema_compatible,
        analysis_config_compatible,
        analyzer_implementations_compatible,
        artifacts_commit_pinned,
        signal_registries_valid,
        interval_surface_complete,
        qualified_identified_set,
        blockers,
    }
}

fn compare_signal_sets(
    left: &FrontierProfile,
    right: &FrontierProfile,
) -> Vec<IdentifiedSignalComparison> {
    signal_ids()
        .map(|id| {
            let contract = signal_contract(id).expect("registered signal has a contract");
            match (
                unique_signal(left, id, "left"),
                unique_signal(right, id, "right"),
            ) {
                (Ok(left), Ok(right)) => compare_signal(left, right),
                (left, right) => {
                    let mut reasons = Vec::new();
                    if let Err(error) = left {
                        reasons.push(error);
                    }
                    if let Err(error) = right {
                        reasons.push(error);
                    }
                    IdentifiedSignalComparison {
                        id: id.to_owned(),
                        family: contract.family.to_owned(),
                        label: id.to_owned(),
                        polarity: contract.polarity,
                        compatible: false,
                        left: None,
                        right: None,
                        possible_outcomes: Vec::new(),
                        reason: Some(reasons.join("; ")),
                    }
                }
            }
        })
        .collect()
}

fn compare_signal(left: &FrontierSignal, right: &FrontierSignal) -> IdentifiedSignalComparison {
    let compatible = left.id == right.id
        && left.family == right.family
        && left.polarity == right.polarity
        && left.unit == right.unit
        && left.analyzer_id == right.analyzer_id
        && left.json_pointers == right.json_pointers;
    let left_result = interval_evidence(left);
    let right_result = interval_evidence(right);
    let left_public = left_result.as_ref().ok().cloned();
    let right_public = right_result.as_ref().ok().cloned();

    let (possible_outcomes, reason) = if !compatible {
        (
            Vec::new(),
            Some("signal family, polarity, unit, analyzer, or projection differs".to_owned()),
        )
    } else if let (Ok(left_evidence), Ok(right_evidence)) = (&left_result, &right_result) {
        (
            local_possible_outcomes(
                left_evidence.interval,
                right_evidence.interval,
                left.polarity,
            ),
            None,
        )
    } else {
        let mut reasons = Vec::new();
        if let Err(error) = left_result {
            reasons.push(format!("left: {error}"));
        }
        if let Err(error) = right_result {
            reasons.push(format!("right: {error}"));
        }
        (Vec::new(), Some(reasons.join("; ")))
    };

    IdentifiedSignalComparison {
        id: left.id.clone(),
        family: left.family.clone(),
        label: left.label.clone(),
        polarity: left.polarity,
        compatible,
        left: left_public,
        right: right_public,
        possible_outcomes,
        reason,
    }
}

fn interval_evidence(signal: &FrontierSignal) -> Result<IntervalEvidence, String> {
    let value = signal
        .value
        .ok_or_else(|| format!("{} has no numeric observation", signal.id))?;
    if !value.is_finite() {
        return Err(format!("{} has a non-finite observation", signal.id));
    }
    if value < 0.0 {
        return Err(format!(
            "{} has a negative value outside its declared domain",
            signal.id
        ));
    }

    match signal.status {
        SignalStatus::Observed => Ok(IntervalEvidence {
            interval: IdentifiedInterval::exact(value)?,
            basis: IntervalBasis::ExactObservation,
            source_status: signal.status,
            interpretation: "exact under the declared mechanical instrument".to_owned(),
        }),
        SignalStatus::Censored if signal.id == CLONE_DENSITY => Ok(IntervalEvidence {
            interval: IdentifiedInterval::lower_bounded(value)?,
            basis: IntervalBasis::CensoringLowerBound,
            source_status: signal.status,
            interpretation:
                "reported clone mass is a lower bound because the group cap was reached".to_owned(),
        }),
        SignalStatus::InsufficientCoverage if signal.id == SYMBOL_WORKING_SET => {
            if value > 1.0 {
                return Err(
                    "symbol working-set fraction exceeds its natural upper bound".to_owned(),
                );
            }
            Ok(IntervalEvidence {
                interval: IdentifiedInterval::bounded(value, 1.0)?,
                basis: IntervalBasis::CoverageLowerBound,
                source_status: signal.status,
                interpretation: "resolved edges provide a lower bound; the normalized reachability fraction is at most one"
                    .to_owned(),
            })
        }
        SignalStatus::Censored => Err(format!(
            "{} has censoring without a preregistered identified bound",
            signal.id
        )),
        SignalStatus::InsufficientCoverage => Err(format!(
            "{} has insufficient coverage without a preregistered identified bound",
            signal.id
        )),
        SignalStatus::Missing => Err(format!("{} is missing", signal.id)),
        SignalStatus::SourceFailed => Err(format!("{} source analyzer failed", signal.id)),
    }
}

fn local_possible_outcomes(
    left: IdentifiedInterval,
    right: IdentifiedInterval,
    polarity: SignalPolarity,
) -> Vec<SignalOutcome> {
    let mut outcomes = Vec::with_capacity(3);
    match polarity {
        SignalPolarity::LowerIsBetter => {
            if strict_less_possible(right.lower, left.upper_value()) {
                outcomes.push(SignalOutcome::RightBetter);
            }
            if strict_less_possible(left.lower, right.upper_value()) {
                outcomes.push(SignalOutcome::LeftBetter);
            }
        }
        SignalPolarity::HigherIsBetter => {
            if strict_less_possible(left.lower, right.upper_value()) {
                outcomes.push(SignalOutcome::RightBetter);
            }
            if strict_less_possible(right.lower, left.upper_value()) {
                outcomes.push(SignalOutcome::LeftBetter);
            }
        }
    }
    if equivalent_possible(left, right) {
        outcomes.push(SignalOutcome::Equivalent);
    }
    outcomes
}

fn strict_less_possible(lower: f64, upper: f64) -> bool {
    if upper.is_infinite() {
        return lower.is_finite();
    }
    lower < upper && !approximately_equal(lower, upper)
}

fn equivalent_possible(left: IdentifiedInterval, right: IdentifiedInterval) -> bool {
    if left.overlaps(right) {
        return true;
    }
    if let Some(left_upper) = left.upper
        && left_upper < right.lower
    {
        return approximately_equal(left_upper, right.lower);
    }
    if let Some(right_upper) = right.upper
        && right_upper < left.lower
    {
        return approximately_equal(right_upper, left.lower);
    }
    false
}

fn approximately_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-12 + 1e-9 * left.abs().max(right.abs())
}

fn derive_sharp_order_set(
    signals: &[IdentifiedSignalComparison],
    registry_valid: bool,
) -> SharpOrderSet {
    let unusable_signal_ids = signals
        .iter()
        .filter(|signal| signal.possible_outcomes.is_empty())
        .map(|signal| signal.id.clone())
        .collect::<Vec<_>>();
    let comparable_signals = signals.len().saturating_sub(unusable_signal_ids.len());
    let expected_signals = signal_ids().len();
    let complete =
        registry_valid && signals.len() == expected_signals && unusable_signal_ids.is_empty();
    if !complete {
        return SharpOrderSet {
            registry_valid,
            complete,
            comparable_signals,
            unusable_signal_ids,
            possible_orders: Vec::new(),
            necessary_order: None,
            right_necessarily_not_worse: false,
            left_necessarily_not_worse: false,
        };
    }

    let mut states = BTreeSet::from([DirectionState::default()]);
    for signal in signals {
        let mut next = BTreeSet::new();
        for state in &states {
            for outcome in &signal.possible_outcomes {
                let mut updated = *state;
                match outcome {
                    SignalOutcome::RightBetter => updated.right = true,
                    SignalOutcome::LeftBetter => updated.left = true,
                    SignalOutcome::Equivalent => {}
                    SignalOutcome::Unavailable | SignalOutcome::Incompatible => continue,
                }
                next.insert(updated);
            }
        }
        states = next;
    }

    let candidates = [
        (
            DirectionState {
                right: true,
                left: false,
            },
            PartialOrder::RightDominates,
        ),
        (
            DirectionState {
                right: false,
                left: true,
            },
            PartialOrder::LeftDominates,
        ),
        (
            DirectionState {
                right: true,
                left: true,
            },
            PartialOrder::Tradeoff,
        ),
        (DirectionState::default(), PartialOrder::Equivalent),
    ];
    let possible_orders = candidates
        .into_iter()
        .filter_map(|(state, order)| states.contains(&state).then_some(order))
        .collect::<Vec<_>>();
    let necessary_order = if possible_orders.len() == 1 {
        possible_orders.first().copied()
    } else {
        None
    };
    let right_necessarily_not_worse = !possible_orders.is_empty()
        && possible_orders.iter().all(|order| {
            matches!(
                order,
                PartialOrder::RightDominates | PartialOrder::Equivalent
            )
        });
    let left_necessarily_not_worse = !possible_orders.is_empty()
        && possible_orders.iter().all(|order| {
            matches!(
                order,
                PartialOrder::LeftDominates | PartialOrder::Equivalent
            )
        });

    SharpOrderSet {
        registry_valid,
        complete,
        comparable_signals,
        unusable_signal_ids,
        possible_orders,
        necessary_order,
        right_necessarily_not_worse,
        left_necessarily_not_worse,
    }
}

fn unique_signal<'a>(
    profile: &'a FrontierProfile,
    id: &str,
    side: &str,
) -> Result<&'a FrontierSignal, String> {
    let mut matches = profile.signals.iter().filter(|signal| signal.id == id);
    let signal = matches
        .next()
        .ok_or_else(|| format!("{side} profile is missing {id}"))?;
    if matches.next().is_some() {
        return Err(format!("{side} profile duplicates {id}"));
    }
    Ok(signal)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct DirectionState {
    right: bool,
    left: bool,
}

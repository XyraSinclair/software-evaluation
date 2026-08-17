use std::collections::BTreeSet;

use super::*;

pub(crate) struct SignalContract {
    pub id: &'static str,
    pub family: &'static str,
    pub polarity: SignalPolarity,
    pub analyzer_id: &'static str,
    pub unit: &'static str,
    pub json_pointers: &'static [&'static str],
    pub bounded_by_one: bool,
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
    ("interface-depth", &["interface.shallow-function-fraction"]),
    (
        "effect-locality",
        &[
            "effects.syntactic-pure-fraction",
            "effects.mutable-live-range-p90-lines",
        ],
    ),
    ("uniformity", &["uniformity.reported-clone-token-density"]),
];

const ANALYZER_IDS: [&str; 4] = [SHAPE, SYMBOLS, DISCIPLINE, DUPLICATES];

pub(crate) fn signal_ids() -> impl ExactSizeIterator<Item = &'static str> + Clone {
    SIGNAL_CONTRACTS.iter().map(|contract| contract.id)
}

pub(crate) fn signal_contract(id: &str) -> Option<&'static SignalContract> {
    SIGNAL_CONTRACTS.iter().find(|contract| contract.id == id)
}

pub(crate) fn valid_signal_registry(profile: &FrontierProfile) -> bool {
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

pub(crate) fn compatible_config(left: &FrontierConfig, right: &FrontierConfig) -> bool {
    left.validate().is_ok() && right.validate().is_ok() && left == right
}

pub(crate) fn compatible_implementations(
    left: &FrontierProfile,
    right: &FrontierProfile,
) -> bool {
    valid_analyzer_registry(left)
        && valid_analyzer_registry(right)
        && ANALYZER_IDS.into_iter().all(|analyzer_id| {
            let left = unique_complete_implementation(left, analyzer_id);
            let right = unique_complete_implementation(right, analyzer_id);
            matches!((left, right), (Some(left), Some(right)) if left == right)
        })
}

pub(crate) fn artifact_is_pinned(profile: &FrontierProfile) -> bool {
    let Some(git) = profile.artifact.git.as_ref() else {
        return false;
    };
    profile.artifact.identity_error.is_none()
        && !profile.artifact.input.trim().is_empty()
        && !git.id.trim().is_empty()
        && git.root.is_absolute()
        && git.kind == "git-repository"
        && valid_git_oid(&git.revision)
        && valid_git_oid(&git.tree_digest)
        && git.revision.len() == git.tree_digest.len()
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
            signal.value.is_some()
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
        && profile.coverage.unusable_signal_ids.len() == reported_unusable.len()
        && observed + unusable.len() == SIGNAL_CONTRACTS.len()
        && unusable == reported_unusable
}

fn valid_analyzer_registry(profile: &FrontierProfile) -> bool {
    profile.analyzers.len() == ANALYZER_IDS.len()
        && ANALYZER_IDS.into_iter().all(|id| {
            let mut matches = profile
                .analyzers
                .iter()
                .filter(|receipt| receipt.id == id);
            matches.next().is_some() && matches.next().is_none()
        })
}

fn unique_complete_implementation<'a>(
    profile: &'a FrontierProfile,
    analyzer_id: &str,
) -> Option<&'a str> {
    let receipt = profile
        .analyzers
        .iter()
        .find(|receipt| receipt.id == analyzer_id)?;
    if receipt.status != AnalyzerStatus::Complete
        || receipt.error.is_some()
        || receipt.coverage.is_none()
        || !receipt
            .payload_sha256
            .as_deref()
            .is_some_and(valid_sha256_hex)
    {
        return None;
    }
    receipt
        .implementation
        .as_deref()
        .filter(|implementation| !implementation.trim().is_empty())
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

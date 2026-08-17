use super::*;

pub(super) fn project(
    reports: &BTreeMap<String, Value>,
    config: &FrontierConfig,
) -> Vec<FrontierSignal> {
    vec![
        scalar(
            reports,
            SHAPE,
            "reader.local-cognitive-p90",
            "reader-load",
            "local cognitive p90",
            SignalPolarity::LowerIsBetter,
            "/distributions/cognitive/p90",
            "/coverage/functions_analyzed",
            "AST cognitive-complexity units",
            "Within-unit reader load; paired with cross-unit symbol reachability.",
        ),
        symbol_working_set(reports, config),
        ratio(
            reports,
            SHAPE,
            "interface.shallow-function-fraction",
            "interface-depth",
            "shallow-function fraction",
            SignalPolarity::LowerIsBetter,
            "/coverage/shallow_functions",
            "/coverage/shallow_denominator",
            "fraction of analyzed functions",
            "Interface width is at least the observed interior volume; globally guarded by reader load.",
        ),
        scalar(
            reports,
            DISCIPLINE,
            "effects.syntactic-pure-fraction",
            "effect-locality",
            "syntactically pure function fraction",
            SignalPolarity::HigherIsBetter,
            "/coverage/pure_fraction",
            "/coverage/functions_total",
            "fraction of analyzed functions",
            "Syntactic effects-at-the-edges proxy; unresolved calls remain a limitation.",
        ),
        scalar(
            reports,
            DISCIPLINE,
            "effects.mutable-live-range-p90-lines",
            "effect-locality",
            "mutable live-range p90",
            SignalPolarity::LowerIsBetter,
            "/coverage/tails/mutable_live_range_lines_given_mutable/p90",
            "/coverage/functions_with_mutable_bindings",
            "source lines",
            "Tail of the longest syntactic mutable-binding live range per function, conditioned on functions with at least one mutable binding.",
        ),
        clone_density(reports, config),
    ]
}

#[allow(clippy::too_many_arguments)]
fn scalar(
    reports: &BTreeMap<String, Value>,
    analyzer: &str,
    id: &str,
    family: &str,
    label: &str,
    polarity: SignalPolarity,
    value_pointer: &str,
    denominator_pointer: &str,
    unit: &str,
    note: &str,
) -> FrontierSignal {
    let pointers = [value_pointer, denominator_pointer];
    let Some(report) = reports.get(analyzer) else {
        return unavailable(
            analyzer,
            id,
            family,
            label,
            polarity,
            unit,
            &pointers,
            SignalStatus::SourceFailed,
            None,
            None,
            None,
            "analyzer did not produce an observation",
            note,
        );
    };
    let value = number(report, value_pointer);
    let denominator = number(report, denominator_pointer);
    if value.is_none() || denominator.is_none_or(|denominator| denominator <= 0.0) {
        return unavailable(
            analyzer,
            id,
            family,
            label,
            polarity,
            unit,
            &pointers,
            SignalStatus::Missing,
            value,
            None,
            denominator,
            "value or supporting denominator was unavailable",
            note,
        );
    }
    observed(
        analyzer,
        id,
        family,
        label,
        polarity,
        unit,
        &pointers,
        value,
        None,
        denominator,
        note,
    )
}

#[allow(clippy::too_many_arguments)]
fn ratio(
    reports: &BTreeMap<String, Value>,
    analyzer: &str,
    id: &str,
    family: &str,
    label: &str,
    polarity: SignalPolarity,
    numerator_pointer: &str,
    denominator_pointer: &str,
    unit: &str,
    note: &str,
) -> FrontierSignal {
    let pointers = [numerator_pointer, denominator_pointer];
    let Some(report) = reports.get(analyzer) else {
        return unavailable(
            analyzer,
            id,
            family,
            label,
            polarity,
            unit,
            &pointers,
            SignalStatus::SourceFailed,
            None,
            None,
            None,
            "analyzer did not produce an observation",
            note,
        );
    };
    let numerator = number(report, numerator_pointer);
    let denominator = number(report, denominator_pointer);
    let value = numerator
        .zip(denominator)
        .filter(|(_, denominator)| *denominator > 0.0)
        .map(|(numerator, denominator)| numerator / denominator);
    if value.is_none() {
        return unavailable(
            analyzer,
            id,
            family,
            label,
            polarity,
            unit,
            &pointers,
            SignalStatus::Missing,
            value,
            numerator,
            denominator,
            "ratio denominator was absent or zero",
            note,
        );
    }
    observed(
        analyzer,
        id,
        family,
        label,
        polarity,
        unit,
        &pointers,
        value,
        numerator,
        denominator,
        note,
    )
}

fn symbol_working_set(
    reports: &BTreeMap<String, Value>,
    config: &FrontierConfig,
) -> FrontierSignal {
    let pointers = [
        "/working_set_reachability/p90",
        "/working_set_reachability/nodes_in_distribution",
        "/graph/node_count",
        "/resolution/resolution_fraction",
    ];
    let Some(report) = reports.get(SYMBOLS) else {
        return unavailable(
            SYMBOLS,
            SIGNAL_IDS[1],
            "reader-load",
            "symbol working-set p90 fraction",
            SignalPolarity::LowerIsBetter,
            "fraction of other resolved symbols",
            &pointers,
            SignalStatus::SourceFailed,
            None,
            None,
            None,
            "analyzer did not produce an observation",
            "Cross-unit reader working set on the resolved lower-bound Rust symbol graph.",
        );
    };
    let numerator = number(report, pointers[0]);
    let denominator = number(report, pointers[2])
        .filter(|nodes| *nodes >= 2.0)
        .map(|nodes| nodes - 1.0);
    let value = numerator.zip(denominator).map(|(p90, total)| p90 / total);
    if number(report, pointers[1]).unwrap_or(0.0) == 0.0 || value.is_none() {
        return unavailable(
            SYMBOLS,
            SIGNAL_IDS[1],
            "reader-load",
            "symbol working-set p90 fraction",
            SignalPolarity::LowerIsBetter,
            "fraction of other resolved symbols",
            &pointers,
            SignalStatus::Missing,
            value,
            numerator,
            denominator,
            "no nontrivial symbol working-set distribution was available",
            "Cross-unit reader working set on the resolved lower-bound Rust symbol graph.",
        );
    }
    let resolution = number(report, pointers[3]);
    if resolution.is_none_or(|value| value < config.min_symbol_resolution_fraction) {
        return unavailable(
            SYMBOLS,
            SIGNAL_IDS[1],
            "reader-load",
            "symbol working-set p90 fraction",
            SignalPolarity::LowerIsBetter,
            "fraction of other resolved symbols",
            &pointers,
            SignalStatus::InsufficientCoverage,
            value,
            numerator,
            denominator,
            &format!(
                "resolved-reference fraction {:?} is below coverage gate {:.3}",
                resolution, config.min_symbol_resolution_fraction
            ),
            "Cross-unit reader working set on the resolved lower-bound Rust symbol graph.",
        );
    }
    observed(
        SYMBOLS,
        SIGNAL_IDS[1],
        "reader-load",
        "symbol working-set p90 fraction",
        SignalPolarity::LowerIsBetter,
        "fraction of other resolved symbols",
        &pointers,
        value,
        numerator,
        denominator,
        "Cross-unit reader working set on the resolved lower-bound Rust symbol graph.",
    )
}

fn clone_density(reports: &BTreeMap<String, Value>, config: &FrontierConfig) -> FrontierSignal {
    let pointers = [
        "/totals/duplicated_tokens",
        "/coverage/considered_tokens",
        "/totals/clone_groups",
        "/config/max_groups",
    ];
    let Some(report) = reports.get(DUPLICATES) else {
        return unavailable(
            DUPLICATES,
            SIGNAL_IDS[5],
            "uniformity",
            "reported clone-token density",
            SignalPolarity::LowerIsBetter,
            "reported duplicated token mass / considered token",
            &pointers,
            SignalStatus::SourceFailed,
            None,
            None,
            None,
            "analyzer did not produce an observation",
            "AST-normalized clone mass; mass can exceed one because it is not unique coverage.",
        );
    };
    let numerator = number(report, pointers[0]);
    let denominator = number(report, pointers[1]);
    let value = numerator
        .zip(denominator)
        .filter(|(_, denominator)| *denominator > 0.0)
        .map(|(duplicated, considered)| duplicated / considered);
    if value.is_none() {
        return unavailable(
            DUPLICATES,
            SIGNAL_IDS[5],
            "uniformity",
            "reported clone-token density",
            SignalPolarity::LowerIsBetter,
            "reported duplicated token mass / considered token",
            &pointers,
            SignalStatus::Missing,
            value,
            numerator,
            denominator,
            "no considered-token denominator was available",
            "AST-normalized clone mass; mass can exceed one because it is not unique coverage.",
        );
    }
    let groups = number(report, pointers[2]).unwrap_or(0.0);
    let cap = number(report, pointers[3]).unwrap_or(config.duplicate_max_groups as f64);
    if groups >= cap {
        return unavailable(
            DUPLICATES,
            SIGNAL_IDS[5],
            "uniformity",
            "reported clone-token density",
            SignalPolarity::LowerIsBetter,
            "reported duplicated token mass / considered token",
            &pointers,
            SignalStatus::Censored,
            value,
            numerator,
            denominator,
            "clone-group output reached its cap; totals are a lower bound",
            "AST-normalized clone mass; mass can exceed one because it is not unique coverage.",
        );
    }
    observed(
        DUPLICATES,
        SIGNAL_IDS[5],
        "uniformity",
        "reported clone-token density",
        SignalPolarity::LowerIsBetter,
        "reported duplicated token mass / considered token",
        &pointers,
        value,
        numerator,
        denominator,
        "AST-normalized clone mass; mass can exceed one because it is not unique coverage.",
    )
}

#[allow(clippy::too_many_arguments)]
fn observed(
    analyzer: &str,
    id: &str,
    family: &str,
    label: &str,
    polarity: SignalPolarity,
    unit: &str,
    pointers: &[&str],
    value: Option<f64>,
    numerator: Option<f64>,
    denominator: Option<f64>,
    note: &str,
) -> FrontierSignal {
    FrontierSignal {
        id: id.to_owned(),
        family: family.to_owned(),
        label: label.to_owned(),
        polarity,
        status: SignalStatus::Observed,
        value,
        numerator,
        denominator,
        unit: unit.to_owned(),
        analyzer_id: analyzer.to_owned(),
        json_pointers: pointers
            .iter()
            .map(|pointer| (*pointer).to_owned())
            .collect(),
        note: note.to_owned(),
        unavailable_reason: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn unavailable(
    analyzer: &str,
    id: &str,
    family: &str,
    label: &str,
    polarity: SignalPolarity,
    unit: &str,
    pointers: &[&str],
    status: SignalStatus,
    value: Option<f64>,
    numerator: Option<f64>,
    denominator: Option<f64>,
    reason: &str,
    note: &str,
) -> FrontierSignal {
    let mut signal = observed(
        analyzer,
        id,
        family,
        label,
        polarity,
        unit,
        pointers,
        value,
        numerator,
        denominator,
        note,
    );
    signal.status = status;
    signal.unavailable_reason = Some(reason.to_owned());
    signal
}

fn number(value: &Value, pointer: &str) -> Option<f64> {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
}

pub(super) fn families() -> Vec<SignalFamily> {
    vec![
        SignalFamily {
            id: "reader-load".to_owned(),
            label: "reader load".to_owned(),
            signal_ids: SIGNAL_IDS[..2].iter().map(|id| (*id).to_owned()).collect(),
            rule:
                "Neither within-unit cognitive tail nor cross-unit symbol working set may regress."
                    .to_owned(),
        },
        SignalFamily {
            id: "interface-depth".to_owned(),
            label: "interface depth".to_owned(),
            signal_ids: vec![SIGNAL_IDS[2].to_owned()],
            rule: "Prefer fewer shallow functions, subject to the global reader-load guards."
                .to_owned(),
        },
        SignalFamily {
            id: "effect-locality".to_owned(),
            label: "effect locality".to_owned(),
            signal_ids: SIGNAL_IDS[3..5].iter().map(|id| (*id).to_owned()).collect(),
            rule: "Syntactic purity may rise only without lengthening the mutable-state tail."
                .to_owned(),
        },
        SignalFamily {
            id: "uniformity".to_owned(),
            label: "uniformity".to_owned(),
            signal_ids: vec![SIGNAL_IDS[5].to_owned()],
            rule: "Prefer lower uncensored clone mass, subject to every global guard.".to_owned(),
        },
    ]
}

pub(super) fn coverage(signals: &[FrontierSignal]) -> DirectionalCoverage {
    let unusable_signal_ids = signals
        .iter()
        .filter(|signal| signal.status != SignalStatus::Observed)
        .map(|signal| signal.id.clone())
        .collect::<Vec<_>>();
    DirectionalCoverage {
        declared: SIGNAL_IDS.len(),
        observed: SIGNAL_IDS.len().saturating_sub(unusable_signal_ids.len()),
        unusable_signal_ids,
    }
}

pub(super) fn limitations() -> Vec<String> {
    vec![
        "The frontier is a routing surface over mechanical proxies, not an absolute software-quality measurement or proof.".to_owned(),
        "Correctness behavior, security, performance, documentation truth, fitness-to-intent, and independent judgment remain outside this fast pass.".to_owned(),
        "The four source analyzers currently discover, read, and parse independently. They run concurrently, but a shared immutable parse substrate is the main remaining computational-parsimony opportunity.".to_owned(),
        "The symbol working-set coordinate is a Rust-only resolved lower bound and is gated on declared resolution coverage.".to_owned(),
        "Reaching the clone-group presentation cap censors clone density instead of treating a lower bound as complete.".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_clone_totals_are_censored() {
        let reports = BTreeMap::from([(
            DUPLICATES.to_owned(),
            serde_json::json!({
                "coverage": {"considered_tokens": 1000},
                "config": {"max_groups": 100},
                "totals": {"clone_groups": 100, "duplicated_tokens": 250}
            }),
        )]);
        let signal = clone_density(&reports, &FrontierConfig::default());
        assert_eq!(signal.status, SignalStatus::Censored);
        assert_eq!(signal.value, Some(0.25));
    }

    #[test]
    fn low_symbol_resolution_retains_value_but_blocks_dominance() {
        let reports = BTreeMap::from([(
            SYMBOLS.to_owned(),
            serde_json::json!({
                "resolution": {"resolution_fraction": 0.2},
                "graph": {"node_count": 11},
                "working_set_reachability": {"nodes_in_distribution": 11, "p90": 5}
            }),
        )]);
        let signal = symbol_working_set(&reports, &FrontierConfig::default());
        assert_eq!(signal.status, SignalStatus::InsufficientCoverage);
        assert_eq!(signal.value, Some(0.5));
    }
}

//! Information-theoretic planning for audit probes.
//!
//! The planner keeps information, Bayes decision risk, money, and elapsed time
//! as separate quantities. Claim importance is applied only while constructing
//! a plan; the metrics reported for an individual probe remain unweighted.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const EXACT_ENUMERATION_LIMIT: usize = 20;
const ROUND_OFF_FACTOR: f64 = 256.0;

/// Complete input to the audit planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSpec {
    pub claims: Vec<ClaimSpec>,
    pub probes: Vec<ProbeSpec>,
    pub budget: Budget,
    pub strategy: SelectionStrategy,
    pub conditional_independence_assumed: bool,
}

/// Prior, losses, and planning importance for one binary claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimSpec {
    pub id: String,
    pub prior_probability: f64,
    pub false_positive_loss: f64,
    pub false_negative_loss: f64,
    pub importance: f64,
}

/// Calibration and resource estimates for one binary audit probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeSpec {
    pub id: String,
    pub claim_id: String,
    pub sensitivity: f64,
    pub specificity: f64,
    pub expected_usd: f64,
    pub expected_seconds: f64,
    pub dependence_group: String,
}

/// Hard resource limits for a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub max_usd: f64,
    pub max_seconds: f64,
    pub max_probes: usize,
}

/// Greedy objective used to choose among the current Pareto frontier.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStrategy {
    MaxInformation,
    InformationPerUsd,
    InformationPerSecond,
    MaxRiskReduction,
}

/// Raw, unweighted metrics for a single probe.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeMetrics {
    pub probe_id: String,
    pub claim_id: String,
    pub probability_positive: f64,
    pub posterior_if_positive: f64,
    pub posterior_if_negative: f64,
    pub information_gain_bits: f64,
    pub expected_bayes_risk_reduction: f64,
    pub decision_flip_probability: f64,
    pub bits_per_usd: Option<f64>,
    pub bits_per_second: Option<f64>,
    pub expected_usd: f64,
    pub expected_seconds: f64,
    pub dependence_group: String,
}

/// One choice made by the sequential greedy planner.
#[derive(Debug, Clone, Serialize)]
pub struct PlanStep {
    pub ordinal: usize,
    pub probe_id: String,
    pub claim_id: String,
    pub marginal_information_bits: f64,
    pub marginal_risk_reduction: f64,
    pub cumulative_information_bits: f64,
    pub cumulative_expected_risk_reduction: f64,
    pub cumulative_usd: f64,
    pub cumulative_seconds: f64,
    pub candidate_frontier: Vec<String>,
}

/// Metrics, frontier, chosen sequence, totals, and explicit model assumptions.
#[derive(Debug, Clone, Serialize)]
pub struct PlanReport {
    pub probe_metrics: Vec<ProbeMetrics>,
    pub single_probe_frontier: Vec<String>,
    pub selected: Vec<PlanStep>,
    pub total_information_bits: f64,
    pub total_expected_risk_reduction: f64,
    pub total_usd: f64,
    pub total_seconds: f64,
    pub stopped_reason: String,
    pub assumptions: Vec<String>,
}

/// Invalid model inputs or arithmetic that cannot be represented honestly.
#[derive(Debug, Error)]
pub enum PlanError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("numerical error: {0}")]
    Numerical(String),
    #[error(
        "exact enumeration for claim {claim_id:?} requires {requested} probes; the limit is {limit}"
    )]
    ExactEnumerationLimit {
        claim_id: String,
        requested: usize,
        limit: usize,
    },
}

/// Binary Shannon entropy in bits.
///
/// The boundary values are defined by continuity, so `H(0) = H(1) = 0`.
/// Invalid probabilities produce `NaN`; public operations that accept model
/// input validate probabilities before calling this helper.
pub fn binary_entropy_bits(p: f64) -> f64 {
    if p == 0.0 || p == 1.0 {
        0.0
    } else if p.is_finite() && p > 0.0 && p < 1.0 {
        -p * p.log2() - (1.0 - p) * (1.0 - p).log2()
    } else {
        f64::NAN
    }
}

/// Compute raw metrics for a single calibrated probe and binary claim.
pub fn metrics_for_probe(claim: &ClaimSpec, probe: &ProbeSpec) -> Result<ProbeMetrics, PlanError> {
    validate_claim(claim, "claim")?;
    validate_probe(probe, "probe")?;
    if probe.claim_id != claim.id {
        return Err(PlanError::InvalidInput(format!(
            "probe {:?} refers to claim {:?}, not {:?}",
            probe.id, probe.claim_id, claim.id
        )));
    }

    let prior = claim.prior_probability;
    let prior_false = 1.0 - prior;
    let false_positive_rate = 1.0 - probe.specificity;
    let false_negative_rate = 1.0 - probe.sensitivity;

    let positive_true_mass = prior * probe.sensitivity;
    let positive_false_mass = prior_false * false_positive_rate;
    let negative_true_mass = prior * false_negative_rate;
    let negative_false_mass = prior_false * probe.specificity;

    let probability_positive = unit_roundoff(
        checked_add(
            positive_true_mass,
            positive_false_mass,
            "positive outcome probability",
        )?,
        "positive outcome probability",
    )?;
    let probability_negative = unit_roundoff(
        checked_add(
            negative_true_mass,
            negative_false_mass,
            "negative outcome probability",
        )?,
        "negative outcome probability",
    )?;

    let posterior_if_positive = posterior_probability(
        positive_true_mass,
        probability_positive,
        prior,
        "positive outcome posterior",
    )?;
    let posterior_if_negative = posterior_probability(
        negative_true_mass,
        probability_negative,
        prior,
        "negative outcome posterior",
    )?;

    let prior_entropy = binary_entropy_bits(prior);
    let expected_posterior_entropy = checked_add(
        probability_positive * binary_entropy_bits(posterior_if_positive),
        probability_negative * binary_entropy_bits(posterior_if_negative),
        "expected posterior entropy",
    )?;
    let information_gain_bits = nonnegative_roundoff(
        prior_entropy - expected_posterior_entropy,
        prior_entropy.max(expected_posterior_entropy),
        "single-probe mutual information",
    )?;

    let current_risk = bayes_risk(prior, claim)?;
    // Multiplying conditional risk by the outcome probability simplifies to
    // these joint-mass expressions and remains defined for impossible outcomes.
    let positive_risk = checked_min_loss(
        positive_false_mass,
        claim.false_positive_loss,
        positive_true_mass,
        claim.false_negative_loss,
        "positive-outcome Bayes risk",
    )?;
    let negative_risk = checked_min_loss(
        negative_false_mass,
        claim.false_positive_loss,
        negative_true_mass,
        claim.false_negative_loss,
        "negative-outcome Bayes risk",
    )?;
    let expected_post_probe_risk = checked_add(
        positive_risk,
        negative_risk,
        "expected post-probe Bayes risk",
    )?;
    let expected_bayes_risk_reduction = nonnegative_roundoff(
        current_risk - expected_post_probe_risk,
        current_risk.max(expected_post_probe_risk),
        "single-probe Bayes risk reduction",
    )?;

    let current_action = bayes_action(prior, claim)?;
    let positive_action = bayes_action(posterior_if_positive, claim)?;
    let negative_action = bayes_action(posterior_if_negative, claim)?;
    let mut decision_flip_probability = 0.0;
    if positive_action != current_action {
        decision_flip_probability = checked_add(
            decision_flip_probability,
            probability_positive,
            "decision-flip probability",
        )?;
    }
    if negative_action != current_action {
        decision_flip_probability = checked_add(
            decision_flip_probability,
            probability_negative,
            "decision-flip probability",
        )?;
    }
    let decision_flip_probability =
        unit_roundoff(decision_flip_probability, "decision-flip probability")?;

    let bits_per_usd = rate_or_none(
        information_gain_bits,
        probe.expected_usd,
        "information per USD",
    )?;
    let bits_per_second = rate_or_none(
        information_gain_bits,
        probe.expected_seconds,
        "information per second",
    )?;

    Ok(ProbeMetrics {
        probe_id: probe.id.clone(),
        claim_id: probe.claim_id.clone(),
        probability_positive,
        posterior_if_positive,
        posterior_if_negative,
        information_gain_bits,
        expected_bayes_risk_reduction,
        decision_flip_probability,
        bits_per_usd,
        bits_per_second,
        expected_usd: probe.expected_usd,
        expected_seconds: probe.expected_seconds,
        dependence_group: probe.dependence_group.clone(),
    })
}

/// Validate the model, compute raw metrics, and construct a budget-feasible
/// greedy audit plan.
pub fn plan(spec: &PlanSpec) -> Result<PlanReport, PlanError> {
    let claim_index_by_id = validate_plan_spec(spec)?;

    let mut probe_metrics = Vec::with_capacity(spec.probes.len());
    let mut probe_claim_indices = Vec::with_capacity(spec.probes.len());
    for probe in &spec.probes {
        let claim_index = match claim_index_by_id.get(&probe.claim_id) {
            Some(index) => *index,
            None => {
                return Err(PlanError::InvalidInput(format!(
                    "probe {:?} refers to unknown claim {:?}",
                    probe.id, probe.claim_id
                )));
            }
        };
        let claim = match spec.claims.get(claim_index) {
            Some(claim) => claim,
            None => {
                return Err(PlanError::Numerical(
                    "validated claim index was unavailable".to_owned(),
                ));
            }
        };
        probe_metrics.push(metrics_for_probe(claim, probe)?);
        probe_claim_indices.push(claim_index);
    }

    let single_probe_frontier = single_probe_frontier(spec, &probe_metrics, &probe_claim_indices)?;

    let mut states = Vec::with_capacity(spec.claims.len());
    for claim in &spec.claims {
        states.push(ClaimState {
            selected_probe_indices: Vec::new(),
            joint: JointMetrics {
                information_bits: 0.0,
                risk_reduction: 0.0,
            },
            claim_id: claim.id.clone(),
        });
    }

    let mut selected_flags = vec![false; spec.probes.len()];
    let mut selected = Vec::new();
    let mut total_information_bits = 0.0;
    let mut total_expected_risk_reduction = 0.0;
    let mut total_usd = 0.0;
    let mut total_seconds = 0.0;

    let stopped_reason = loop {
        if selected.len() >= spec.budget.max_probes {
            break "max probes reached".to_owned();
        }
        if !spec.conditional_independence_assumed && !selected.is_empty() {
            break "dependence constraint".to_owned();
        }

        let mut remaining_unselected = 0usize;
        let mut after_dependence_constraint = 0usize;
        let mut after_enumeration_limit = 0usize;
        let mut candidates = Vec::new();

        for (probe_index, probe) in spec.probes.iter().enumerate() {
            let already_selected = match selected_flags.get(probe_index) {
                Some(flag) => *flag,
                None => {
                    return Err(PlanError::Numerical(
                        "probe selection state was unavailable".to_owned(),
                    ));
                }
            };
            if already_selected {
                continue;
            }
            remaining_unselected += 1;

            let claim_index = match probe_claim_indices.get(probe_index) {
                Some(index) => *index,
                None => {
                    return Err(PlanError::Numerical(
                        "probe-to-claim index was unavailable".to_owned(),
                    ));
                }
            };
            let state = match states.get(claim_index) {
                Some(state) => state,
                None => {
                    return Err(PlanError::Numerical(
                        "claim selection state was unavailable".to_owned(),
                    ));
                }
            };

            if has_dependence_conflict(spec, state, probe)? {
                continue;
            }
            after_dependence_constraint += 1;

            if state.selected_probe_indices.len() >= EXACT_ENUMERATION_LIMIT {
                continue;
            }
            after_enumeration_limit += 1;

            if !fits_budget(total_usd, probe.expected_usd, spec.budget.max_usd)
                || !fits_budget(
                    total_seconds,
                    probe.expected_seconds,
                    spec.budget.max_seconds,
                )
            {
                continue;
            }

            let claim = match spec.claims.get(claim_index) {
                Some(claim) => claim,
                None => {
                    return Err(PlanError::Numerical(
                        "candidate claim was unavailable".to_owned(),
                    ));
                }
            };
            let mut joint_probes = Vec::with_capacity(state.selected_probe_indices.len() + 1);
            for selected_index in &state.selected_probe_indices {
                let selected_probe = match spec.probes.get(*selected_index) {
                    Some(selected_probe) => selected_probe,
                    None => {
                        return Err(PlanError::Numerical(
                            "selected probe was unavailable".to_owned(),
                        ));
                    }
                };
                joint_probes.push(selected_probe);
            }
            joint_probes.push(probe);

            let mut new_joint = joint_metrics(claim, &joint_probes)?;
            let raw_marginal_information = nonnegative_roundoff(
                new_joint.information_bits - state.joint.information_bits,
                new_joint.information_bits.max(state.joint.information_bits),
                "marginal mutual information",
            )?;
            let raw_marginal_risk = nonnegative_roundoff(
                new_joint.risk_reduction - state.joint.risk_reduction,
                new_joint.risk_reduction.max(state.joint.risk_reduction),
                "marginal Bayes risk reduction",
            )?;
            // If subtraction identified only round-off, retain the previous
            // exact-model total so cumulative values cannot drift downward.
            if new_joint.information_bits < state.joint.information_bits {
                new_joint.information_bits = state.joint.information_bits;
            }
            if new_joint.risk_reduction < state.joint.risk_reduction {
                new_joint.risk_reduction = state.joint.risk_reduction;
            }
            let marginal_information = checked_product(
                raw_marginal_information,
                claim.importance,
                "importance-weighted marginal information",
            )?;
            let marginal_risk = checked_product(
                raw_marginal_risk,
                claim.importance,
                "importance-weighted marginal Bayes risk reduction",
            )?;

            candidates.push(Candidate {
                probe_index,
                claim_index,
                probe_id: probe.id.clone(),
                marginal_information,
                marginal_risk,
                expected_usd: probe.expected_usd,
                expected_seconds: probe.expected_seconds,
                new_joint,
            });
        }

        if candidates.is_empty() {
            if remaining_unselected == 0 {
                break "no candidates".to_owned();
            }
            if after_dependence_constraint == 0 {
                break "dependence constraint".to_owned();
            }
            if after_enumeration_limit == 0 {
                break "exact enumeration limit".to_owned();
            }
            break "budget exhausted".to_owned();
        }

        let candidate_frontier = candidate_frontier(&candidates);
        let mut frontier_candidates = Vec::with_capacity(candidate_frontier.len());
        for candidate in &candidates {
            if candidate_frontier
                .iter()
                .any(|probe_id| probe_id == &candidate.probe_id)
            {
                frontier_candidates.push(candidate.clone());
            }
        }
        if frontier_candidates.is_empty() {
            break "no positive marginal value".to_owned();
        }
        let best_index = best_candidate_index(&frontier_candidates, &spec.strategy)?;
        let chosen = match frontier_candidates.get(best_index) {
            Some(candidate) => candidate.clone(),
            None => {
                return Err(PlanError::Numerical(
                    "chosen candidate was unavailable".to_owned(),
                ));
            }
        };

        if !has_positive_strategy_value(&chosen, &spec.strategy) {
            break "no positive marginal value".to_owned();
        }

        let chosen_probe = match spec.probes.get(chosen.probe_index) {
            Some(probe) => probe,
            None => {
                return Err(PlanError::Numerical(
                    "chosen probe was unavailable".to_owned(),
                ));
            }
        };
        let chosen_claim = match spec.claims.get(chosen.claim_index) {
            Some(claim) => claim,
            None => {
                return Err(PlanError::Numerical(
                    "chosen claim was unavailable".to_owned(),
                ));
            }
        };

        match selected_flags.get_mut(chosen.probe_index) {
            Some(flag) => *flag = true,
            None => {
                return Err(PlanError::Numerical(
                    "chosen probe selection flag was unavailable".to_owned(),
                ));
            }
        }
        match states.get_mut(chosen.claim_index) {
            Some(state) => {
                state.selected_probe_indices.push(chosen.probe_index);
                state.joint = chosen.new_joint;
            }
            None => {
                return Err(PlanError::Numerical(
                    "chosen claim selection state was unavailable".to_owned(),
                ));
            }
        }

        total_usd = checked_add(total_usd, chosen_probe.expected_usd, "cumulative USD")?;
        total_seconds = checked_add(
            total_seconds,
            chosen_probe.expected_seconds,
            "cumulative seconds",
        )?;
        let totals = weighted_joint_totals(spec, &states)?;
        total_information_bits = totals.information_bits;
        total_expected_risk_reduction = totals.risk_reduction;

        selected.push(PlanStep {
            ordinal: selected.len() + 1,
            probe_id: chosen_probe.id.clone(),
            claim_id: chosen_claim.id.clone(),
            marginal_information_bits: chosen.marginal_information,
            marginal_risk_reduction: chosen.marginal_risk,
            cumulative_information_bits: total_information_bits,
            cumulative_expected_risk_reduction: total_expected_risk_reduction,
            cumulative_usd: total_usd,
            cumulative_seconds: total_seconds,
            candidate_frontier,
        });
    };

    let assumptions = if spec.conditional_independence_assumed {
        vec![
            "Probe outcomes are assumed conditionally independent given each claim; this permits exact joint-information calculations.".to_owned(),
            "Sensitivity and specificity are treated as calibrated inputs, not as facts proven by the planner.".to_owned(),
            "The multi-budget greedy plan is heuristic, although each step's information and Bayes-risk calculations are exact under the stated model.".to_owned(),
        ]
    } else {
        vec![
            "Conditional independence is not assumed; at most one probe is selected because combined information cannot be computed without dependence assumptions.".to_owned(),
            "Sensitivity and specificity are treated as calibrated inputs, not as facts proven by the planner.".to_owned(),
            "The multi-budget greedy plan is heuristic, although the selected step's information and Bayes-risk calculations are exact for a single probe.".to_owned(),
        ]
    };

    Ok(PlanReport {
        probe_metrics,
        single_probe_frontier,
        selected,
        total_information_bits,
        total_expected_risk_reduction,
        total_usd,
        total_seconds,
        stopped_reason,
        assumptions,
    })
}

#[derive(Debug, Clone, Copy)]
struct JointMetrics {
    information_bits: f64,
    risk_reduction: f64,
}

#[derive(Debug)]
struct ClaimState {
    selected_probe_indices: Vec<usize>,
    joint: JointMetrics,
    claim_id: String,
}

#[derive(Debug, Clone)]
struct Candidate {
    probe_index: usize,
    claim_index: usize,
    probe_id: String,
    marginal_information: f64,
    marginal_risk: f64,
    expected_usd: f64,
    expected_seconds: f64,
    new_joint: JointMetrics,
}

#[derive(Debug, Default)]
struct CompensatedSum {
    sum: f64,
    compensation: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64, context: &str) -> Result<(), PlanError> {
        if !value.is_finite() {
            return Err(PlanError::Numerical(format!(
                "{context} produced a non-finite term"
            )));
        }
        let adjusted = value - self.compensation;
        let next = self.sum + adjusted;
        if !next.is_finite() {
            return Err(PlanError::Numerical(format!(
                "{context} overflowed while summing"
            )));
        }
        self.compensation = (next - self.sum) - adjusted;
        self.sum = next;
        Ok(())
    }
}

fn validate_plan_spec(spec: &PlanSpec) -> Result<HashMap<String, usize>, PlanError> {
    validate_nonnegative_finite(spec.budget.max_usd, "budget.max_usd")?;
    validate_nonnegative_finite(spec.budget.max_seconds, "budget.max_seconds")?;
    if spec.budget.max_probes == 0 {
        return Err(PlanError::InvalidInput(
            "budget.max_probes must be greater than zero".to_owned(),
        ));
    }
    if spec.claims.is_empty() {
        return Err(PlanError::InvalidInput(
            "claims must contain at least one claim".to_owned(),
        ));
    }
    if spec.probes.is_empty() {
        return Err(PlanError::InvalidInput(
            "probes must contain at least one probe".to_owned(),
        ));
    }

    let mut claim_indices = HashMap::with_capacity(spec.claims.len());
    for (index, claim) in spec.claims.iter().enumerate() {
        validate_claim(claim, &format!("claims[{index}]"))?;
        if claim_indices.insert(claim.id.clone(), index).is_some() {
            return Err(PlanError::InvalidInput(format!(
                "duplicate claim id {:?}",
                claim.id
            )));
        }
    }

    let mut probe_ids = HashSet::with_capacity(spec.probes.len());
    for (index, probe) in spec.probes.iter().enumerate() {
        validate_probe(probe, &format!("probes[{index}]"))?;
        if !probe_ids.insert(probe.id.clone()) {
            return Err(PlanError::InvalidInput(format!(
                "duplicate probe id {:?}",
                probe.id
            )));
        }
        if !claim_indices.contains_key(&probe.claim_id) {
            return Err(PlanError::InvalidInput(format!(
                "probe {:?} refers to unknown claim {:?}",
                probe.id, probe.claim_id
            )));
        }
    }

    Ok(claim_indices)
}

fn validate_claim(claim: &ClaimSpec, path: &str) -> Result<(), PlanError> {
    validate_nonempty(&claim.id, &format!("{path}.id"))?;
    validate_probability(
        claim.prior_probability,
        &format!("{path}.prior_probability"),
    )?;
    validate_nonnegative_finite(
        claim.false_positive_loss,
        &format!("{path}.false_positive_loss"),
    )?;
    validate_nonnegative_finite(
        claim.false_negative_loss,
        &format!("{path}.false_negative_loss"),
    )?;
    validate_nonnegative_finite(claim.importance, &format!("{path}.importance"))?;
    Ok(())
}

fn validate_probe(probe: &ProbeSpec, path: &str) -> Result<(), PlanError> {
    validate_nonempty(&probe.id, &format!("{path}.id"))?;
    validate_nonempty(&probe.claim_id, &format!("{path}.claim_id"))?;
    validate_probability(probe.sensitivity, &format!("{path}.sensitivity"))?;
    validate_probability(probe.specificity, &format!("{path}.specificity"))?;
    validate_nonnegative_finite(probe.expected_usd, &format!("{path}.expected_usd"))?;
    validate_nonnegative_finite(probe.expected_seconds, &format!("{path}.expected_seconds"))?;
    validate_nonempty(&probe.dependence_group, &format!("{path}.dependence_group"))?;
    Ok(())
}

fn validate_nonempty(value: &str, field: &str) -> Result<(), PlanError> {
    if value.trim().is_empty() {
        Err(PlanError::InvalidInput(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_probability(value: f64, field: &str) -> Result<(), PlanError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        Err(PlanError::InvalidInput(format!(
            "{field} must be finite and in [0, 1], got {value}"
        )))
    } else {
        Ok(())
    }
}

fn validate_nonnegative_finite(value: f64, field: &str) -> Result<(), PlanError> {
    if !value.is_finite() || value < 0.0 {
        Err(PlanError::InvalidInput(format!(
            "{field} must be finite and nonnegative, got {value}"
        )))
    } else {
        Ok(())
    }
}

fn posterior_probability(
    true_joint_mass: f64,
    outcome_probability: f64,
    prior: f64,
    context: &str,
) -> Result<f64, PlanError> {
    if outcome_probability == 0.0 {
        // Conditioning on an impossible event is undefined. Returning the prior
        // is a harmless totalization because the event receives zero weight in
        // every expectation and preserves degenerate priors at zero and one.
        return Ok(prior);
    }
    let posterior = true_joint_mass / outcome_probability;
    unit_roundoff(posterior, context)
}

fn bayes_risk(probability_true: f64, claim: &ClaimSpec) -> Result<f64, PlanError> {
    let true_action_loss = checked_product(
        1.0 - probability_true,
        claim.false_positive_loss,
        "true-action Bayes loss",
    )?;
    let false_action_loss = checked_product(
        probability_true,
        claim.false_negative_loss,
        "false-action Bayes loss",
    )?;
    Ok(true_action_loss.min(false_action_loss))
}

fn bayes_action(probability_true: f64, claim: &ClaimSpec) -> Result<bool, PlanError> {
    let true_action_loss = checked_product(
        1.0 - probability_true,
        claim.false_positive_loss,
        "true-action decision loss",
    )?;
    let false_action_loss = checked_product(
        probability_true,
        claim.false_negative_loss,
        "false-action decision loss",
    )?;
    // Ties deterministically choose false.
    Ok(true_action_loss < false_action_loss)
}

fn checked_min_loss(
    false_mass: f64,
    false_positive_loss: f64,
    true_mass: f64,
    false_negative_loss: f64,
    context: &str,
) -> Result<f64, PlanError> {
    let true_action = checked_product(false_mass, false_positive_loss, context)?;
    let false_action = checked_product(true_mass, false_negative_loss, context)?;
    Ok(true_action.min(false_action))
}

fn rate_or_none(numerator: f64, denominator: f64, context: &str) -> Result<Option<f64>, PlanError> {
    if denominator == 0.0 {
        return Ok(None);
    }
    let rate = numerator / denominator;
    if rate.is_finite() {
        Ok(Some(rate))
    } else {
        Err(PlanError::Numerical(format!(
            "{context} is not representable as a finite f64"
        )))
    }
}

fn checked_product(left: f64, right: f64, context: &str) -> Result<f64, PlanError> {
    let product = left * right;
    if product.is_finite() {
        Ok(product)
    } else {
        Err(PlanError::Numerical(format!(
            "{context} overflowed or became non-finite"
        )))
    }
}

fn checked_add(left: f64, right: f64, context: &str) -> Result<f64, PlanError> {
    let sum = left + right;
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(PlanError::Numerical(format!(
            "{context} overflowed or became non-finite"
        )))
    }
}

fn unit_roundoff(value: f64, context: &str) -> Result<f64, PlanError> {
    if !value.is_finite() {
        return Err(PlanError::Numerical(format!("{context} became non-finite")));
    }
    let tolerance = ROUND_OFF_FACTOR * f64::EPSILON;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else if value >= -tolerance && value < 0.0 {
        Ok(0.0)
    } else if value > 1.0 && value <= 1.0 + tolerance {
        Ok(1.0)
    } else {
        Err(PlanError::Numerical(format!(
            "{context} fell outside [0, 1]: {value}"
        )))
    }
}

fn nonnegative_roundoff(value: f64, scale: f64, context: &str) -> Result<f64, PlanError> {
    if !value.is_finite() || !scale.is_finite() {
        return Err(PlanError::Numerical(format!("{context} became non-finite")));
    }
    if value >= 0.0 {
        return Ok(value);
    }
    let tolerance = ROUND_OFF_FACTOR * f64::EPSILON * scale.max(1.0);
    if value >= -tolerance {
        Ok(0.0)
    } else {
        Err(PlanError::Numerical(format!(
            "{context} was materially negative: {value}"
        )))
    }
}

fn joint_metrics(claim: &ClaimSpec, probes: &[&ProbeSpec]) -> Result<JointMetrics, PlanError> {
    if probes.len() > EXACT_ENUMERATION_LIMIT {
        return Err(PlanError::ExactEnumerationLimit {
            claim_id: claim.id.clone(),
            requested: probes.len(),
            limit: EXACT_ENUMERATION_LIMIT,
        });
    }

    let shift = match u32::try_from(probes.len()) {
        Ok(shift) => shift,
        Err(_) => {
            return Err(PlanError::ExactEnumerationLimit {
                claim_id: claim.id.clone(),
                requested: probes.len(),
                limit: EXACT_ENUMERATION_LIMIT,
            });
        }
    };
    let outcome_count = match 1usize.checked_shl(shift) {
        Some(count) => count,
        None => {
            return Err(PlanError::ExactEnumerationLimit {
                claim_id: claim.id.clone(),
                requested: probes.len(),
                limit: EXACT_ENUMERATION_LIMIT,
            });
        }
    };

    let prior = claim.prior_probability;
    let mut expected_entropy = CompensatedSum::default();
    let mut expected_risk = CompensatedSum::default();

    for outcome_mask in 0..outcome_count {
        let mut likelihood_if_true = 1.0;
        let mut likelihood_if_false = 1.0;
        for (position, probe) in probes.iter().enumerate() {
            let positive = (outcome_mask & (1usize << position)) != 0;
            let true_factor = if positive {
                probe.sensitivity
            } else {
                1.0 - probe.sensitivity
            };
            let false_factor = if positive {
                1.0 - probe.specificity
            } else {
                probe.specificity
            };
            likelihood_if_true *= true_factor;
            likelihood_if_false *= false_factor;
        }

        let true_mass = prior * likelihood_if_true;
        let false_mass = (1.0 - prior) * likelihood_if_false;
        let outcome_probability = checked_add(true_mass, false_mass, "joint outcome probability")?;
        let posterior = posterior_probability(
            true_mass,
            outcome_probability,
            prior,
            "joint posterior probability",
        )?;
        expected_entropy.add(
            outcome_probability * binary_entropy_bits(posterior),
            "joint expected posterior entropy",
        )?;
        expected_risk.add(
            checked_min_loss(
                false_mass,
                claim.false_positive_loss,
                true_mass,
                claim.false_negative_loss,
                "joint post-observation Bayes risk",
            )?,
            "joint expected post-observation Bayes risk",
        )?;
    }

    let prior_entropy = binary_entropy_bits(prior);
    let information_bits = nonnegative_roundoff(
        prior_entropy - expected_entropy.sum,
        prior_entropy.max(expected_entropy.sum),
        "joint mutual information",
    )?;
    let current_risk = bayes_risk(prior, claim)?;
    let risk_reduction = nonnegative_roundoff(
        current_risk - expected_risk.sum,
        current_risk.max(expected_risk.sum),
        "joint Bayes risk reduction",
    )?;

    Ok(JointMetrics {
        information_bits,
        risk_reduction,
    })
}

fn has_dependence_conflict(
    spec: &PlanSpec,
    state: &ClaimState,
    candidate: &ProbeSpec,
) -> Result<bool, PlanError> {
    for selected_index in &state.selected_probe_indices {
        let selected = match spec.probes.get(*selected_index) {
            Some(probe) => probe,
            None => {
                return Err(PlanError::Numerical(format!(
                    "selected probe for claim {:?} was unavailable",
                    state.claim_id
                )));
            }
        };
        if selected.dependence_group == candidate.dependence_group {
            return Ok(true);
        }
    }
    Ok(false)
}

fn fits_budget(current: f64, addition: f64, maximum: f64) -> bool {
    let proposed = current + addition;
    proposed.is_finite() && proposed <= maximum
}

fn weighted_joint_totals(
    spec: &PlanSpec,
    states: &[ClaimState],
) -> Result<JointMetrics, PlanError> {
    let mut information = CompensatedSum::default();
    let mut risk = CompensatedSum::default();
    for (claim_index, state) in states.iter().enumerate() {
        let claim = match spec.claims.get(claim_index) {
            Some(claim) => claim,
            None => {
                return Err(PlanError::Numerical(
                    "claim was unavailable while totaling the plan".to_owned(),
                ));
            }
        };
        information.add(
            checked_product(
                state.joint.information_bits,
                claim.importance,
                "importance-weighted joint information",
            )?,
            "total importance-weighted information",
        )?;
        risk.add(
            checked_product(
                state.joint.risk_reduction,
                claim.importance,
                "importance-weighted joint Bayes risk reduction",
            )?,
            "total importance-weighted Bayes risk reduction",
        )?;
    }
    Ok(JointMetrics {
        information_bits: information.sum,
        risk_reduction: risk.sum,
    })
}

fn single_probe_frontier(
    spec: &PlanSpec,
    metrics: &[ProbeMetrics],
    probe_claim_indices: &[usize],
) -> Result<Vec<String>, PlanError> {
    let mut values = Vec::with_capacity(metrics.len());
    for (probe_index, metric) in metrics.iter().enumerate() {
        let claim_index = match probe_claim_indices.get(probe_index) {
            Some(index) => *index,
            None => {
                return Err(PlanError::Numerical(
                    "probe claim index was unavailable while building the frontier".to_owned(),
                ));
            }
        };
        let claim = match spec.claims.get(claim_index) {
            Some(claim) => claim,
            None => {
                return Err(PlanError::Numerical(
                    "claim was unavailable while building the frontier".to_owned(),
                ));
            }
        };
        values.push(FrontierValue {
            id: metric.probe_id.clone(),
            information: checked_product(
                metric.information_gain_bits,
                claim.importance,
                "importance-weighted single-probe information",
            )?,
            risk: checked_product(
                metric.expected_bayes_risk_reduction,
                claim.importance,
                "importance-weighted single-probe Bayes risk reduction",
            )?,
            usd: metric.expected_usd,
            seconds: metric.expected_seconds,
        });
    }
    Ok(frontier_ids(&values))
}

#[derive(Debug)]
struct FrontierValue {
    id: String,
    information: f64,
    risk: f64,
    usd: f64,
    seconds: f64,
}

fn frontier_ids(values: &[FrontierValue]) -> Vec<String> {
    let mut frontier = Vec::new();
    for (index, value) in values.iter().enumerate() {
        // The no-probe baseline has zero information, zero decision value,
        // zero cost, and zero time. It dominates any probe that adds no value.
        if value.information <= 0.0 && value.risk <= 0.0 {
            continue;
        }
        let dominated = values.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.information >= value.information
                && other.risk >= value.risk
                && other.usd <= value.usd
                && other.seconds <= value.seconds
                && (other.information > value.information
                    || other.risk > value.risk
                    || other.usd < value.usd
                    || other.seconds < value.seconds)
        });
        if !dominated {
            frontier.push(value.id.clone());
        }
    }
    frontier.sort();
    frontier
}

fn candidate_frontier(candidates: &[Candidate]) -> Vec<String> {
    let values: Vec<FrontierValue> = candidates
        .iter()
        .map(|candidate| FrontierValue {
            id: candidate.probe_id.clone(),
            information: candidate.marginal_information,
            risk: candidate.marginal_risk,
            usd: candidate.expected_usd,
            seconds: candidate.expected_seconds,
        })
        .collect();
    frontier_ids(&values)
}

fn best_candidate_index(
    candidates: &[Candidate],
    strategy: &SelectionStrategy,
) -> Result<usize, PlanError> {
    let mut best_index = match candidates.first() {
        Some(_) => 0,
        None => {
            return Err(PlanError::Numerical(
                "cannot choose from an empty candidate set".to_owned(),
            ));
        }
    };
    for index in 1..candidates.len() {
        let candidate = match candidates.get(index) {
            Some(candidate) => candidate,
            None => {
                return Err(PlanError::Numerical(
                    "candidate was unavailable during selection".to_owned(),
                ));
            }
        };
        let best = match candidates.get(best_index) {
            Some(best) => best,
            None => {
                return Err(PlanError::Numerical(
                    "best candidate was unavailable during selection".to_owned(),
                ));
            }
        };
        if candidate_is_better(candidate, best, strategy) {
            best_index = index;
        }
    }
    Ok(best_index)
}

fn candidate_is_better(
    candidate: &Candidate,
    incumbent: &Candidate,
    strategy: &SelectionStrategy,
) -> bool {
    let primary = match strategy {
        SelectionStrategy::MaxInformation => candidate
            .marginal_information
            .total_cmp(&incumbent.marginal_information),
        SelectionStrategy::InformationPerUsd => ratio_order(
            candidate.marginal_information,
            candidate.expected_usd,
            incumbent.marginal_information,
            incumbent.expected_usd,
        ),
        SelectionStrategy::InformationPerSecond => ratio_order(
            candidate.marginal_information,
            candidate.expected_seconds,
            incumbent.marginal_information,
            incumbent.expected_seconds,
        ),
        SelectionStrategy::MaxRiskReduction => {
            candidate.marginal_risk.total_cmp(&incumbent.marginal_risk)
        }
    };
    if primary != Ordering::Equal {
        return primary == Ordering::Greater;
    }

    let information_order = candidate
        .marginal_information
        .total_cmp(&incumbent.marginal_information);
    if information_order != Ordering::Equal {
        return information_order == Ordering::Greater;
    }
    let risk_order = candidate.marginal_risk.total_cmp(&incumbent.marginal_risk);
    if risk_order != Ordering::Equal {
        return risk_order == Ordering::Greater;
    }
    let usd_order = incumbent.expected_usd.total_cmp(&candidate.expected_usd);
    if usd_order != Ordering::Equal {
        return usd_order == Ordering::Greater;
    }
    let seconds_order = incumbent
        .expected_seconds
        .total_cmp(&candidate.expected_seconds);
    if seconds_order != Ordering::Equal {
        return seconds_order == Ordering::Greater;
    }
    candidate.probe_id < incumbent.probe_id
}

fn ratio_order(
    candidate_value: f64,
    candidate_cost: f64,
    incumbent_value: f64,
    incumbent_cost: f64,
) -> Ordering {
    let candidate_free_positive = candidate_cost == 0.0 && candidate_value > 0.0;
    let incumbent_free_positive = incumbent_cost == 0.0 && incumbent_value > 0.0;
    match candidate_free_positive.cmp(&incumbent_free_positive) {
        Ordering::Equal => {
            if candidate_free_positive {
                Ordering::Equal
            } else {
                let candidate_ratio = if candidate_cost == 0.0 {
                    0.0
                } else {
                    candidate_value / candidate_cost
                };
                let incumbent_ratio = if incumbent_cost == 0.0 {
                    0.0
                } else {
                    incumbent_value / incumbent_cost
                };
                candidate_ratio.total_cmp(&incumbent_ratio)
            }
        }
        order => order,
    }
}

fn has_positive_strategy_value(candidate: &Candidate, strategy: &SelectionStrategy) -> bool {
    match strategy {
        SelectionStrategy::MaxInformation
        | SelectionStrategy::InformationPerUsd
        | SelectionStrategy::InformationPerSecond => candidate.marginal_information > 0.0,
        SelectionStrategy::MaxRiskReduction => candidate.marginal_risk > 0.0,
    }
}

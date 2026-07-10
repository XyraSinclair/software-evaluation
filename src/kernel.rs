//! Compositional execution for evidence-producing criterion programs.
//!
//! A receipt certifies what this kernel recorded for one attempted evaluation:
//! the supplied program descriptor and artifact identity, the applicability and
//! execution status, the program's estimate, resource use reported by the
//! program, kernel-measured wall time, the number of program invocations, and
//! (when successful) the digest of the serialized observation. A receipt does
//! not certify that an artifact digest identifies the claimed contents, that
//! evidence is true or complete, that reported non-time resources are accurate,
//! or that a criterion program is correct, calibrated, deterministic, or
//! appropriate for a decision. Program correctness and calibration remain
//! external responsibilities.
//!
//! Observation digests are SHA-256 hashes of the bytes produced by
//! [`serde_json::to_vec`]. This is deterministic for a given in-memory value and
//! serializer version, but the kernel does not claim canonical JSON key
//! ordering. A future canonical serializer can replace this scheme by versioning
//! the receipt protocol.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactSnapshot {
    pub id: String,
    pub root: PathBuf,
    pub revision: String,
    pub tree_digest: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionSpec {
    pub id: String,
    pub description: String,
    pub claim_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeliefState {
    pub probabilities: BTreeMap<String, f64>,
    pub observation_digests: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceVector {
    pub usd: f64,
    pub wall_time_ms: u64,
    pub cpu_time_ms: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub programs: u32,
}

impl ResourceVector {
    pub const fn zero() -> Self {
        Self {
            usd: 0.0,
            wall_time_ms: 0,
            cpu_time_ms: None,
            peak_memory_bytes: None,
            bytes_read: 0,
            bytes_written: 0,
            programs: 0,
        }
    }

    pub fn validate(&self) -> Result<(), ResourceValidationError> {
        validate_usd(self.usd)
    }

    /// Aggregates sequential resource use without collapsing its dimensions.
    ///
    /// Additive counters are summed with overflow checks. Peak memory is the
    /// maximum of the two known peaks. An optional dimension remains known only
    /// when both inputs measured it.
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        if self.validate().is_err() || other.validate().is_err() {
            return None;
        }

        let usd = self.usd + other.usd;
        if !usd.is_finite() {
            return None;
        }

        Some(Self {
            usd,
            wall_time_ms: self.wall_time_ms.checked_add(other.wall_time_ms)?,
            cpu_time_ms: checked_optional_add(self.cpu_time_ms, other.cpu_time_ms)?,
            peak_memory_bytes: checked_optional_max(
                self.peak_memory_bytes,
                other.peak_memory_bytes,
            ),
            bytes_read: self.bytes_read.checked_add(other.bytes_read)?,
            bytes_written: self.bytes_written.checked_add(other.bytes_written)?,
            programs: self.programs.checked_add(other.programs)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ResourceValidationError {
    #[error("USD must be finite and nonnegative")]
    InvalidUsd,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceBudget {
    pub max_usd: f64,
    pub max_wall_time_ms: u64,
    pub max_programs: u32,
}

impl ResourceBudget {
    pub fn validate(&self) -> Result<(), KernelError> {
        validate_usd(self.max_usd).map_err(|error| KernelError::InvalidBudget(error.to_string()))
    }

    pub fn remaining(&self) -> Result<RemainingResources, KernelError> {
        RemainingResources::from_budget(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemainingResources {
    pub usd: f64,
    pub wall_time_ms: u64,
    pub programs: u32,
}

impl RemainingResources {
    pub fn from_budget(budget: &ResourceBudget) -> Result<Self, KernelError> {
        budget.validate()?;
        Ok(Self {
            usd: budget.max_usd,
            wall_time_ms: budget.max_wall_time_ms,
            programs: budget.max_programs,
        })
    }

    pub fn validate(&self) -> Result<(), ResourceValidationError> {
        validate_usd(self.usd)
    }

    pub fn decremented_by(&self, actual: &ResourceVector) -> Result<Self, ResourceValidationError> {
        self.validate()?;
        actual.validate()?;
        Ok(self.decrement_validated(actual))
    }

    fn decrement_validated(&self, actual: &ResourceVector) -> Self {
        Self {
            usd: if actual.usd >= self.usd {
                0.0
            } else {
                self.usd - actual.usd
            },
            wall_time_ms: self.wall_time_ms.saturating_sub(actual.wall_time_ms),
            programs: self.programs.saturating_sub(actual.programs),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramDescriptor {
    pub id: String,
    pub version: String,
    pub criterion: String,
    pub epistemic_class: EpistemicClass,
    pub deterministic: bool,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicClass {
    Exact,
    Proxy,
    Judgment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Applicability {
    Applicable,
    Inapplicable { reason: String },
    Unsupported { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceItem {
    pub kind: String,
    pub locator: String,
    pub digest: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeliefUpdate {
    pub claim_id: String,
    pub posterior_probability: f64,
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgramOutput {
    pub observation: Value,
    pub evidence: Vec<EvidenceItem>,
    pub belief_updates: Vec<BeliefUpdate>,
    pub resources: ResourceVector,
    pub continuation_hints: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgramStatus {
    Completed,
    Inapplicable,
    Unsupported,
    BudgetBlocked,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgramReceipt {
    pub program: ProgramDescriptor,
    pub artifact_id: String,
    pub artifact_tree_digest: String,
    pub started_unix_ms: u128,
    pub elapsed_ms: u64,
    pub estimated_resources: ResourceVector,
    pub actual_resources: ResourceVector,
    pub observation_digest: Option<String>,
    pub status: ProgramStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    BudgetExhausted,
    ProgramLimit,
    ProgramFailed,
    Inapplicable,
    Unsupported,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Continuation {
    Continue {
        remaining: RemainingResources,
        hints: Vec<String>,
    },
    Stop {
        remaining: RemainingResources,
        reason: StopReason,
        message: String,
    },
}

impl Continuation {
    pub fn remaining(&self) -> &RemainingResources {
        match self {
            Self::Continue { remaining, .. } | Self::Stop { remaining, .. } => remaining,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationStep {
    pub observation: Option<Value>,
    pub evidence: Vec<EvidenceItem>,
    pub receipt: ProgramReceipt,
    pub posterior: BeliefState,
    pub continuation: Continuation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationRun {
    pub artifact: ArtifactSnapshot,
    pub decision: DecisionSpec,
    pub steps: Vec<EvaluationStep>,
    pub posterior: BeliefState,
    pub remaining: RemainingResources,
    pub stopped_reason: StopReason,
}

pub trait CriterionProgram: Send + Sync {
    fn descriptor(&self) -> ProgramDescriptor;
    fn applicability(&self, artifact: &ArtifactSnapshot) -> Applicability;
    fn estimate(&self, artifact: &ArtifactSnapshot) -> Result<ResourceVector, ProgramFailure>;
    fn run(&self, context: &ProgramContext<'_>) -> Result<ProgramOutput, ProgramFailure>;
}

#[derive(Debug, Clone, Copy)]
pub struct ProgramContext<'a> {
    pub artifact: &'a ArtifactSnapshot,
    pub evidence: &'a BeliefState,
    pub decision: &'a DecisionSpec,
    pub remaining: &'a RemainingResources,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Error)]
#[error("{kind}: {message}")]
pub struct ProgramFailure {
    pub kind: ProgramFailureKind,
    pub message: String,
}

impl ProgramFailure {
    pub fn new(kind: ProgramFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ProgramFailureKind::InvalidInput, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ProgramFailureKind::Io, message)
    }

    pub fn tool(message: impl Into<String>) -> Self {
        Self::new(ProgramFailureKind::Tool, message)
    }

    pub fn invariant(message: impl Into<String>) -> Self {
        Self::new(ProgramFailureKind::Invariant, message)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Error)]
#[serde(rename_all = "snake_case")]
pub enum ProgramFailureKind {
    #[error("invalid input")]
    InvalidInput,
    #[error("I/O")]
    Io,
    #[error("tool")]
    Tool,
    #[error("invariant")]
    Invariant,
}

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("invalid artifact: {0}")]
    InvalidArtifact(String),
    #[error("invalid decision: {0}")]
    InvalidDecision(String),
    #[error("invalid resource budget: {0}")]
    InvalidBudget(String),
    #[error("invalid belief state: {0}")]
    InvalidBeliefs(String),
    #[error("invalid criterion program descriptor: {0}")]
    InvalidProgramDescriptor(String),
    #[error("invalid program output: {0}")]
    InvalidProgramOutput(String),
    #[error("invalid program pipeline: {0}")]
    InvalidPrograms(String),
    #[error("observation serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    #[error("system clock failed: {0}")]
    Clock(String),
}

/// Evaluates one criterion program against the supplied evidence and resources.
pub fn evaluate(
    artifact: &ArtifactSnapshot,
    criterion_program: &dyn CriterionProgram,
    evidence_already_known: &BeliefState,
    decision_being_supported: &DecisionSpec,
    resources_still_available: &RemainingResources,
) -> Result<EvaluationStep, KernelError> {
    validate_artifact(artifact)?;
    validate_decision(decision_being_supported)?;
    validate_beliefs(evidence_already_known)?;
    validate_remaining(resources_still_available)?;

    let descriptor = criterion_program.descriptor();
    validate_descriptor(&descriptor)?;

    evaluate_validated(
        artifact,
        criterion_program,
        descriptor,
        evidence_already_known,
        decision_being_supported,
        resources_still_available,
    )
}

/// Sequentially composes criterion programs without combining observations into
/// a score or a shared observation shape.
pub fn evaluate_pipeline(
    artifact: &ArtifactSnapshot,
    programs: &[&dyn CriterionProgram],
    evidence: &BeliefState,
    decision: &DecisionSpec,
    budget: &ResourceBudget,
) -> Result<EvaluationRun, KernelError> {
    if programs.is_empty() {
        return Err(KernelError::InvalidPrograms(
            "at least one criterion program is required".to_owned(),
        ));
    }

    validate_artifact(artifact)?;
    validate_decision(decision)?;
    validate_beliefs(evidence)?;
    budget.validate()?;

    let mut descriptors = Vec::with_capacity(programs.len());
    for program in programs {
        let descriptor = program.descriptor();
        validate_descriptor(&descriptor)?;
        descriptors.push(descriptor);
    }

    let mut posterior = evidence.clone();
    let mut remaining = RemainingResources::from_budget(budget)?;
    let mut steps = Vec::with_capacity(programs.len());
    let mut stopped_reason = None;

    for (index, (program, descriptor)) in programs.iter().zip(descriptors).enumerate() {
        let remaining_before = remaining.clone();
        let mut step = evaluate_validated(
            artifact, *program, descriptor, &posterior, decision, &remaining,
        )?;

        let is_last = index + 1 == programs.len();
        if is_last && step.receipt.status == ProgramStatus::Completed {
            let actual = &step.receipt.actual_resources;
            let overran_resources = actual.usd > remaining_before.usd
                || actual.wall_time_ms > remaining_before.wall_time_ms
                || actual.programs > remaining_before.programs;

            if !overran_resources {
                let completed_remaining = step.continuation.remaining().clone();
                step.continuation = Continuation::Stop {
                    remaining: completed_remaining,
                    reason: StopReason::Complete,
                    message: "all criterion programs completed".to_owned(),
                };
            }
        }

        posterior = step.posterior.clone();
        remaining = step.continuation.remaining().clone();

        let step_stop_reason = match &step.continuation {
            Continuation::Continue { .. } => None,
            Continuation::Stop { reason, .. } => Some(*reason),
        };
        steps.push(step);

        if let Some(reason) = step_stop_reason {
            stopped_reason = Some(reason);
            break;
        }
    }

    Ok(EvaluationRun {
        artifact: artifact.clone(),
        decision: decision.clone(),
        steps,
        posterior,
        remaining,
        stopped_reason: stopped_reason.unwrap_or(StopReason::Complete),
    })
}

fn evaluate_validated(
    artifact: &ArtifactSnapshot,
    program: &dyn CriterionProgram,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    decision: &DecisionSpec,
    remaining: &RemainingResources,
) -> Result<EvaluationStep, KernelError> {
    let started_unix_ms = unix_time_ms()?;

    match program.applicability(artifact) {
        Applicability::Inapplicable { reason } => {
            if reason.trim().is_empty() {
                return Ok(nonexecuted_failure_step(
                    artifact,
                    descriptor,
                    evidence,
                    remaining,
                    started_unix_ms,
                    ResourceVector::zero(),
                    ProgramFailure::invariant(
                        "applicability returned an inapplicable result without a reason",
                    ),
                ));
            }

            let message = format!("program is inapplicable: {reason}");
            Ok(nonexecuted_stop_step(
                artifact,
                descriptor,
                evidence,
                remaining,
                started_unix_ms,
                ProgramStatus::Inapplicable,
                StopReason::Inapplicable,
                message,
            ))
        }
        Applicability::Unsupported { reason } => {
            if reason.trim().is_empty() {
                return Ok(nonexecuted_failure_step(
                    artifact,
                    descriptor,
                    evidence,
                    remaining,
                    started_unix_ms,
                    ResourceVector::zero(),
                    ProgramFailure::invariant(
                        "applicability returned an unsupported result without a reason",
                    ),
                ));
            }

            let message = format!("program is unsupported: {reason}");
            Ok(nonexecuted_stop_step(
                artifact,
                descriptor,
                evidence,
                remaining,
                started_unix_ms,
                ProgramStatus::Unsupported,
                StopReason::Unsupported,
                message,
            ))
        }
        Applicability::Applicable => {
            let estimate = match program.estimate(artifact) {
                Ok(resources) => resources,
                Err(failure) => {
                    return Ok(nonexecuted_failure_step(
                        artifact,
                        descriptor,
                        evidence,
                        remaining,
                        started_unix_ms,
                        ResourceVector::zero(),
                        normalize_failure(failure, "resource estimation"),
                    ));
                }
            };

            let estimated_resources = match resources_with_program_invocation(estimate) {
                Ok(resources) => resources,
                Err(message) => {
                    return Ok(nonexecuted_failure_step(
                        artifact,
                        descriptor,
                        evidence,
                        remaining,
                        started_unix_ms,
                        ResourceVector::zero(),
                        ProgramFailure::invariant(format!("invalid resource estimate: {message}")),
                    ));
                }
            };

            if estimated_resources.programs > remaining.programs {
                let message = format!(
                    "program estimates {} invocation(s), but only {} remain",
                    estimated_resources.programs, remaining.programs
                );
                return Ok(budget_blocked_step(
                    artifact,
                    descriptor,
                    evidence,
                    remaining,
                    started_unix_ms,
                    estimated_resources,
                    StopReason::ProgramLimit,
                    message,
                ));
            }

            if estimated_resources.usd > remaining.usd
                || estimated_resources.wall_time_ms > remaining.wall_time_ms
            {
                let message = format!(
                    "program estimate exceeds remaining budget (estimated ${:.6} and {} ms; remaining ${:.6} and {} ms)",
                    estimated_resources.usd,
                    estimated_resources.wall_time_ms,
                    remaining.usd,
                    remaining.wall_time_ms
                );
                return Ok(budget_blocked_step(
                    artifact,
                    descriptor,
                    evidence,
                    remaining,
                    started_unix_ms,
                    estimated_resources,
                    StopReason::BudgetExhausted,
                    message,
                ));
            }

            execute_program(
                artifact,
                program,
                descriptor,
                evidence,
                decision,
                remaining,
                started_unix_ms,
                estimated_resources,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_program(
    artifact: &ArtifactSnapshot,
    program: &dyn CriterionProgram,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    decision: &DecisionSpec,
    remaining: &RemainingResources,
    started_unix_ms: u128,
    estimated_resources: ResourceVector,
) -> Result<EvaluationStep, KernelError> {
    let context = ProgramContext {
        artifact,
        evidence,
        decision,
        remaining,
    };
    let timer = Instant::now();
    let result = program.run(&context);
    let elapsed_ms = measured_elapsed_ms(timer);

    match result {
        Err(failure) => {
            let actual_resources = executed_failure_resources(elapsed_ms);
            let next_remaining = remaining.decrement_validated(&actual_resources);
            let failure = normalize_failure(failure, "program execution");
            let message = failure.to_string();
            Ok(EvaluationStep {
                observation: None,
                evidence: Vec::new(),
                receipt: receipt(
                    artifact,
                    descriptor,
                    started_unix_ms,
                    elapsed_ms,
                    estimated_resources,
                    actual_resources,
                    None,
                    ProgramStatus::Failed,
                    Some(message.clone()),
                ),
                posterior: evidence.clone(),
                continuation: Continuation::Stop {
                    remaining: next_remaining,
                    reason: StopReason::ProgramFailed,
                    message,
                },
            })
        }
        Ok(output) => finish_program_output(
            artifact,
            descriptor,
            evidence,
            decision,
            remaining,
            started_unix_ms,
            elapsed_ms,
            estimated_resources,
            output,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_program_output(
    artifact: &ArtifactSnapshot,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    decision: &DecisionSpec,
    remaining: &RemainingResources,
    started_unix_ms: u128,
    elapsed_ms: u64,
    estimated_resources: ResourceVector,
    output: ProgramOutput,
) -> Result<EvaluationStep, KernelError> {
    let actual_resources = match actual_resources(output.resources.clone(), elapsed_ms) {
        Ok(resources) => resources,
        Err(message) => {
            return Ok(executed_invariant_failure_step(
                artifact,
                descriptor,
                evidence,
                remaining,
                started_unix_ms,
                elapsed_ms,
                estimated_resources,
                executed_failure_resources(elapsed_ms),
                format!("invalid program resources: {message}"),
            ));
        }
    };

    if let Err(message) = validate_program_output(&output, decision) {
        return Ok(executed_invariant_failure_step(
            artifact,
            descriptor,
            evidence,
            remaining,
            started_unix_ms,
            elapsed_ms,
            estimated_resources,
            actual_resources,
            message,
        ));
    }

    let observation_bytes =
        serde_json::to_vec(&output.observation).map_err(KernelError::Serialization)?;
    let observation_digest = sha256_hex(&observation_bytes);

    let mut posterior = evidence.clone();
    for update in &output.belief_updates {
        posterior
            .probabilities
            .insert(update.claim_id.clone(), update.posterior_probability);
    }
    posterior
        .observation_digests
        .push(observation_digest.clone());

    let next_remaining = remaining.decrement_validated(&actual_resources);
    let exceeded_budget = actual_resources.usd > remaining.usd
        || actual_resources.wall_time_ms > remaining.wall_time_ms;
    let exceeded_program_limit = actual_resources.programs > remaining.programs;

    let continuation = if exceeded_budget {
        Continuation::Stop {
            remaining: next_remaining.clone(),
            reason: StopReason::BudgetExhausted,
            message: "program completed, but actual resource use exceeded the remaining budget"
                .to_owned(),
        }
    } else if exceeded_program_limit || next_remaining.programs == 0 {
        Continuation::Stop {
            remaining: next_remaining.clone(),
            reason: StopReason::ProgramLimit,
            message: "program completed and no further program invocations remain".to_owned(),
        }
    } else if (actual_resources.usd > 0.0 && next_remaining.usd == 0.0)
        || (actual_resources.wall_time_ms > 0 && next_remaining.wall_time_ms == 0)
    {
        Continuation::Stop {
            remaining: next_remaining.clone(),
            reason: StopReason::BudgetExhausted,
            message: "program completed and exhausted a remaining budget dimension".to_owned(),
        }
    } else {
        Continuation::Continue {
            remaining: next_remaining.clone(),
            hints: output.continuation_hints.clone(),
        }
    };

    let receipt_message = if output.limitations.is_empty() {
        None
    } else {
        Some(format!("limitations: {}", output.limitations.join("; ")))
    };

    Ok(EvaluationStep {
        observation: Some(output.observation),
        evidence: output.evidence,
        receipt: receipt(
            artifact,
            descriptor,
            started_unix_ms,
            elapsed_ms,
            estimated_resources,
            actual_resources,
            Some(observation_digest),
            ProgramStatus::Completed,
            receipt_message,
        ),
        posterior,
        continuation,
    })
}

#[allow(clippy::too_many_arguments)]
fn executed_invariant_failure_step(
    artifact: &ArtifactSnapshot,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    remaining: &RemainingResources,
    started_unix_ms: u128,
    elapsed_ms: u64,
    estimated_resources: ResourceVector,
    actual_resources: ResourceVector,
    detail: String,
) -> EvaluationStep {
    let failure = ProgramFailure::invariant(format!("invalid program output: {detail}"));
    let message = failure.to_string();
    let next_remaining = remaining.decrement_validated(&actual_resources);
    EvaluationStep {
        observation: None,
        evidence: Vec::new(),
        receipt: receipt(
            artifact,
            descriptor,
            started_unix_ms,
            elapsed_ms,
            estimated_resources,
            actual_resources,
            None,
            ProgramStatus::Failed,
            Some(message.clone()),
        ),
        posterior: evidence.clone(),
        continuation: Continuation::Stop {
            remaining: next_remaining,
            reason: StopReason::ProgramFailed,
            message,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn nonexecuted_stop_step(
    artifact: &ArtifactSnapshot,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    remaining: &RemainingResources,
    started_unix_ms: u128,
    status: ProgramStatus,
    reason: StopReason,
    message: String,
) -> EvaluationStep {
    EvaluationStep {
        observation: None,
        evidence: Vec::new(),
        receipt: receipt(
            artifact,
            descriptor,
            started_unix_ms,
            0,
            ResourceVector::zero(),
            ResourceVector::zero(),
            None,
            status,
            Some(message.clone()),
        ),
        posterior: evidence.clone(),
        continuation: Continuation::Stop {
            remaining: remaining.clone(),
            reason,
            message,
        },
    }
}

fn nonexecuted_failure_step(
    artifact: &ArtifactSnapshot,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    remaining: &RemainingResources,
    started_unix_ms: u128,
    estimated_resources: ResourceVector,
    failure: ProgramFailure,
) -> EvaluationStep {
    let message = failure.to_string();
    EvaluationStep {
        observation: None,
        evidence: Vec::new(),
        receipt: receipt(
            artifact,
            descriptor,
            started_unix_ms,
            0,
            estimated_resources,
            ResourceVector::zero(),
            None,
            ProgramStatus::Failed,
            Some(message.clone()),
        ),
        posterior: evidence.clone(),
        continuation: Continuation::Stop {
            remaining: remaining.clone(),
            reason: StopReason::ProgramFailed,
            message,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn budget_blocked_step(
    artifact: &ArtifactSnapshot,
    descriptor: ProgramDescriptor,
    evidence: &BeliefState,
    remaining: &RemainingResources,
    started_unix_ms: u128,
    estimated_resources: ResourceVector,
    reason: StopReason,
    message: String,
) -> EvaluationStep {
    EvaluationStep {
        observation: None,
        evidence: Vec::new(),
        receipt: receipt(
            artifact,
            descriptor,
            started_unix_ms,
            0,
            estimated_resources,
            ResourceVector::zero(),
            None,
            ProgramStatus::BudgetBlocked,
            Some(message.clone()),
        ),
        posterior: evidence.clone(),
        continuation: Continuation::Stop {
            remaining: remaining.clone(),
            reason,
            message,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn receipt(
    artifact: &ArtifactSnapshot,
    program: ProgramDescriptor,
    started_unix_ms: u128,
    elapsed_ms: u64,
    estimated_resources: ResourceVector,
    actual_resources: ResourceVector,
    observation_digest: Option<String>,
    status: ProgramStatus,
    message: Option<String>,
) -> ProgramReceipt {
    ProgramReceipt {
        program,
        artifact_id: artifact.id.clone(),
        artifact_tree_digest: artifact.tree_digest.clone(),
        started_unix_ms,
        elapsed_ms,
        estimated_resources,
        actual_resources,
        observation_digest,
        status,
        message,
    }
}

fn validate_artifact(artifact: &ArtifactSnapshot) -> Result<(), KernelError> {
    require_nonempty(&artifact.id, "artifact id").map_err(KernelError::InvalidArtifact)?;
    require_nonempty(&artifact.revision, "artifact revision")
        .map_err(KernelError::InvalidArtifact)?;
    require_nonempty(&artifact.tree_digest, "artifact tree digest")
        .map_err(KernelError::InvalidArtifact)?;
    require_nonempty(&artifact.kind, "artifact kind").map_err(KernelError::InvalidArtifact)?;
    Ok(())
}

fn validate_decision(decision: &DecisionSpec) -> Result<(), KernelError> {
    require_nonempty(&decision.id, "decision id").map_err(KernelError::InvalidDecision)?;
    if decision.claim_ids.is_empty() {
        return Err(KernelError::InvalidDecision(
            "at least one claim id is required".to_owned(),
        ));
    }

    let mut claim_ids = BTreeSet::new();
    for claim_id in &decision.claim_ids {
        require_nonempty(claim_id, "decision claim id").map_err(KernelError::InvalidDecision)?;
        if !claim_ids.insert(claim_id.as_str()) {
            return Err(KernelError::InvalidDecision(format!(
                "duplicate claim id `{claim_id}`"
            )));
        }
    }
    Ok(())
}

fn validate_beliefs(beliefs: &BeliefState) -> Result<(), KernelError> {
    for (claim_id, probability) in &beliefs.probabilities {
        require_nonempty(claim_id, "belief claim id").map_err(KernelError::InvalidBeliefs)?;
        if !valid_probability(*probability) {
            return Err(KernelError::InvalidBeliefs(format!(
                "probability for claim `{claim_id}` must be finite and in [0, 1]"
            )));
        }
    }

    for digest in &beliefs.observation_digests {
        require_nonempty(digest, "observation digest").map_err(KernelError::InvalidBeliefs)?;
    }
    Ok(())
}

fn validate_remaining(remaining: &RemainingResources) -> Result<(), KernelError> {
    remaining
        .validate()
        .map_err(|error| KernelError::InvalidBudget(format!("remaining resources: {error}")))
}

fn validate_descriptor(descriptor: &ProgramDescriptor) -> Result<(), KernelError> {
    require_nonempty(&descriptor.id, "program id")
        .map_err(KernelError::InvalidProgramDescriptor)?;
    require_nonempty(&descriptor.version, "program version")
        .map_err(KernelError::InvalidProgramDescriptor)?;
    require_nonempty(&descriptor.criterion, "program criterion")
        .map_err(KernelError::InvalidProgramDescriptor)?;
    require_nonempty(&descriptor.description, "program description")
        .map_err(KernelError::InvalidProgramDescriptor)?;
    Ok(())
}

fn validate_program_output(output: &ProgramOutput, decision: &DecisionSpec) -> Result<(), String> {
    output
        .resources
        .validate()
        .map_err(|error| error.to_string())?;

    for (index, item) in output.evidence.iter().enumerate() {
        require_nonempty(&item.kind, "evidence kind")
            .map_err(|message| format!("evidence item {index}: {message}"))?;
        require_nonempty(&item.locator, "evidence locator")
            .map_err(|message| format!("evidence item {index}: {message}"))?;
        require_nonempty(&item.description, "evidence description")
            .map_err(|message| format!("evidence item {index}: {message}"))?;
        if let Some(digest) = &item.digest {
            require_nonempty(digest, "evidence digest")
                .map_err(|message| format!("evidence item {index}: {message}"))?;
        }
    }

    let declared_claims: BTreeSet<&str> = decision.claim_ids.iter().map(String::as_str).collect();
    let mut updated_claims = BTreeSet::new();
    for update in &output.belief_updates {
        require_nonempty(&update.claim_id, "belief update claim id")?;
        if !declared_claims.contains(update.claim_id.as_str()) {
            return Err(format!(
                "belief update refers to undeclared claim `{}`",
                update.claim_id
            ));
        }
        if !updated_claims.insert(update.claim_id.as_str()) {
            return Err(format!("belief update repeats claim `{}`", update.claim_id));
        }
        if !valid_probability(update.posterior_probability) {
            return Err(format!(
                "posterior probability for claim `{}` must be finite and in [0, 1]",
                update.claim_id
            ));
        }
        require_nonempty(&update.basis, "belief update basis")?;
    }

    for (index, hint) in output.continuation_hints.iter().enumerate() {
        require_nonempty(hint, "continuation hint")
            .map_err(|message| format!("continuation hint {index}: {message}"))?;
    }
    for (index, limitation) in output.limitations.iter().enumerate() {
        require_nonempty(limitation, "limitation")
            .map_err(|message| format!("limitation {index}: {message}"))?;
    }
    Ok(())
}

fn resources_with_program_invocation(
    mut resources: ResourceVector,
) -> Result<ResourceVector, String> {
    resources.validate().map_err(|error| error.to_string())?;
    resources.programs = resources
        .programs
        .checked_add(1)
        .ok_or_else(|| "program count overflowed while accounting for invocation".to_owned())?;
    Ok(resources)
}

fn actual_resources(resources: ResourceVector, elapsed_ms: u64) -> Result<ResourceVector, String> {
    let mut resources = resources_with_program_invocation(resources)?;
    resources.wall_time_ms = elapsed_ms;
    Ok(resources)
}

fn executed_failure_resources(elapsed_ms: u64) -> ResourceVector {
    ResourceVector {
        wall_time_ms: elapsed_ms,
        programs: 1,
        ..ResourceVector::zero()
    }
}

fn normalize_failure(failure: ProgramFailure, phase: &str) -> ProgramFailure {
    if failure.message.trim().is_empty() {
        ProgramFailure::invariant(format!(
            "{phase} failed without an actionable failure message"
        ))
    } else {
        failure
    }
}

fn valid_probability(probability: f64) -> bool {
    probability.is_finite() && (0.0..=1.0).contains(&probability)
}

fn require_nonempty(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_usd(usd: f64) -> Result<(), ResourceValidationError> {
    if usd.is_finite() && usd >= 0.0 {
        Ok(())
    } else {
        Err(ResourceValidationError::InvalidUsd)
    }
}

fn checked_optional_add(left: Option<u64>, right: Option<u64>) -> Option<Option<u64>> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right).map(Some),
        _ => Some(None),
    }
}

fn checked_optional_max(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        _ => None,
    }
}

fn measured_elapsed_ms(started: Instant) -> u64 {
    let elapsed = started.elapsed();
    if elapsed.is_zero() {
        return 0;
    }

    let elapsed_ms = elapsed.as_millis();
    if elapsed_ms == 0 {
        1
    } else {
        u64::try_from(elapsed_ms).unwrap_or(u64::MAX)
    }
}

fn unix_time_ms() -> Result<u128, KernelError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| KernelError::Clock(error.to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

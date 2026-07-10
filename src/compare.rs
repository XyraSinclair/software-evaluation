//! Transparent, matched comparisons of completed evaluation runs.
//!
//! This module deliberately preserves every criterion as a separate set of
//! numeric differences. It does not assign weights, infer whether a change is
//! desirable, or produce an overall score.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::kernel::{EvaluationRun, ProgramDescriptor, ProgramStatus, StopReason};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Error)]
pub enum CompareError {
    #[error(
        "cannot compare the identical artifact revision `{revision}` with tree digest `{tree_digest}`"
    )]
    IdenticalArtifact {
        revision: String,
        tree_digest: String,
    },
    #[error("cannot compare incomplete {side} evaluation run (stop reason: {reason:?})")]
    IncompleteRun { side: String, reason: StopReason },

    #[error("incompatible program sequences: both runs must contain at least one evaluation step")]
    EmptyProgramSequence,

    #[error(
        "incompatible program sequences: left run has {left_count} steps but right run has {right_count} steps"
    )]
    ProgramCountMismatch {
        left_count: usize,
        right_count: usize,
    },

    #[error(
        "incompatible program sequences at step {step_index}: descriptor field `{field}` differs (left `{left}`, right `{right}`)"
    )]
    ProgramDescriptorMismatch {
        step_index: usize,
        field: String,
        left: String,
        right: String,
    },

    #[error(
        "program `{program_id}` version `{program_version}` on {side} did not complete (status: {status:?})"
    )]
    IncompleteProgram {
        program_id: String,
        program_version: String,
        side: String,
        status: ProgramStatus,
    },

    #[error(
        "program `{program_id}` version `{program_version}` on {side} completed without an observation"
    )]
    MissingObservation {
        program_id: String,
        program_version: String,
        side: String,
    },

    #[error(
        "program `{program_id}` version `{program_version}` has unmatched numeric observation paths (left only: {left_only:?}, right only: {right_only:?})"
    )]
    UnmatchedNumericPaths {
        program_id: String,
        program_version: String,
        left_only: Vec<String>,
        right_only: Vec<String>,
    },

    #[error(
        "program `{program_id}` version `{program_version}` on {side} has an invalid or nonfinite numeric value `{value}` at JSON Pointer `{path}`"
    )]
    InvalidNumericValue {
        program_id: String,
        program_version: String,
        side: String,
        path: String,
        value: String,
    },

    #[error(
        "program `{program_id}` version `{program_version}` produced a nonfinite right-minus-left difference at JSON Pointer `{path}` (left {left}, right {right})"
    )]
    NonFiniteDifference {
        program_id: String,
        program_version: String,
        path: String,
        left: f64,
        right: f64,
    },

    #[error(
        "program `{program_id}` version `{program_version}` on {side} produced duplicate numeric JSON Pointer `{path}`"
    )]
    DuplicateNumericPath {
        program_id: String,
        program_version: String,
        side: String,
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NumericDifference {
    pub path: String,
    pub left: f64,
    pub right: f64,
    pub right_minus_left: f64,
    pub relative_change_from_left: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgramDifference {
    pub program_id: String,
    pub program_version: String,
    pub criterion: String,
    pub differences: Vec<NumericDifference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationComparison {
    pub left_artifact_id: String,
    pub right_artifact_id: String,
    pub programs: Vec<ProgramDifference>,
    pub limitations: Vec<String>,
}

/// Compares numeric observation leaves from two runs of the same ordered
/// criterion-program sequence.
pub fn compare_evaluation_runs(
    left: &EvaluationRun,
    right: &EvaluationRun,
) -> Result<EvaluationComparison, CompareError> {
    if left.stopped_reason != StopReason::Complete {
        return Err(CompareError::IncompleteRun {
            side: "left".to_owned(),
            reason: left.stopped_reason,
        });
    }
    if right.stopped_reason != StopReason::Complete {
        return Err(CompareError::IncompleteRun {
            side: "right".to_owned(),
            reason: right.stopped_reason,
        });
    }

    if left.artifact.revision == right.artifact.revision
        && left.artifact.tree_digest == right.artifact.tree_digest
    {
        return Err(CompareError::IdenticalArtifact {
            revision: left.artifact.revision.clone(),
            tree_digest: left.artifact.tree_digest.clone(),
        });
    }

    if left.steps.is_empty() && right.steps.is_empty() {
        return Err(CompareError::EmptyProgramSequence);
    }

    if left.steps.len() != right.steps.len() {
        return Err(CompareError::ProgramCountMismatch {
            left_count: left.steps.len(),
            right_count: right.steps.len(),
        });
    }

    let mut programs = Vec::with_capacity(left.steps.len());

    for (step_index, (left_step, right_step)) in left.steps.iter().zip(&right.steps).enumerate() {
        ensure_matching_descriptors(
            step_index,
            &left_step.receipt.program,
            &right_step.receipt.program,
        )?;

        let descriptor = &left_step.receipt.program;
        ensure_completed(descriptor, "left", left_step.receipt.status)?;
        ensure_completed(descriptor, "right", right_step.receipt.status)?;

        let left_observation =
            observation_or_error(descriptor, "left", left_step.observation.as_ref())?;
        let right_observation =
            observation_or_error(descriptor, "right", right_step.observation.as_ref())?;

        let left_numeric = collect_numeric_leaves(descriptor, "left", left_observation)?;
        let right_numeric = collect_numeric_leaves(descriptor, "right", right_observation)?;

        let left_only: Vec<String> = left_numeric
            .keys()
            .filter(|path| !right_numeric.contains_key(*path))
            .cloned()
            .collect();
        let right_only: Vec<String> = right_numeric
            .keys()
            .filter(|path| !left_numeric.contains_key(*path))
            .cloned()
            .collect();

        if !left_only.is_empty() || !right_only.is_empty() {
            return Err(CompareError::UnmatchedNumericPaths {
                program_id: descriptor.id.clone(),
                program_version: descriptor.version.clone(),
                left_only,
                right_only,
            });
        }

        let mut differences = Vec::with_capacity(left_numeric.len());
        for (path, left_value) in left_numeric {
            let Some(right_value) = right_numeric.get(&path).copied() else {
                // The set equality check above establishes that this cannot
                // occur, but retaining a data error avoids relying on it.
                return Err(CompareError::UnmatchedNumericPaths {
                    program_id: descriptor.id.clone(),
                    program_version: descriptor.version.clone(),
                    left_only: vec![path],
                    right_only: Vec::new(),
                });
            };

            let right_minus_left = right_value - left_value;
            if !right_minus_left.is_finite() {
                return Err(CompareError::NonFiniteDifference {
                    program_id: descriptor.id.clone(),
                    program_version: descriptor.version.clone(),
                    path,
                    left: left_value,
                    right: right_value,
                });
            }

            let relative_change_from_left = if left_value != 0.0 {
                let relative = right_minus_left / left_value.abs();
                relative.is_finite().then_some(relative)
            } else {
                None
            };

            differences.push(NumericDifference {
                path,
                left: left_value,
                right: right_value,
                right_minus_left,
                relative_change_from_left,
            });
        }

        programs.push(ProgramDifference {
            program_id: descriptor.id.clone(),
            program_version: descriptor.version.clone(),
            criterion: descriptor.criterion.clone(),
            differences,
        });
    }

    Ok(EvaluationComparison {
        left_artifact_id: left.artifact.id.clone(),
        right_artifact_id: right.artifact.id.clone(),
        programs,
        limitations: comparison_limitations(),
    })
}

fn ensure_matching_descriptors(
    step_index: usize,
    left: &ProgramDescriptor,
    right: &ProgramDescriptor,
) -> Result<(), CompareError> {
    if left.id != right.id {
        return descriptor_mismatch(step_index, "id", &left.id, &right.id);
    }
    if left.version != right.version {
        return descriptor_mismatch(step_index, "version", &left.version, &right.version);
    }
    if left.criterion != right.criterion {
        return descriptor_mismatch(step_index, "criterion", &left.criterion, &right.criterion);
    }
    if left.epistemic_class != right.epistemic_class {
        return descriptor_mismatch(
            step_index,
            "epistemic_class",
            &format!("{:?}", left.epistemic_class),
            &format!("{:?}", right.epistemic_class),
        );
    }
    if left.deterministic != right.deterministic {
        return descriptor_mismatch(
            step_index,
            "deterministic",
            &left.deterministic.to_string(),
            &right.deterministic.to_string(),
        );
    }

    Ok(())
}

fn descriptor_mismatch<T>(
    step_index: usize,
    field: &str,
    left: &str,
    right: &str,
) -> Result<T, CompareError> {
    Err(CompareError::ProgramDescriptorMismatch {
        step_index,
        field: field.to_owned(),
        left: left.to_owned(),
        right: right.to_owned(),
    })
}

fn ensure_completed(
    descriptor: &ProgramDescriptor,
    side: &str,
    status: ProgramStatus,
) -> Result<(), CompareError> {
    if status == ProgramStatus::Completed {
        return Ok(());
    }

    Err(CompareError::IncompleteProgram {
        program_id: descriptor.id.clone(),
        program_version: descriptor.version.clone(),
        side: side.to_owned(),
        status,
    })
}

fn observation_or_error<'a>(
    descriptor: &ProgramDescriptor,
    side: &str,
    observation: Option<&'a Value>,
) -> Result<&'a Value, CompareError> {
    observation.ok_or_else(|| CompareError::MissingObservation {
        program_id: descriptor.id.clone(),
        program_version: descriptor.version.clone(),
        side: side.to_owned(),
    })
}

fn collect_numeric_leaves(
    descriptor: &ProgramDescriptor,
    side: &str,
    observation: &Value,
) -> Result<BTreeMap<String, f64>, CompareError> {
    let mut leaves = BTreeMap::new();
    collect_numeric_leaves_at(descriptor, side, observation, "", &mut leaves)?;
    Ok(leaves)
}

fn collect_numeric_leaves_at(
    descriptor: &ProgramDescriptor,
    side: &str,
    value: &Value,
    path: &str,
    leaves: &mut BTreeMap<String, f64>,
) -> Result<(), CompareError> {
    match value {
        Value::Number(number) => {
            let Some(numeric_value) = exact_finite_f64(number) else {
                return Err(CompareError::InvalidNumericValue {
                    program_id: descriptor.id.clone(),
                    program_version: descriptor.version.clone(),
                    side: side.to_owned(),
                    path: path.to_owned(),
                    value: number.to_string(),
                });
            };

            if leaves.insert(path.to_owned(), numeric_value).is_some() {
                return Err(CompareError::DuplicateNumericPath {
                    program_id: descriptor.id.clone(),
                    program_version: descriptor.version.clone(),
                    side: side.to_owned(),
                    path: path.to_owned(),
                });
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                let child_path = pointer_path(path, &index.to_string());
                collect_numeric_leaves_at(descriptor, side, child, &child_path, leaves)?;
            }
        }
        Value::Object(values) => {
            for (key, child) in values {
                let child_path = pointer_path(path, &escape_pointer_token(key));
                collect_numeric_leaves_at(descriptor, side, child, &child_path, leaves)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }

    Ok(())
}

fn exact_finite_f64(number: &serde_json::Number) -> Option<f64> {
    if let Some(value) = number.as_i64() {
        let converted = value as f64;
        return (converted.is_finite() && converted as i128 == i128::from(value))
            .then_some(converted);
    }
    if let Some(value) = number.as_u64() {
        let converted = value as f64;
        return (converted.is_finite() && converted as i128 == i128::from(value))
            .then_some(converted);
    }
    number.as_f64().filter(|value| value.is_finite())
}

fn pointer_path(parent: &str, token: &str) -> String {
    let mut path = String::with_capacity(parent.len() + token.len() + 1);
    path.push_str(parent);
    path.push('/');
    path.push_str(token);
    path
}

fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn comparison_limitations() -> Vec<String> {
    vec![
        "Numeric deltas have no intrinsic good/bad direction.".to_owned(),
        "Values inherit each program's proxy/oracle limits.".to_owned(),
        "Matched program versions do not prove matched external tool/environment versions."
            .to_owned(),
        "Only numeric leaves are compared; they must exist at identical JSON Pointer paths."
            .to_owned(),
        "Uncertainty is not synthesized.".to_owned(),
    ]
}

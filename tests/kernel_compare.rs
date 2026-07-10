use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use software_evaluation::compare::{CompareError, compare_evaluation_runs};
use software_evaluation::kernel::{
    Applicability, ArtifactSnapshot, BeliefState, BeliefUpdate, Continuation, CriterionProgram,
    DecisionSpec, EpistemicClass, EvaluationRun, ProgramContext, ProgramDescriptor, ProgramFailure,
    ProgramOutput, ProgramStatus, ResourceBudget, ResourceVector, StopReason, evaluate_pipeline,
};

struct TinyProgram {
    version: String,
    observation: Value,
    belief_updates: Vec<BeliefUpdate>,
    estimate: ResourceVector,
}

impl TinyProgram {
    fn observing(version: &str, observation: Value) -> Self {
        Self {
            version: version.to_owned(),
            observation,
            belief_updates: Vec::new(),
            estimate: ResourceVector::zero(),
        }
    }

    fn with_updates(mut self, belief_updates: Vec<BeliefUpdate>) -> Self {
        self.belief_updates = belief_updates;
        self
    }
}

impl CriterionProgram for TinyProgram {
    fn descriptor(&self) -> ProgramDescriptor {
        ProgramDescriptor {
            id: "tiny-observer".to_owned(),
            version: self.version.clone(),
            criterion: "test criterion".to_owned(),
            epistemic_class: EpistemicClass::Exact,
            deterministic: true,
            description: "Deterministic in-test observation program".to_owned(),
        }
    }

    fn applicability(&self, _artifact: &ArtifactSnapshot) -> Applicability {
        Applicability::Applicable
    }

    fn estimate(&self, _artifact: &ArtifactSnapshot) -> Result<ResourceVector, ProgramFailure> {
        Ok(self.estimate.clone())
    }

    fn run(&self, _context: &ProgramContext<'_>) -> Result<ProgramOutput, ProgramFailure> {
        Ok(ProgramOutput {
            observation: self.observation.clone(),
            evidence: Vec::new(),
            belief_updates: self.belief_updates.clone(),
            resources: ResourceVector::zero(),
            continuation_hints: Vec::new(),
            limitations: Vec::new(),
        })
    }
}

struct MustNotRunProgram;

impl CriterionProgram for MustNotRunProgram {
    fn descriptor(&self) -> ProgramDescriptor {
        ProgramDescriptor {
            id: "must-not-run".to_owned(),
            version: "1".to_owned(),
            criterion: "execution boundary".to_owned(),
            epistemic_class: EpistemicClass::Exact,
            deterministic: true,
            description: "Panics if a blocked program is executed".to_owned(),
        }
    }

    fn applicability(&self, _artifact: &ArtifactSnapshot) -> Applicability {
        Applicability::Applicable
    }

    fn estimate(&self, _artifact: &ArtifactSnapshot) -> Result<ResourceVector, ProgramFailure> {
        Ok(ResourceVector::zero())
    }

    fn run(&self, _context: &ProgramContext<'_>) -> Result<ProgramOutput, ProgramFailure> {
        panic!("a program blocked by the invocation limit must not execute")
    }
}

fn artifact(revision: &str, tree_digest: &str) -> ArtifactSnapshot {
    ArtifactSnapshot {
        id: format!("fixture-{revision}"),
        root: PathBuf::from("fixture"),
        revision: revision.to_owned(),
        tree_digest: tree_digest.to_owned(),
        kind: "repository".to_owned(),
    }
}

fn decision() -> DecisionSpec {
    DecisionSpec {
        id: "ship-decision".to_owned(),
        description: "Decide whether to ship".to_owned(),
        claim_ids: vec!["safe".to_owned()],
    }
}

fn initial_beliefs() -> BeliefState {
    BeliefState {
        probabilities: BTreeMap::from([("safe".to_owned(), 0.4)]),
        observation_digests: Vec::new(),
    }
}

fn budget(max_programs: u32) -> ResourceBudget {
    ResourceBudget {
        max_usd: 0.0,
        max_wall_time_ms: 10_000,
        max_programs,
    }
}

fn run_program(revision: &str, tree_digest: &str, program: &dyn CriterionProgram) -> EvaluationRun {
    evaluate_pipeline(
        &artifact(revision, tree_digest),
        &[program],
        &initial_beliefs(),
        &decision(),
        &budget(1),
    )
    .expect("valid fixture program should evaluate")
}

#[test]
fn zero_dollar_programs_continue_to_the_next_step_and_distinct_digests_accumulate() {
    let first = TinyProgram::observing("1", json!({"value": 1}));
    let second = TinyProgram::observing("1", json!({"value": 2}));

    let run = evaluate_pipeline(
        &artifact("rev-a", "tree-a"),
        &[&first, &second],
        &initial_beliefs(),
        &decision(),
        &budget(2),
    )
    .expect("zero-cost programs should fit a zero-dollar budget");

    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.steps[0].receipt.status, ProgramStatus::Completed);
    assert!(matches!(
        run.steps[0].continuation,
        Continuation::Continue { .. }
    ));
    assert_eq!(run.steps[1].receipt.status, ProgramStatus::Completed);
    assert_eq!(run.stopped_reason, StopReason::Complete);
    assert_eq!(run.remaining.usd, 0.0);

    let first_digest = run.steps[0]
        .receipt
        .observation_digest
        .as_ref()
        .expect("completed observation has a digest");
    let second_digest = run.steps[1]
        .receipt
        .observation_digest
        .as_ref()
        .expect("completed observation has a digest");
    assert_ne!(first_digest, second_digest);
    assert_eq!(
        run.posterior.observation_digests,
        [first_digest.clone(), second_digest.clone()]
    );
}

#[test]
fn program_invocation_estimate_blocks_execution_when_no_invocations_remain() {
    let program = MustNotRunProgram;
    let prior = initial_beliefs();

    let run = evaluate_pipeline(
        &artifact("rev-a", "tree-a"),
        &[&program],
        &prior,
        &decision(),
        &budget(0),
    )
    .expect("a budget block is a recorded evaluation outcome");

    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.steps[0].receipt.status, ProgramStatus::BudgetBlocked);
    assert_eq!(run.stopped_reason, StopReason::ProgramLimit);
    assert!(run.steps[0].observation.is_none());
    assert_eq!(run.posterior, prior);
}

#[test]
fn malformed_belief_updates_fail_closed_without_changing_the_prior() {
    let cases = [
        (
            "undeclared claim",
            vec![BeliefUpdate {
                claim_id: "unknown".to_owned(),
                posterior_probability: 0.8,
                basis: "observation".to_owned(),
            }],
            "undeclared claim",
        ),
        (
            "duplicate claim",
            vec![
                BeliefUpdate {
                    claim_id: "safe".to_owned(),
                    posterior_probability: 0.7,
                    basis: "first".to_owned(),
                },
                BeliefUpdate {
                    claim_id: "safe".to_owned(),
                    posterior_probability: 0.8,
                    basis: "second".to_owned(),
                },
            ],
            "repeats claim",
        ),
        (
            "nonfinite probability",
            vec![BeliefUpdate {
                claim_id: "safe".to_owned(),
                posterior_probability: f64::NAN,
                basis: "observation".to_owned(),
            }],
            "finite and in [0, 1]",
        ),
        (
            "empty basis",
            vec![BeliefUpdate {
                claim_id: "safe".to_owned(),
                posterior_probability: 0.8,
                basis: " ".to_owned(),
            }],
            "belief update basis",
        ),
    ];

    for (name, updates, expected_message) in cases {
        let program = TinyProgram::observing("1", json!({"value": 1})).with_updates(updates);
        let prior = initial_beliefs();
        let run = run_program("rev-a", "tree-a", &program);
        let step = &run.steps[0];

        assert_eq!(step.receipt.status, ProgramStatus::Failed, "{name}");
        assert_eq!(run.stopped_reason, StopReason::ProgramFailed, "{name}");
        assert!(step.observation.is_none(), "{name}");
        assert!(step.receipt.observation_digest.is_none(), "{name}");
        assert_eq!(run.posterior, prior, "{name}");
        assert!(
            step.receipt
                .message
                .as_deref()
                .is_some_and(|message| message.contains(expected_message)),
            "{name}: {:?}",
            step.receipt.message
        );
    }
}

#[test]
fn comparison_orders_numeric_deltas_lexicographically_and_escapes_pointer_tokens() {
    let left_program = TinyProgram::observing(
        "1",
        json!({"z": 10, "a/b": {"~key": 2}, "array": [0, 4], "ignored": "left"}),
    );
    let right_program = TinyProgram::observing(
        "1",
        json!({"z": 7, "a/b": {"~key": 5}, "array": [1, 2], "ignored": false}),
    );
    let left = run_program("rev-left", "tree-left", &left_program);
    let right = run_program("rev-right", "tree-right", &right_program);

    let comparison = compare_evaluation_runs(&left, &right)
        .expect("matching completed programs should be comparable");
    let deltas: Vec<_> = comparison.programs[0]
        .differences
        .iter()
        .map(|difference| {
            (
                difference.path.as_str(),
                difference.left,
                difference.right,
                difference.right_minus_left,
            )
        })
        .collect();

    assert_eq!(
        deltas,
        [
            ("/array/0", 0.0, 1.0, 1.0),
            ("/array/1", 4.0, 2.0, -2.0),
            ("/a~1b/~0key", 2.0, 5.0, 3.0),
            ("/z", 10.0, 7.0, -3.0),
        ]
    );
}

#[test]
fn optional_numeric_leaf_present_on_only_one_side_fails_closed() {
    let left_program = TinyProgram::observing("1", json!({"metric": 1, "optional": null}));
    let right_program = TinyProgram::observing("1", json!({"metric": 2, "optional": 3}));
    let left = run_program("rev-left", "tree-left", &left_program);
    let right = run_program("rev-right", "tree-right", &right_program);

    let error = compare_evaluation_runs(&left, &right)
        .expect_err("numeric leaves must be matched by JSON Pointer");

    assert_eq!(
        error,
        CompareError::UnmatchedNumericPaths {
            program_id: "tiny-observer".to_owned(),
            program_version: "1".to_owned(),
            left_only: Vec::new(),
            right_only: vec!["/optional".to_owned()],
        }
    );
}

#[test]
fn incompatible_program_versions_are_rejected() {
    let left_program = TinyProgram::observing("1", json!({"metric": 1}));
    let right_program = TinyProgram::observing("2", json!({"metric": 2}));
    let left = run_program("rev-left", "tree-left", &left_program);
    let right = run_program("rev-right", "tree-right", &right_program);

    let error = compare_evaluation_runs(&left, &right)
        .expect_err("different program versions are not matched measurements");

    assert_eq!(
        error,
        CompareError::ProgramDescriptorMismatch {
            step_index: 0,
            field: "version".to_owned(),
            left: "1".to_owned(),
            right: "2".to_owned(),
        }
    );
}

#[test]
fn same_tree_at_different_revisions_remains_comparable() {
    let left_program = TinyProgram::observing("1", json!({"metric": 1}));
    let right_program = TinyProgram::observing("1", json!({"metric": 2}));
    let left = run_program("rev-left", "same-tree", &left_program);
    let right = run_program("rev-right", "same-tree", &right_program);

    let comparison = compare_evaluation_runs(&left, &right)
        .expect("revision identity, not tree identity alone, distinguishes artifacts");

    assert_eq!(comparison.programs[0].differences[0].path, "/metric");
    assert_eq!(comparison.programs[0].differences[0].right_minus_left, 1.0);
}

#[test]
fn identical_revision_and_tree_are_rejected() {
    let left_program = TinyProgram::observing("1", json!({"metric": 1}));
    let right_program = TinyProgram::observing("1", json!({"metric": 2}));
    let left = run_program("same-revision", "same-tree", &left_program);
    let right = run_program("same-revision", "same-tree", &right_program);

    let error = compare_evaluation_runs(&left, &right)
        .expect_err("the same artifact identity is not a matched comparison");

    assert_eq!(
        error,
        CompareError::IdenticalArtifact {
            revision: "same-revision".to_owned(),
            tree_digest: "same-tree".to_owned(),
        }
    );
}

#[test]
fn equal_completed_prefixes_stopped_by_program_limit_are_not_comparable() {
    let left_first = TinyProgram::observing("1", json!({"metric": 1}));
    let left_second = TinyProgram::observing("1", json!({"metric": 10}));
    let right_first = TinyProgram::observing("1", json!({"metric": 2}));
    let right_second = TinyProgram::observing("1", json!({"metric": 20}));
    let prior = initial_beliefs();
    let decision = decision();
    let one_program_budget = budget(1);

    let left = evaluate_pipeline(
        &artifact("rev-left", "tree-left"),
        &[&left_first, &left_second],
        &prior,
        &decision,
        &one_program_budget,
    )
    .expect("the left run should record its completed prefix");
    let right = evaluate_pipeline(
        &artifact("rev-right", "tree-right"),
        &[&right_first, &right_second],
        &prior,
        &decision,
        &one_program_budget,
    )
    .expect("the right run should record its completed prefix");

    assert_eq!(left.steps.len(), 1);
    assert_eq!(right.steps.len(), 1);
    assert_eq!(left.steps[0].receipt.status, ProgramStatus::Completed);
    assert_eq!(right.steps[0].receipt.status, ProgramStatus::Completed);
    assert_eq!(left.stopped_reason, StopReason::ProgramLimit);
    assert_eq!(right.stopped_reason, StopReason::ProgramLimit);

    let error = compare_evaluation_runs(&left, &right)
        .expect_err("equal completed prefixes do not represent complete matched runs");

    assert_eq!(
        error,
        CompareError::IncompleteRun {
            side: "left".to_owned(),
            reason: StopReason::ProgramLimit,
        }
    );
}

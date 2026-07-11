# Canonicality denominator

This file defines what “canonical” means for `software-evaluation`. It is the
review denominator, not a self-awarded grade. Every row is one of **covered**,
**named-gap**, or **ruled-out**. A covered row must resolve to executable or
committed evidence; a named gap remains part of the denominator until closed.

Preserved target: the tool must faithfully expose the quality shape and branch
differences of fixed software artifacts without turning partial proxies into an
overall-quality claim.

## Truth

| Property | State | Evidence or gap |
|---|---|---|
| Published CLI behavior is exercised, including invalid and partial inputs | named-gap | Compiled-binary tests cover 9 of 13 subcommands; `audit`, `plan`, `repo-profile`, `repo-compare`, aliases, help, and version still lack CLI-wiring coverage. |
| Repository comparisons bind both sides to explicit commits and reject tracked worktree drift | covered | `repo-profile`/`repo-compare` receipts plus `tests/repo.rs`. |
| Numeric deltas compare only compatible program versions and retain both operands | covered | `src/compare.rs` and `tests/kernel_compare.rs`. |
| Every reported denominator remains independent of presentation limits | named-gap | Clone output must prove whether `--max-groups` truncates totals as well as rows; current large-repository runs hit the configured ceiling. |
| All supported-language parsers expose syntax-error coverage instead of silently passing | covered | Source-instrument JSON coverage plus parser regression tests. |
| Exact, proxy, and judgment evidence cannot be confused in stored records | named-gap | Criterion receipts type epistemic class, but one-off source JSON does not yet emit a uniform record envelope. |
| Archive auditing proves record/evidence/report closure without accepting path escape or vacuous evidence | named-gap | Audit now enforces commit-pin syntax, instrument classes, agent/timestamp shape, and canonical path containment; reverse report closure, fragment targets, evidence digests, and exact-prompt proof remain unenforced. |
| Deterministic outputs are stable under repeated execution at one artifact commit | named-gap | Determinism is designed and unit-tested locally; a committed repeat-run equality receipt is still absent. |

## First contact

| Property | State | Evidence or gap |
|---|---|---|
| A stranger can identify the tool, its audience, and its central refusal in the first screen | covered | `README.md` opening and `seval --help`. |
| One documented command produces a real result from a fresh checkout | named-gap | README commands exist, but a committed cold-clone transcript is still absent. |
| Branch comparison has a literal, reproducible workflow from refs to clean snapshots to receipts | named-gap | `repo-compare` requires clean worktrees; README does not yet show the branch/worktree workflow. |
| Every output explains what it can and cannot establish | covered | Instrument limitations are emitted in JSON and documented in `INSTRUMENTS.md`. |
| Failure and partial-result exit codes are discoverable at the point of use | named-gap | Global README documents exit classes; per-subcommand help does not consistently surface them. |

## Depth

| Property | State | Evidence or gap |
|---|---|---|
| All nine taxonomy axes are labeled evaluated, waived with reason, or uncovered in each final report | named-gap | Mechanical branch receipts cover proxies for parsimony, interface shape, correctness machinery, and evolvability; judged and empirical axes remain to be run. |
| Mechanical evidence includes denominators, distributions/tails where relevant, and raw evidence identities | named-gap | Repository programs emit Git argv/digests; one-off source instruments emit denominators and tails but not a uniform commit/tree-linked receipt. |
| Correctness claims use behavior tests or empirical probes rather than static shape | named-gap | The current exopriors comparison inventories test machinery but has not executed either branch’s behavioral suite. |
| Normative judgments use at least two independent, adversarially symmetric judges and report spread | named-gap | No judged pass has yet been archived for the exopriors comparison. |
| An overall winner appears only under a preregistered gate, Pareto rule, or owner policy | covered | `REPORT.md` forbids axis counting and composite scores. |

## Craft

| Property | State | Evidence or gap |
|---|---|---|
| Public concepts have one canonical name and one representation | named-gap | The audit found drift between canonical taxonomy axes, custom criteria, instrument classes, and epistemic classes; the namespaces remain to be separated in schema and examples. |
| Algorithms avoid avoidable allocation, duplicate traversal, and unbounded retained output | named-gap | Large-repository runs complete, but clone totals are presentation-capped and benchmark/process-tree and allocation behavior remain unprofiled. |
| Errors identify the failed contract and preserve the underlying cause | named-gap | Error-path review found criterion failures that cannot report actual resource use and benchmark timeouts that do not bound descendants. |
| Documentation claims, command surface, schemas, and tests agree at HEAD | named-gap | A 77/77 claim audit found remaining aspiration-versus-integration, provenance-schema, resident-fixture, packaging, and CLI-wiring gaps. |
| Repository prose has one voice and every sentence changes operator behavior or understanding | named-gap | The first pass removed the main end-to-end overclaim; a full dry prose round has not occurred. |

## Stewardship

| Property | State | Evidence or gap |
| Evaluation reports pin artifact commits and remain immutable historical facts | named-gap | Audit now enforces hexadecimal pin syntax, but not commit existence, report immutability, or evidence-to-commit binding. |
| Accepted and rejected review findings retain reasons and receipts | named-gap | The present drive has no committed findings ledger yet. |
| A full adversarial round returns zero must-fix findings before “canonical” is claimed | named-gap | No dry round has occurred. |
| A final cold-clone smoke test passes at the reviewed commit | named-gap | Not yet run. |
| Remaining limitations are named in the artifact rather than hidden in session prose | named-gap | This denominator names the current residue; the exopriors evaluation report remains to be written. |

## Ruled out

| Property | State | Reason |
|---|---|---|
| A scalar or weighted overall quality score | ruled-out | Quality is a shape; weights belong to an explicit owner policy. |
| Static metrics as proof of correctness, security, maintainability, or fitness-to-intent | ruled-out | These instruments are bounded proxies only. |
| Forced verdicts on non-applicable axes | ruled-out | Applicability waivers are part of the report denominator. |
| Silent averaging of judge disagreement | ruled-out | Spread and contested axes are first-class findings. |

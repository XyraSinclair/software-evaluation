# Findings ledger

Disposition is explicit: **fixed**, **accepted-open**, or **ruled-out-as-bug**. Open findings remain release blockers for calling `software-evaluation` canonical; they do not invalidate every mechanical observation in this bundle.

## Fixed in `software-evaluation@fbcb444`

### F-001 — Archive audit admitted false provenance

**Finding.** `audit` accepted symbolic artifact pins such as `repo@HEAD`, arbitrary instrument strings, missing agent/timestamp provenance, and procedure/evidence symlinks resolving outside the bundle. It also rejected a faithful two-artifact comparison identity.

**Disposition:** fixed. `src/audit.rs` now validates one or two hexadecimal commit-pin identities, the three instrument classes, structured agent identity, nullable/RFC3339-like timestamps, and canonical path containment. Eight new contracts in `tests/audit.rs`; `cargo test --test audit` passed 17/17.

**Remaining boundary:** commit existence, evidence-to-commit binding, fragment targets, evidence digests, reverse record/report closure, and exact-prompt identity remain open under F-006.

### F-002 — Metrics hid syntax-error parsing

**Finding.** `metrics` and `metrics-compare` could analyze tree-sitter recovery trees while reporting no malformed-file denominator.

**Disposition:** fixed. Aggregate coverage now includes `syntax_error_files` from the same parse used for metrics; no second parse or traversal. Regression `aggregate_json_reports_expose_syntax_error_coverage`; `cargo test --test metrics_cli` passed 6/6.

### F-003 — Rust API surface inverted reachability

**Finding.** Rust analysis counted `pub(crate)`/restricted declarations and `pub` items below private modules, while omitting methods declared by public traits. This materially changed the ExoPriors API comparison.

**Disposition:** fixed. Rust traversal now follows externally reachable module visibility and inventories required/default methods of reachable public traits. `cargo test --test api_surface` passed 3/3. Final branch receipts were regenerated after the fix.

### F-004 — Test inventory misclassified ignored and owned tests

**Finding.** JavaScript/TypeScript cases under skipped suites remained non-ignored. Same-stem ownership was global-basename-only, so ambiguous modules could be matched across unrelated directories.

**Disposition:** fixed. Ignored-suite ancestry propagates recursively. Same-stem matching prefers a unique direct-directory candidate, then a globally unique candidate; ambiguous global stems remain unmatched. `cargo test --test tests_analysis` passed 4/4. Final branch receipts were regenerated after the fix.

## Accepted and open

### F-005 — Comparison accepts incoherent/tampered public runs

`compare_evaluation_runs` matches program descriptors and numeric paths but does not recompute observation digests, bind step receipts to the enclosing artifact, validate posterior/continuation chains, or require matching decision/prior configuration. `EvaluationComparison` also drops revision and tree digests.

**Risk:** a deserialized/tampered run can produce authoritative-looking deltas. The direct `repo-compare` CLI path used here generated both runs and the archive retains the full wrapper identities, so this is not evidence that the present file was tampered with. It remains a canonical-trust blocker.

**Required closure:** validated/opaque run type or complete pre-comparison validation; carry full artifact and configuration identities into the result; adversarial tamper regressions.

### F-006 — Archive closure remains structural, not evidentiary

The strengthened audit validates schema shape and path containment but does not prove commit existence, evidence content relevance, content digests, Markdown fragment existence, exact judged-prompt identity, every verdict sentence’s citation, or reverse record-to-report use.

**Risk:** audit PASS means the bundle is structurally closed under its current rules, not that every claim is true or independently reproducible.

### F-007 — Failure receipts cannot report actual resource use

`ProgramFailure` carries only kind/message; failed criterion execution is receipted with zero USD and zero I/O except wall time. Normal pipeline completion also discards final-program continuation hints.

**Risk:** budgets and provenance are least trustworthy on paid/error paths; follow-up recommendations can disappear.

**Required closure:** success/failure execution outcomes both carry measured or explicitly unknown resources; terminal steps retain hints; regression programs consume resources then fail.

### F-008 — Planner units and numerics overclaim exactness

Importance-weighted utility is exposed in fields named `*_information_bits`; probability-space multiplication and entropy subtraction can underflow/cancel rare or weak signals; binary floating budget addition can reject decimal-feasible plans.

**Risk:** a binary claim can appear to yield more than one literal bit, rare-event posteriors can collapse, and probe selection can change at decimal budget boundaries.

**Required closure:** separate raw bits from unitless weighted utility, stable log-space/KL calculations with declared tolerances, and fixed-point/decimal money.

### F-009 — Git repository receipts are not hermetic

Repository programs retain Git argv/version/stdout digest, commit, and tree, but Git inherits configuration/environment and does not detect shallow history, replace/graft state, global attributes, or alternate object behavior. Raw Git stdout is not archived by the native receipt.

**Risk:** the same named revision can yield different bounded-history observations under different repository/process state.

**Required closure:** sanitize and record relevant Git environment/config, detect shallow/replace/graft conditions, and retain raw stdout or a complete replay capsule.

### F-010 — Benchmark deadlines and identity do not cover process trees

Timeout kills only the immediate child; descendants can survive while holding capture pipes open. Command identity excludes resolved executable bytes/path and most environment, and execution re-resolves the user program spelling.

**Risk:** a timed-out benchmark can outlive its deadline and the receipt may not identify what executable actually ran.

**Required closure:** process-group/job-object termination, elapsed measurement through final drain, launch the resolved executable, and hash executable/environment/spec identity.

### F-011 — Dependency extraction has false unresolved and source-kind cases

Rust grouped imports, nested out-of-line modules, and some `super` paths are mishandled. Valid npm/Python path, shorthand, tarball, and direct-URL dependencies can be classified as registry; mixed Cargo source metadata loses provenance.

**Risk:** cycle/topology and risky-dependency counts can be materially low. The current dependency outputs remain conservative structural proxies, not complete graphs.

### F-012 — Clone output has semantic and denominator gaps

`lines_per_occurrence` is the maximum occurrence span, not a per-occurrence invariant; grouping is language-local without a first-class scope field. Both ExoPriors scans saturated `max_groups=1000`.

**Risk:** the retained clone totals cannot support a complete cross-branch duplication verdict.

### F-013 — End-to-end product and stranger path remain incomplete

The CLI exposes 13 bounded commands but no full nine-axis evaluation, judge orchestration, bundle construction, or report generation command. Compiled-binary tests cover 9/13 commands; `audit`, `plan`, `repo-profile`, `repo-compare`, aliases, help, and version lack CLI-wiring coverage. There is no committed positive cold-clone transcript, and the Forward resident bundle is deliberately invalid.

**Required closure:** literal branch workflow, positive fixture, uniform commit-bound one-off envelopes, full CLI denominator, cold-clone receipt, behavioral probes, independent judged axes, and a policy-gated synthesis path.

## Ruled out as implementation bugs

### R-001 — Capped clone totals

`DuplicateTotals` describes emitted groups after deterministic `max_groups` truncation; this behavior is explicit in output limitations and tested. The cap is not itself a bug. Treating saturated totals as a full-corpus comparison would be an interpretation bug, so this report issues no clone direction.

### R-002 — Numeric-only repository comparison

`repo-compare` intentionally compares matched numeric leaves and states that it ignores nonnumeric values. This is a documented scope choice, not silently repaired here. The full side observations are retained so changed path identities remain inspectable. Open improvement: reject or flag vacuous comparisons and expose ignored nonnumeric differences explicitly.

### R-003 — No mechanical overall winner

The absence of a scalar/winner is deliberate. Quality direction for size, API density, dependency count, test count, and change topology is normative and must remain outside the mechanical instruments unless an owner policy is preregistered.

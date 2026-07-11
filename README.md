# software-evaluation

Deep, provenanced software evaluation: ordinal comparisons of software
artifacts along **nine timeless, orthogonal axes**, measured by the
strongest available instrument, with every verdict resolving to a committed
receipt. The output is a report that says which artifact is stronger, how
the two differ *in kind*, and exactly where the genuine open judgments live.

Built on three refusals:

- **No composite score.** Quality is a shape, not a scalar. Axis weights are
  the owner's values; a weighted average would launder them.
- **No unprovenance'd verdicts.** Every number and every judgment resolves,
  in two hops, to a replayable command, a transcript, or a judge receipt —
  see [PROVENANCE.md](PROVENANCE.md).
- **No manufactured oracles.** Axes with no computable truth (fitness-to-
  intent, conceptual parsimony) are triangulated across ≥2 independent
  judges; agreement compresses, disagreement is reported as the finding and
  routed to the owner.

## The pieces

| Doc | What it holds |
|---|---|
| [TAXONOMY.md](TAXONOMY.md) | The nine axes — Form (parsimony, consistency, interface sharpness), Assurance (correctness machinery, robustness, documentation truth), Life (evolvability, operational legibility, fitness-to-intent) — with applicability gates: not all software must honor all nine, but every waiver is declared. |
| [INSTRUMENTS.md](INSTRUMENTS.md) | Mechanical, empirical, and judged instruments, selected by target binding and oracle coverage before reproducibility or cost. AST-level complexity/clone/dependency metrics, git co-change mining, mutation testing; cold-start and claim audits; triangulated judgment via [cardinal-harness](https://github.com/XyraSinclair/cardinal-harness). |
| [PROVENANCE.md](PROVENANCE.md) | The record schema: artifact@commit, instrument, agent, procedure, evidence, verdict, integrity caveats. Honest nulls over neat fictions. |
| [REPORT.md](REPORT.md) | The report template and its rules: spread never mean, adversarial symmetry, denominators always, verdicts carry record ids. |
| `evaluations/` | Actual runs: one directory per evaluation — report, records.jsonl, raw judge outputs and metric dumps. |

## Executable core

`seval` now closes five loops:

1. **Archive integrity:** prove that an evaluation has nonempty records, safe
   and existing evidence/procedure paths, literal report-to-record references,
   commit-pinned artifact identities, and non-vacuous inputs.
2. **Audit planning:** choose probes under hard dollar, time, and count budgets
   using Shannon information gain, expected Bayes-risk reduction, decision-flip
   probability, redundancy-aware conditional information, and Pareto frontiers.
3. **Criterion execution:** run independently shaped programs through
   `evaluate(artifact, program, evidence, decision, resources)` and retain the
   typed observation, evidence, receipt, posterior state, and continuation.
4. **Repository profiling and comparison:** run two fast, deterministic proxy
   programs against committed Git snapshots, then derive matched numeric deltas
   without assigning a winner or an overall quality score.
5. **One-off source and execution measurement:** tabulate AST complexity,
   dependency topology, normalized clones, public API surface, test machinery,
   and direct-argv benchmark receipts; compare source trees as explicit
   `right - left` differences without manufacturing a global score.

```console
$ cargo run -- audit evaluations/forward-cycle1-20260709
$ cargo run -- plan examples/audit-plan.json
$ cargo run -- repo-profile /path/to/clean/repo --format json
$ cargo run -- repo-compare /path/to/left /path/to/right --format json
$ cargo run --release -- metrics /path/to/repo
$ cargo run --release -- functions /path/to/repo --sort cognitive --top 30
$ cargo run --release -- files /path/to/repo --sort maintainability --top 30
$ cargo run --release -- metrics-compare /path/to/left /path/to/right --format json
$ cargo run --release -- deps /path/to/repo
$ cargo run --release -- duplicates /path/to/repo --min-tokens 40 --min-lines 5
$ cargo run --release -- api /path/to/repo --top 100
$ cargo run --release -- tests /path/to/repo --top 100
$ cargo run --release -- bench --warmup 1 --runs 20 -- ./program --exact-arg
```

`repo-profile` composes two versioned programs:

- `repo.static-shape@1`: tracked blob bytes and paths, lexical source/test/docs/
  configuration/generated classification, source-size concentration, effective
  file/component counts, test/docs-to-source ratios, and path depth;
- `repo.git-change-shape@1`: bounded non-merge history, files-per-commit tails,
  change-mass concentration and hotspots, cross-top-level cochange, broad-commit
  rate, and source/test/docs cochange rates.

Both programs run at committed `HEAD` and reject tracked uncommitted changes;
untracked files are ignored because they are outside the measured snapshot.
Every observation carries its classifier, Git command, Git version, raw-output
SHA-256 digest, resource vector, limitations, and a kernel-measured receipt.
`repo-compare` applies the same program versions and budgets independently to
both repositories, rejects incomplete or structurally incompatible results,
and reports `right - left` at matched JSON Pointer paths. Delta direction is
not quality direction.

The worktree instruments analyze the current file or directory, not a commit
snapshot. Their shared walker respects ignore files, does not follow symlinks,
and recognizes Rust, Python, JavaScript, TypeScript/TSX, and Go. `metrics`
reports totals, normalized rates, and nearest-rank function tails; `functions`
and `files` rank hotspots; `metrics-compare` preserves both sides and reports
matched `right - left` differences. `deps` extracts import evidence, direct
manifest dependencies, fan-in/out, SCCs, cycles, weak components, and
condensation depth. `duplicates` finds maximal non-overlapping clone groups
after AST-token normalization. `api` inventories language-native explicit or
documented proxy publicness. `tests` measures cases, ignored cases,
assertion-like calls, source/test lines, and conservative same-stem coverage.
`bench` executes exact argv without a shell, retains successful, failed, and
timed-out samples, separates the first measured run from warmed distributions,
and optionally reports units/s and bytes/s. These are bounded observations:
none establishes correctness, security, semantic equivalence, test adequacy,
fitness-to-intent, or quality.

The archive command is expected to fail on the resident evaluation: that bundle
is deliberately the auditor's first negative fixture. A failed audit,
incomplete repository profile, or benchmark with any failed/timed-out attempt
exits 1; a CLI/input failure exits 2.

The planner's sensitivity and specificity values are **calibrated inputs, not
facts inferred by the planner**. Joint information is computed only when the
spec explicitly accepts conditional independence, and at most one probe per
claim/dependence group is selected. Information, decision value, dollars, and
seconds remain separate; the selected multi-budget sequence is labeled as a
greedy heuristic.

## Uses

- **A vs B**: which of two libraries/services/repos is stronger, and where
  each wins.
- **Before vs after**: did a refactor, a rewrite, or an autonomous-agent
  campaign actually improve the artifact? (First resident evaluation:
  a before/after of a self-improvement cycle —
  `evaluations/forward-cycle1-20260709/`.)
- **Periodic**: the same artifact on the same axes over time; the trend is
  the health curve.

## What "stronger" means here

Stronger per axis, on the axis intersection, after both artifacts survived a
demolition pass. An overall ordinal verdict requires a preregistered
feasibility gate, Pareto dominance under declared practical-difference
thresholds, or an explicit owner decision policy. Otherwise the honest answer
— "stronger at different things" — is stated with the split. The most
valuable section is often **Contested axes**: where independent judges
disagree and the owner's values legitimately decide.

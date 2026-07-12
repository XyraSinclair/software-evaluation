# software-evaluation

Evidence-first primitives for comparing software artifacts along nine declared
axes. `seval` computes bounded mechanical observations, plans audits, executes
criterion programs through library APIs, and checks hand-authored evaluation
bundles. It does not yet orchestrate a complete nine-axis evaluation or choose
a winner; empirical probes, judged axes, and report synthesis remain explicit
operator work.

Built on three refusals:

- **No composite score.** Quality is a shape, not a scalar. Axis weights are
  the owner's values; a weighted average would launder them.
- **No unsupported verdicts.** In a conforming evaluation, every number and
  judgment resolves in two hops to a replayable command, transcript, or judge
  receipt — see [PROVENANCE.md](PROVENANCE.md).
- **No manufactured oracles.** Axes with no computable truth (fitness-to-
  intent, conceptual parsimony) are triangulated across ≥2 independent
  judges; agreement compresses, disagreement is reported as the finding and
  routed to the owner.

## The pieces

| Doc | What it holds |
|---|---|
| [TAXONOMY.md](TAXONOMY.md) | The nine axes — Form (parsimony, consistency, interface sharpness), Assurance (correctness machinery, robustness, documentation truth), Life (evolvability, operational legibility, fitness-to-intent) — with applicability gates: not all software must honor all nine, but every waiver is declared. |
| [INSTRUMENTS.md](INSTRUMENTS.md) | Implemented mechanical instruments plus external empirical and judged methods. AST-level complexity/clone/dependency metrics and git co-change mining are built in; mutation testing, cold-start/claim audits, and [cardinal-harness](https://github.com/XyraSinclair/cardinal-harness) judgment are operator-run methods. |
| [PROVENANCE.md](PROVENANCE.md) | The record schema: artifact@commit, instrument, agent, procedure, evidence, verdict, integrity caveats. Honest nulls over neat fictions. |
| [REPORT.md](REPORT.md) | The report template and its rules: spread never mean, adversarial symmetry, denominators always, verdicts carry record ids. |
| `evaluations/` | Archived runs and fixtures. Each complete run contains a report, records.jsonl, procedures, and raw evidence; deliberately invalid fixtures are labeled. |

## Executable core

The crate implements five bounded pieces; the CLI exposes the commands listed
below, while generic criterion execution is currently a library API:

1. **Archive integrity:** reject empty or malformed records, unsafe or escaping
   evidence/procedure paths, dangling literal report references, invalid
   instrument classes, and malformed commit-pin identities.
2. **Audit planning:** choose probes under hard dollar, time, and count budgets
   using information gain, expected Bayes-risk reduction, decision-flip
   probability, redundancy-aware conditional information, and Pareto frontiers.
3. **Criterion execution:** run independently shaped programs through
   `evaluate(artifact, program, evidence, decision, resources)` and retain the
   typed observation, evidence, receipt, posterior state, and continuation.
4. **Repository profiling and comparison:** run two deterministic proxy
   programs against committed Git snapshots, then derive matched numeric deltas
   without assigning a winner or an overall quality score.
5. **One-off source and execution measurement:** tabulate AST complexity,
   dependency topology, normalized clones, externally reachable Rust API plus
   documented lexical proxies for other languages, test machinery, and
   direct-argv benchmark receipts; source-tree comparisons remain explicit
   `right - left` differences without a global score.

```console
$ cargo run -- audit evaluations/forward-cycle1-20260709
$ cargo run -- plan examples/audit-plan.json
$ cargo run -- repo-profile /path/to/clean/repo --format json
$ cargo run -- repo-compare /path/to/left /path/to/right --format json
$ cargo run --release -- change-profile /path/to/clean/repo --history-commits 200 --format svg
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

`change-profile` is a third commit-pinned repository instrument. It enumerates
all regular blobs at `HEAD`, selects supported source extensions whose raw Git
paths are UTF-8, reads the selected objects from Git rather than from the
worktree, and joins their current AST structure to up to N non-merge history
commits by exact raw path bytes. Worktree ignore rules do not alter this
committed-tree denominator. JSON preserves every row; text provides a bounded
operational table; SVG renders paired absolute and size-normalized language
facets with raw-value log ticks, explicit missingness, coordinate extrema, and
a bounded history-only ledger. The output never multiplies change, size, and
complexity into a score. Tree, blob-batch, and history receipts retain exact
Git commands, versions, byte counts, and SHA-256 digests.

The worktree instruments analyze the current file or directory, not a commit
snapshot. Their shared walker respects ignore files, does not follow symlinks,
and recognizes Rust, Python, JavaScript, TypeScript/TSX, and Go. `metrics`
reports totals, normalized rates, and nearest-rank function tails; `functions`
and `files` rank hotspots; `metrics-compare` preserves both sides and reports
matched `right - left` differences. `deps` extracts import evidence, direct
manifest dependencies, all-edge and resolved-internal fan-in/out, SCCs, cycles,
weak components, condensation depth, and bounded exact non-self transitive
reachability over the observed internal file graph. `duplicates` finds maximal
non-overlapping clone groups after AST-token normalization. `api` inventories
declarations and documented lexical publicness proxies for the other supported
languages. `tests` measures cases, inherited ignored-suite state,
assertion-like calls, source/test lines, and conservative path-aware same-stem
coverage.
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

## Public GitHub snapshot service

`sevald` exposes the five source instruments through a bounded asynchronous API
for public GitHub repositories. It resolves the default branch to an immutable
commit, downloads a fixed-host ZIP archive, extracts it under strict path and
size limits, and analyzes it in a kill-and-reap worker process. Repository code,
tests, hooks, builds, and package managers are never executed.

```console
$ cargo run --release --bin sevald -- serve \
    --listen 127.0.0.1:7077 \
    --cache-dir .seval-cache
$ curl -sS -X POST http://127.0.0.1:7077/v1/analyses \
    -H 'content-type: application/json' \
    -d '{"owner":"octocat","repo":"Hello-World"}'
```

Poll the returned `/v1/analyses/{analysis_id}` URL until `state` is
`completed`, `completed_partial`, or `failed`. Completed responses retain the
immutable commit, cache provenance, each analyzer's coverage denominator,
observations, and limitations. They contain no aggregate score or verdict.
Dependency observations also retain the propagation numerator and denominator,
cycle participation, the computation status and resource bounds, and bounded
direct/transitive in/out hotspot rows. These remain topology coordinates, not
maintenance scores or refactoring recommendations.
`GITHUB_TOKEN` is optional and stays server-side; authenticated acquisition
still rejects private repositories.

For a contained local deployment, `docker compose up --build -d` builds only
`sevald`, binds it to host loopback on port 7077, drops Linux capabilities,
uses a read-only root filesystem, and applies CPU, memory, process, and temp
storage limits. Put an HTTPS reverse proxy with request-rate limits in front of
that loopback listener before exposing it publicly. The service deliberately
has no end-user authentication or TLS termination. Keep `.seval-cache` on the
named volume; archive and source bytes live only in the bounded temporary
filesystem and are deleted after each job.

The zero-build Manifest V3 client lives at
[`extensions/github-software-evaluation/`](extensions/github-software-evaluation/).
Its development configuration points only to `http://127.0.0.1:7077`; the
extension README defines the atomic two-file cutover to one production HTTPS
origin.

## Uses

- **A vs B**: expose the artifacts' difference shape and the evidence needed
  for a policy or owner to decide where either is stronger.
- **Before vs after**: test whether a refactor, rewrite, or autonomous-agent
  campaign improved the artifact. The resident
  `evaluations/forward-cycle1-20260709/` bundle is deliberately invalid and is
  retained as an archive-audit negative fixture, not as a canonical verdict.
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

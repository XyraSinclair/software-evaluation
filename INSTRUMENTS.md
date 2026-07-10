# Instruments — how each axis is measured

Orthogonal to *what* is measured (TAXONOMY.md) is *how*. Three instrument
classes recur: mechanical, empirical, and judged. They are not a validity
ranking. Choose the instrument that binds the target claim most directly;
then prefer stronger coverage, replayability, independence, lower cost, and
lower latency. A deterministic proxy can be precisely wrong, while an
empirical run can exactly settle a narrow behavioral contract.

Every observation declares its epistemic class separately:

- **exact** — directly decides a finite or formal contract in the stated scope;
- **proxy** — reproducible evidence correlated with a target construct, with
  the uncovered proxy gap named;
- **judgment** — irreducibly normative evidence whose judge, rubric,
  independence structure, and disagreement remain visible.

Instrument class says how evidence was produced. Epistemic class says what it
can certify. Never infer the second from the first.

## 1. Mechanical — computed by tools

Deterministic functions of the artifact at a commit. Strong replay provenance:
the record is `(tool, version, command, commit) → number`, reproducible by anyone;
its construct validity and coverage still require a separate argument.

| Axis | Mechanical instruments |
|---|---|
| Parsimony | AST metrics (cognitive/cyclomatic complexity *distributions*, not means — e.g. `rust-code-analysis`, `lizard`, `scc`); clone detection (`jscpd`, PMD CPD); dependency-graph stats (depth, fan-in/out entropy, cycles — `cargo-modules`, `madge`, import graphs) |
| Consistency | idiom-entropy greps; internal-link checkers; schema validators run on the artifact's own data files |
| Interface sharpness | public-symbol counts (`cargo public-api`, API extractors); surface-to-volume ratios |
| Correctness machinery | mutation testing (`cargo-mutants`, `mutmut`) — the strongest mechanical oracle-strength measure; coverage **with denominator** |
| Evolvability | co-change graph mined from `git log` (files that change together but live apart = hidden coupling); files-per-commit trend; diff blast-radius stats |

Rules: pin tool versions in the record; report distributions and tails, not
means; a metric only enters a report attached to the claim it supports —
naked numbers invite Goodhart.

### Implemented repository proxies

`seval repo-profile` runs two mechanical criterion programs over a clean,
commit-pinned Git snapshot. They are deliberately **proxy**, not exact:

| Program | Observation denominator | What it can establish | What it cannot establish |
|---|---|---|---|
| `repo.static-shape@1` | blobs in the committed tree, classified by `repo-lexical-v1` | byte/path concentration, category ratios, effective source files and top-level components, largest-file and top-decile shares | correctness, semantic complexity, architecture quality, security, maintainability, or user value |
| `repo.git-change-shape@1` | up to N non-merge commits ending at the pinned revision | change-mass concentration, files-per-commit tails, cross-layout cochange, broad commits, source/test/docs cochange | causal coupling, modularity, work hidden by squash/rebase/merges, rename identity, or history outside the window |

Each produces a separate observation and receipt: exact Git argv, Git version,
SHA-256 of raw stdout, measured bytes and wall time, classifier/protocol version,
and explicit limitations. `seval repo-compare` compares only numeric leaves at
identical JSON Pointer paths from matched program versions. It preserves each
dimension and attaches no good/bad direction to a delta.

## 2. Empirical — exercised behavior

Run the thing and observe. Provenance: the transcript (commands + output),
plus environment description.

- **Cold-start audit** (operational legibility, documentation truth): fresh
  environment, follow only the docs, record time-to-running and every
  deviation the docs forced.
- **Claim audit** (documentation truth): enumerate the README/docs claims as
  propositions — *state the denominator: N claims found, K tested* — and
  test each testable one.
- **Fault injection / edge probing** (robustness): kill it mid-write, feed
  it the hostile input, exhaust the resource; record behavior.
- **Spot-audit against ground truth** (correctness): sample N real
  outputs/records, verify each against source by hand. Report N and the
  sampling method — a spot-audit without its denominator is an anecdote.

Empirical probes obey **probe integrity**: exit codes triaged (error ≠
no-finding), suspect instruments re-run raw, zero-output-on-should-output
aborts the probe rather than passing it.

## 3. Judged — where no oracle exists

Fitness-to-intent, conceptual parsimony of a design, whether an interface is
"honest" — irreducibly judgment. The discipline that keeps judgment from
becoming vibes:

- **≥2 independent judges per axis** — different models, or genuinely
  blind separate passes. Independence is the point; two correlated reads
  are one read. Judges cite concrete evidence (file + content) per verdict;
  uncited verdicts are discarded.
- **Report the spread, never the mean.** Agreement compresses ("both judges:
  A>B on consistency"); disagreement is *the finding* — it localizes where
  the question underdetermines its answer, and it routes to the owner.
- **Adversarial symmetry**: every judge must attack both artifacts (list
  worst defects of each), so a halo on one side has to survive a demolition
  pass on both.
- **Blinding, honestly.** Blind judges to which artifact is "newer/ours"
  where possible; where impossible (changelogs, self-referential state),
  say so in the report and lean harder on evidence-citation and adversarial
  symmetry. A compromised blind stated is worth more than a perfect blind
  claimed.

### Scaling judgment: cardinal-harness

For anything beyond a handful of pairwise calls, use
[cardinal-harness](https://github.com/XyraSinclair/cardinal-harness): it
turns noisy LLM pairwise **ratio** judgments into globally consistent
cardinal scores **with uncertainty**, spends comparisons where they buy the
most information, and prices framing-sensitivity in nats — a judgment only
counts as a *belief* if it survives presentation-order, wording, and
polarity transformations. Every run emits receipts (comparisons, tokens,
cost, stop reason, per-judgment traces): exactly the provenance record
PROVENANCE.md requires. Pattern:

```console
$ cardinal sort artifacts.txt --by "conceptual parsimony: is this the idea, minimally stated?"
```

with one line per artifact@commit (path or précis), axis phrased as the
`--by` criterion, receipts archived under the evaluation's `records/`.

## Instrument honesty table

Every axis verdict in a report carries its instrument class. A report that
supports a strong ordinal claim entirely out of class-3 instruments when
class-1 was available has an instrumentation defect — note it and fix it
before publishing.

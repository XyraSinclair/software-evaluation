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

`seval` currently closes two foundational loops:

1. **Archive integrity:** prove that an evaluation has nonempty records, safe
   and existing evidence/procedure paths, literal report-to-record references,
   commit-pinned artifact identities, and non-vacuous inputs.
2. **Audit planning:** choose probes under hard dollar, time, and count budgets
   using Shannon information gain, expected Bayes-risk reduction, decision-flip
   probability, redundancy-aware conditional information, and Pareto frontiers.

```console
$ cargo run -- audit evaluations/forward-cycle1-20260709
$ cargo run -- plan examples/audit-plan.json
```

The first command is expected to fail: the resident evaluation is deliberately
the auditor's first negative fixture. It exposes missing evidence, ambiguous
comparison identities, unproven exact prompts, and unresolved report aliases.
A failed audit exits 1; an audit harness/input failure exits 2.

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

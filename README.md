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
| [INSTRUMENTS.md](INSTRUMENTS.md) | Mechanical → empirical → judged, in strict preference order. AST-level complexity/clone/dependency metrics, git co-change mining, mutation testing; cold-start and claim audits; triangulated judgment via [cardinal-harness](https://github.com/XyraSinclair/cardinal-harness) for consistent, framing-tested, receipted scores. |
| [PROVENANCE.md](PROVENANCE.md) | The record schema: artifact@commit, instrument, agent, procedure, evidence, verdict, integrity caveats. Honest nulls over neat fictions. |
| [REPORT.md](REPORT.md) | The report template and its rules: spread never mean, adversarial symmetry, denominators always, verdicts carry record ids. |
| `evaluations/` | Actual runs: one directory per evaluation — report, records.jsonl, raw judge outputs and metric dumps. |

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
demolition pass. Overall ordinal verdicts are issued only when the axis
table is lopsided; otherwise the honest answer — "stronger at different
things" — is stated with the split. The most valuable section of a report is
often not the verdict but **Contested axes**: the places where independent
judges genuinely disagree, which is where the owner's own values decide.

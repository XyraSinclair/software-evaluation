# The Report — form and rules

The deliverable of every evaluation is one report that a busy owner can act
on: **which artifact is stronger, how they differ in kind, and where the
genuine open judgments live.** Reports live at
`evaluations/<name>/report.md`.

## Structure

```markdown
# <A> vs <B> — <date>
Artifacts: <name@commit> / <name@commit>     Evaluators: <named, with independence note>

## Kind statement
One paragraph per artifact: what kind of thing it is, which axes it
legitimately waives and why. (Often the deepest finding: they are
different kinds.)

## Axis verdicts
| Axis | Applies | Instrument | Judges | Verdict | Spread |
Per axis: the ordinal verdict with record ids, and the SPREAD across
independent reads — agreement stated in one line, disagreement quoted
in full.

## How they differ
Prose, not table: the shape of the difference. What A is built to be
that B is not. The non-shared axes live here.

## Contested axes — owner's judgment required
Each axis where independent judges genuinely disagreed: the two
positions, the evidence each cites, the grounds on which the owner
must decide. This section may be the report's most valuable output.

## Overall
An ordinal verdict ONLY if the axis table is lopsided (strict-majority
of applicable axes one way, none strongly the other). Otherwise:
"stronger at different things", stated plainly, with the axis split.

## Denominators
Axes evaluated / waived (with reasons). Claims audited K of N. Probes
run, per area. Judges consulted, independence structure. What this
evaluation did NOT cover.
```

## Rules

1. **No composite score.** Axis weights are the owner's values; a weighted
   sum would launder that judgment into a fake number. The report ranks
   per-axis and speaks plainly about the overall shape.
2. **Spread, never mean.** Two judges disagreeing 
   is a finding, not noise to average away. Consensus compresses;
   disagreement gets quoted.
3. **Ordinal claims need adversarial symmetry.** "A>B" is publishable only
   if both artifacts received a demolition pass (worst-defects list) and
   A's survived better.
4. **Every sentence with a verdict carries a record id.** No receipts, no
   sentence.
5. **Denominators always.** "Consistent" means "K drift-pairs found in N
   cross-references checked", not "felt consistent".
6. **Caveats are visible.** A compromised blind, a small sample, a suspect
   instrument — stated in the sentence that depends on them, not in a
   footnote.
7. **Reports are versioned facts.** A report evaluates artifacts at
   commits; it is never edited to track the artifacts' later evolution —
   a new evaluation supersedes it, linked both ways.
```

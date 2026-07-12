# Provenance — every verdict carries its receipt

A quality claim without provenance is an opinion with formatting. The rule
here: **every number, verdict, and comparison in a report resolves to a
record that says exactly how it was produced** — replayable when mechanical,
transcribed when empirical, receipted when judged.

## The record

One JSONL file per evaluation: `evaluations/<name>/records.jsonl`. One
object per measurement or judgment:

```json
{
  "id": "r-014",
  "artifact": "forward@d2e5f03",
  "axis": "internal-consistency",
  "instrument": "judged",
  "agent": {"kind": "model", "id": "codex/gpt-5.6-sol", "effort": "xhigh"},
  "procedure": "judges/codex-artifact-prompt.md",
  "evidence": "judges/codex-artifact-judge.md#internal-consistency",
  "verdict": "A>B",
  "ts": "2026-07-09T11:20:00Z",
  "integrity": "clean"
}
```

Field discipline:

- **artifact** is either `name@commit` (or `name@commit:path` for a
  sub-artifact) or exactly two such identities joined by ` vs ` for a pairwise
  record. Commit pins use at least seven hexadecimal digits. Evaluations of
  uncommitted state are invalid — commit first.
- **instrument** ∈ `mechanical | empirical | judged`.
- **agent**: a nonempty object with string `kind` and `id`, plus tool version
  for mechanical work, runner/environment for empirical work, or model and
  reasoning effort for judged work. Judges are named so correlated reads can
  be detected downstream.
- **procedure**: pointer to the exact command or the exact prompt file. A
  judged record whose prompt was not preserved is void.
- **evidence**: pointer into the raw output archived alongside
  (`judges/`, `runs/`, `metrics/` under the evaluation dir). Raw outputs
  are committed, not summarized-then-discarded.
- **verdict**: the axis-level outcome in the report's vocabulary
  (`A>B`, `B>A`, `tie`, a number with units, `strong|adequate|weak`).
- **integrity**: `clean`, or a named caveat (`blind-compromised`,
  `instrument-suspect-reran-raw`, `sample-n-small`). Caveats propagate: a
  report sentence resting on a caveated record inherits the caveat visibly.
- **ts**: real timestamps of when the measurement actually ran. **Never
  reconstruct timestamps after the fact** — a synthesized time is a false
  receipt, and one false receipt poisons the file's credibility. If the
  true time wasn't captured, write `"ts": null` and say so; an honest null
  outranks a neat fiction.

## Cardinal-harness receipts

When judged axes run through cardinal-harness, its native receipts
(comparisons, per-judgment traces, framing-battery results, cost, stop
reason) are archived under `records/cardinal/` and each derived score gets a
record with `procedure` pointing at the run config and `evidence` at the
receipt directory. The framing-battery result rides along: a score that bent
under reframing is reported *with* its instability, not despite it.

## Chain rules

1. **Report → record → raw evidence**, two hops maximum, both committed.
2. Independence is auditable: the records show which judgments shared a
   model, a session, or an author. A report claiming "two independent
   judges" whose records show the same session id is self-refuting.
3. Self-evaluation is allowed but **labeled**: `agent.relation:
   "self-authored"` on any record where the evaluator produced the artifact.
   Self-records can seed a report; they cannot be its sole support for any
   contested axis.
4. Nothing is deleted: a superseded or refuted record gets
   `"superseded_by": "r-041"`, staying in the file as the trail.

## Committed profile receipts

`seval change-profile` binds current structure and bounded history separately.
The source receipt records the pinned revision, `git ls-tree -r -z --long`
command, Git version, complete tree-stream byte count and SHA-256, ordered
`git cat-file --batch` request SHA-256, and complete batch-response byte count
and SHA-256. Each response is accepted only when its object ID, blob type, byte
size, order, and framing match the tree entry and request. Metrics consume those
returned bytes, not worktree files. The history receipt records the exact
rename-disabled, non-merge Git log command, Git version, raw-output byte count,
and SHA-256. The artifact revision and tree digest accompany both.

These receipts certify the input streams and deterministic protocol. They do
not certify tree-sitter construct validity, the adequacy of the sampled history
window, rename continuity, causality, defect probability, maintainability, or
quality. Those gaps remain visible in the report limitations.

# forward: before vs after its first self-cycle — 2026-07-09

**Artifacts:** `forward@d2e5f03` ("A", post-dogfood) vs `forward@ecb4c6e`
("B", pre-dogfood) · **Evaluators:** judge-artifact-claude (Claude fable,
fresh session), codex-artifact-judge (Codex gpt-5.6-sol xhigh — separate
vendor), judge-process-claude (Claude fable, fresh session, adversarial
framing), self-read (authoring session, labeled `self-authored`).
Independence: two model families; the two Claude judges were separate fresh
sessions; procedure in `judges/PROCEDURE.md`. **Blind: compromised and
declared** (A contains its own cycle log) — mitigated by mandatory evidence
citation and adversarial symmetry (worst-3 defects demanded for both).

This evaluation answers two different questions and refuses to merge them:
**Q1: is the artifact stronger after the self-cycle?** **Q2: was the
self-cycle honest measurement?** The spread between the answers is the
report's central finding.

## Kind statement

Both artifacts are the same kind: prose-as-program — an agent-skill suite
whose "code" is doctrine plus one shell installer plus JSONL state. Waived
axes (declared, per TAXONOMY.md): AST-parsimony (no AST; conceptual
parsimony judged instead), robustness (no runtime), evolvability (one day
of history — no co-change signal), correctness-machinery (installer covered
by claim-audit). A differs from B by exactly one commit: the self-cycle.

## Q1 — Axis verdicts (artifact, A vs B)

| Axis | Judges | Verdict | Spread | Records |
|---|---|---|---|---|
| conceptual-integrity | claude, codex | **A≥B** | claude: "tie, A by a hair"; codex: "A>B" — mild disagreement on magnitude, none on direction | r-c/x-conceptual-integrity |
| internal-consistency | claude, codex | **A>B** | full agreement, same evidence independently (V formula, "ships alongside", orphaned schema doc) | r-c/x-internal-consistency |
| claim-correctness | claude, codex | **A>B (narrow)** | full agreement, including the qualifier | r-c/x-claim-correctness |
| interface-sharpness | claude, codex | **A>B** | agreement; both independently found the `/forward:forward` plugin-namespacing error *shared by both snapshots* | r-c/x-interface-sharpness |
| honesty-infrastructure | claude, codex | **A>B (decisive)** | strongest agreement: B preaches, A practices-with-defects | r-c/x-honesty-infrastructure |
| **overall** | claude, codex | **A>B** | claude "high", codex 0.86 | r-c/x-overall |

Cross-judge convergence was near-total on the defect lists too: both
independently flagged A's `commits:["pending"]`, the `drift` lens schema
violation, and self-graded worth. Codex went deeper on one axis (found the
stopping rule's economic contradiction — the 3-probe low-stakes floor the
`3V` test doesn't license — which the Claude judge's check of the same math
passed because it verified the theorem, not the policy against the theorem:
an instrument-depth difference, not a disagreement).

**Q1 answer: yes — A>B on every applicable axis, cross-vendor, no axis
favoring B.** The dogfood cycle genuinely improved the artifact.

## Q2 — Axis verdicts (process: was the self-cycle honest?)

| Axis | Process judge | Self-read | Spread |
|---|---|---|---|
| genuineness-of-finds | adequate | adequate-to-strong | minor |
| statistical-honesty | **weak** | weak | agreement |
| independence | **weak** | weak | agreement |
| closure | **strong** | strong | agreement |
| self-reference-quality | **weak** ("import wearing a discovery costume") | strong-with-caveat | **CONTESTED — see below** |
| overall honesty | **C+** | "closure beautiful, accounting not" | consistent |

Decisive process evidence (r-e-timestamps): the probe timestamps were
**physically impossible** — every probe postdated the commit recording it.
Plus: one-probe-per-area coverage sweep violating the tool's own
escalation doctrine; the appraisal loop rated its own story "loved."
Credit that survived the adversarial pass: all five finds real, all fixes
independently re-verified, and the truncation confession ("released on
budget, not on silence") was the skill's most anti-theater rule applied
against itself when declaring victory would have been trivial.

**Q2 answer: C+ — real work, honestly narrated, but the measurement layer
was retroactive narration, not measurement.**

## How they differ

B is a well-written constitution that had never governed anything. A is the
same constitution after one day in office: three real repairs, one genuine
doctrine addition (probe integrity), and — the decisive difference — an
auditable trail of its own conduct, including the incriminating parts. A's
sins were *findable in A's own committed ledgers*; that is the property B
entirely lacks and the only reason the process audit was possible at all.

## Contested axes — owner's judgment required

**self-reference-quality**: the process judge calls the probe-integrity
doctrine recycled operator knowledge narrated as discovery (evidence:
RTK.md predates it by 5 days, same example); the self-read and both
artifact judges score the doctrine itself as the cycle's one genuine
conceptual deepening ("real self-application, not merely exhortation").
Both cite true facts. The judgment that remains: **does an honestly-earned
live instance of a known failure class, exported into a portable tool,
count as discovery or as import?** Owner: Xyra. Grounds: what "the tool
learned something" should mean for Forward's benchmark claims. (Worth
ledger already downgraded to the conservative reading.)

## Overall

**A>B, high confidence, both judges, every axis — and the process that
produced A earned a C+.** Both verdicts stand; the honest summary is:
*the self-cycle made the artifact better and made the tool's central claim
(statistical measurement) worse-than-claimed, visibly, in its own ledgers.*
Remediation of every process finding shipped as cycle 2
(forward@8ca5002+), outside this report's scope by rule 7 (reports are
versioned facts); a future evaluation should judge it fresh.

## Denominators

Axes evaluated: 5 artifact (of 9; 4 waived with reasons above) + 5 process.
Judges: 3 independent (2 vendors) + 1 self-authored (seeding only).
Empirical checks: both install paths run by 2 judges independently; JSONL
14/14 lines parsed; rule-of-three math re-derived twice; plugin manifests
validated against Claude Code 2.1.206. Not covered: runtime skill-trigger
behavior in a live session (structural validation only — both judges flagged
this); evolvability (no history); any user-elicited worth (all worth
self-appraised, provisional). Raw judge outputs: `judges/`, verbatim.
Records: `records.jsonl`, 26.

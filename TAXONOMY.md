# The Taxonomy — nine timeless axes

Software quality is not one number. It decomposes into a small set of
**orthogonal axes** — properties that vary independently, are measurable (or
at least triangulable) at a point in time, and have been true of good
software since before our tools existed and will remain true after them.
Timelessness is the admission test: an axis earns its place by mattering for
FORTRAN in 1970 and for prose-as-program in 2026.

The nine axes form a 3×3: three **families** (what kind of virtue) × the
axes within them. Orthogonality claim: an artifact can score high on any
axis while scoring low on any other — each pairing has real-world witnesses
(a parsimonious wonder with no tests; a fortress of assurance nobody can
evolve; a beautifully documented artifact that doesn't serve its intent).

## Family I — Form (the artifact as a static object)

### 1. Parsimony
Is this the computation, minimally stated? The von Neumann axis.
**Signals:** AST-level economy — nodes-per-behavior, cognitive-complexity
distribution (not just the mean: the tail is where readers die), dependency
graph shape (depth, fan-in/fan-out entropy, cycle count), clone density
(type-1/2/3), concept count (distinct abstractions a reader must hold).
For prose-as-program: concepts per capability, redundancy that isn't
deliberate weighting.
**Anti-Goodhart note:** minimal tokens ≠ parsimony; golfed code fails
parsimony because the *reader's* state, not the character count, is the
quantity being minimized. Measure structure, not length.

### 2. Consistency
Does the artifact agree with itself? One naming system, one idiom per
problem shape, schemas that match across every file that mentions them,
docs that match code, examples that run.
**Signals:** drift pairs (two statements of the same fact that differ),
idiom entropy (how many ways the same operation is spelled), broken internal
references, schema-conformance of the artifact's own data files.

### 3. Interface sharpness
The economy and honesty of the surfaces: API surface-to-volume ratio,
illegal-states-unrepresentable, boolean-parameter and stringly-typed smell,
stability contracts stated and kept, error types that mean something.
**Signals:** public-symbol count vs. capability count; how much a caller
must know that the signature doesn't say; changelog churn on public surfaces.

## Family II — Assurance (what certifies behavior)

### 4. Correctness machinery
Not "is it correct" — unknowable — but **what machinery exists to certify
behavior, and how strong is the oracle**. Types, tests, invariants,
property-based checks, proofs; and crucially the *strength* of each: a test
suite that can't fail is machinery-shaped absence.
**Signals:** oracle strength per claim (what would catch a violation?),
mutation-survival rate, coverage *with its denominator stated*, assert
density on invariants actually assumed by the code.

### 5. Robustness
Behavior at the edges: error paths, partial failure, resource exhaustion,
hostile input, the half-completed operation.
**Signals:** error-path-to-happy-path code ratio and whether error paths are
*tested*; failure containment (does one bad record kill the batch?);
idempotency/retry discipline; timeouts and bounds on everything unbounded.

### 6. Documentation truth
Do the artifact's claims about itself hold? Every README claim is a
proposition with a truth value.
**Signals:** claim-audit pass rate (run every documented command, check
every stated number), stale-claim density, the gap between the entry-path
the docs describe and the one that works.

## Family III — Life (the artifact in time and use)

### 7. Evolvability
The cost of the next change. Coupling as it *actually manifests*: the
co-change graph mined from history (files that always change together but
live far apart = hidden coupling; the git log is the ground truth the module
diagram only aspires to), change amplification (lines touched per unit of
behavioral change), the blast radius of a typical diff.
**Signals:** co-change clustering vs. module boundaries; mean files-per-
commit trend; how much a newcomer's first fix actually touched.

### 8. Operational legibility
Can a stranger install, run, observe, and diagnose it? Distance from symptom
to cause when it breaks.
**Signals:** cold-start success (fresh machine, follow the docs, time-to-
running); log/telemetry quality at the moment of a real failure; whether
state is inspectable (plain formats beat opaque ones here — greppable,
diffable, committable).

### 9. Fitness-to-intent
Does the artifact serve its own stated intent — and is the intent stated?
The other eight axes are intent-free; this one binds them: a technically
superb artifact aimed at nothing scores low here, and features that serve no
recorded intent are debt on this axis regardless of their quality.
**Signals:** existence and freshness of an intent record; capability-to-
intent mapping (each capability traceable to an intent, each load-bearing
intent served); deliberate-limitation honesty (does it say what it doesn't
do, and abstain accordingly?).

## Applicability gates

Not all software honors all nine perfectly, **and that is not a defect** —
a research prototype legitimately waives robustness; a one-shot migration
script legitimately waives evolvability; prose-as-program has no AST but has
conceptual parsimony. The rule:

1. Every evaluation **declares, per artifact, which axes apply**, with a
   one-line reason for each waiver. Waivers are part of the report, not
   silent omissions — an undeclared waiver is a hidden zero.
2. Waivers apply to axes, never to families: an artifact that waives all of
   Assurance is claiming to be a sketch; say so in the report's kind-
   statement.
3. **Comparisons run on the axis intersection.** Non-shared axes are not
   scored — they are reported as *differences in kind* ("A is built to be
   operated; B is built to be read"), which is often the most valuable
   finding in the report.

## What the axes are not

- Not a checklist to max out: axes trade off (parsimony vs. robustness at
  the margins; interface stability vs. evolvability), and fitness-to-intent
  is the axis that says which trade-offs were *right*.
- Not weights for a composite score. There is no composite score. See
  REPORT.md: spread is reported, never averaged, because the weighting of
  axes is irreducibly the owner's judgment, and a composite would launder it.

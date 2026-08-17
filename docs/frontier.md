# The mechanical quality frontier

`seval-frontier` is the smallest decision surface over the strongest fast,
deterministic instruments already in this repository. It answers a narrow
question:

> Does one fixed artifact improve every declared mechanical quality proxy
> without regressing any of its guards?

It does **not** assign an absolute score, average unlike coordinates, count
favorable axes, or infer correctness from static shape.

## Commands

```sh
cargo run --release --bin seval-frontier -- profile .

cargo run --release --bin seval-frontier -- compare \
  /tmp/project-before /tmp/project-after

cargo run --release --bin seval-frontier -- compare \
  /tmp/project-before /tmp/project-after --format json
```

A profile exits `0` when at least one directional signal is observed and `1`
when none is. A comparison exits `0` only when it can emit a qualified partial
order. Invalid input or configuration exits `2`.

## The six-signal kernel

The frontier intentionally admits only six directional signals. Each has an
explicit denominator or supporting population, polarity, source observation,
coverage rule, and Goodhart counterweight.

| Family | Signal | Direction | Guarding relation |
|---|---|---:|---|
| reader load | local cognitive-complexity p90 | lower | paired with transitive symbol working set, so helper-soup cannot win merely by shrinking functions |
| reader load | symbol working-set p90 / other resolved symbols | lower | excluded below the declared reference-resolution coverage gate |
| interface depth | shallow-function fraction | lower | cannot establish dominance if either reader-load coordinate regresses |
| effect locality | syntactically pure function fraction | higher | paired with mutable live-range tail; syntactic purity is never treated as semantic purity |
| effect locality | mutable-binding live-range p90 | lower | prevents purity gains obtained by concentrating state into longer-lived mutation |
| uniformity | reported normalized-clone token mass / considered tokens | lower | reaching the clone-group output cap censors the value rather than presenting a lower bound as complete |

The clone coordinate is a **density of reported mass**, not unique token
coverage; overlapping structural clone mass can exceed one.

## Comparison semantics

For every usable signal, the comparator records `right-better`, `left-better`,
or `equivalent`. The order is then the strict Pareto partial order:

- `right-dominates`: at least one signal improves and none regress.
- `left-dominates`: at least one signal regresses from left to right and none
  improve.
- `tradeoff`: at least one signal moves in each direction.
- `equivalent`: every usable signal is numerically equal within floating-point
  noise.
- `no-comparable-signals`: no directional coordinate is usable on both sides.

The report always exposes the order on the observed intersection. It exposes a
`qualified_order` only when all six conditions hold:

1. Clean Git snapshots observed before and after each scan identify the same
   commit and tree, with no tracked or non-ignored untracked changes at either
   observation.
2. Frontier schema versions match the running implementation.
3. Each required analyzer has exactly one complete, error-free receipt with a
   valid observation digest and a named implementation; implementations match
   across the two profiles.
4. Analysis-affecting configurations are exactly equal.
5. Each profile contains exactly one instance of every preregistered signal and
   no undeclared directional signals.
6. Every declared signal is observed with a finite value on both artifacts.

Missingness never silently changes the denominator. Analyzer failure, panic,
unsupported language coverage, low symbol resolution, non-finite values, and
capped clone output remain distinct machine-readable states.

The Git check is deliberately described as **pre/post snapshot stability**, not
immutable read isolation. A transient worktree mutation reverted between those
two observations could escape detection. Materializing one immutable,
content-digested source corpus for all analyzers is the next provenance and
computational-parsimony step.

## Receipts

The four underlying analyzers run concurrently. Each receipt contains:

- analyzer identity and implementation string,
- completion/failure/panic status,
- wall time,
- SHA-256 of the exact serialized observation,
- coverage payload and the analyzer's own limitations,
- the underlying error when present.

The compact frontier report retains projected values rather than duplicating
large symbol and function tables. The digest allows a separately archived raw
observation to be checked against the receipt and independently reprojected.

## What this can establish

A qualified dominance result establishes only this:

> Under one declared mechanical instrument configuration, every admitted proxy
> moved weakly in the same direction and at least one moved strictly.

It cannot establish behavior, correctness, security, performance, operational
fitness, documentation truth, maintainability, product value, or intent. Those
remain separate evidence classes and decision inputs.

## Why this is a frontier rather than a score

The instrument repository already contains many exact and sophisticated
coordinates. More coordinates do not automatically produce more inference.
The frontier is a deliberately lossy compilation target:

```text
rich observations
    -> six preregistered directional projections
    -> family-local guards
    -> global strict Pareto order
    -> optional qualified routing fact
```

No weight vector is hidden in that pipeline. A single regression blocks
dominance regardless of how many other coordinates improve.

## Computational residue

The analyzers currently perform independent discovery, reads, and parsing.
Concurrency reduces wall time but not total work. The next computational
parsimony step is a shared immutable source substrate that performs one walk,
one read, and one parse per file, attaches a content digest, then lends typed
views to each instrument. That refactor should preserve analyzer outputs
byte-for-byte before any new signal is admitted.

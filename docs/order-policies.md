# Exact contingent order-region policies

`seval-order-probes plan` solves the sequential decision problem induced by
a probe model exactly (repo issue #4, robust slice):

```text
surviving worlds -> choose an admissible probe -> observe an outcome
  -> restrict worlds -> stop when one order remains or nothing helps
```

The target is order-region identification: worlds inducing the same Pareto
order stay interchangeable, and policies are judged only by the order sets
they can terminate with.

## The dynamic program

A state is a surviving-world bitset plus the set of still-unused probes;
because probes are one-shot per path, the remaining hard budget is a pure
function of the unused set, so `(worlds, unused)` is a complete memo key.
For each state the solver computes the exact nondominated set of adaptive
policies under three axes, never scalarized:

- worst-case number of Pareto orders still possible at termination;
- worst-case number of additional probe invocations;
- worst-case componentwise cost along any outcome path.

Stopping immediately is always on a state's frontier — worst-case-free
abstention is a genuine nondominated choice. Ties between incomparable
policies are preserved; one representative tree is kept per distinct
signature.

## Terminal classification

Every leaf carries an exact stop status:

- `identified` — exactly one order remains;
- `observationally-indistinguishable` — several orders remain and every
  unused probe is constant on the survivors;
- `budget-exhausted` — informative probes exist but each violates the
  remaining hard budget on some component;
- `elective-stop` — informative admissible probes exist, but stopping is
  itself nondominated (cheaper or shorter than continuing).

No order is ever reported identified because a posterior crossed a
threshold; identification means cardinality one, exactly.

## Hard budgets

An optional per-path budget vector `(dollars, latency_ms, invocations)` is
enforced componentwise on every root-to-leaf path. The solver is purely
worst-case, so no expected-value argument can launder a branch past a hard
budget. The Bayesian frontier (expected posterior order entropy, expected
identification cost, probability of identification within budget) is a
separate objective family and deliberately not implemented here.

## Exact Blackwell pruning

At every state, probe `B` is pruned when another admissible probe `A`,
restricted to the surviving worlds, refines `B`'s outcome partition at
weakly lower componentwise cost with a strict advantage. Restricted to
survivors, `B`'s outcome is a function of `A`'s; a policy opening with `B`
transforms into one opening with `A` in which any later `A` node has become
deterministic and collapses, so the transformation is weakly better on all
three axes and pruning preserves exactness. Every pruning event is recorded
as a certificate naming the state, the pruned probe, and the dominator.

Probes that split survivors without immediately separating orders are still
expanded — restricting the surviving set can make a later probe decisive
(covered by an explicit test). Probes constant on the survivors are dropped
exactly.

## Fail-closed complexity controls

Declared caps on worlds, probes, memoized states, and per-state frontier
width abort with an explicit error instead of presenting a partial frontier
as complete. The emitted report carries `complete: true` precisely because
incomplete reports are never emitted.

## Verification

- An independently written exhaustive oracle enumerates every adaptive
  decision tree — including constant and Blackwell-dominated probes — for
  40 randomized tiny models (with and without budgets) and must reproduce
  the solver's nondominated signature set exactly.
- Metamorphic invariances: renaming worlds/probes/outcomes, adding a
  constant probe, and duplicating a world with identical observations all
  leave the signature frontier unchanged.
- Budget and truncation semantics are pinned by tests: hard budgets yield
  `budget-exhausted` leaves, and exceeding the state cap is an error, never
  a silently partial frontier.

## CLI

```bash
cargo run --bin seval-order-probes -- plan model.json --format json
cargo run --bin seval-order-probes -- plan model.json \
  --budget-dollars 2 --budget-latency-ms 100000 --budget-invocations 4
```

Exit code 0 on success; 2 for invalid models, invalid budgets, or exceeded
solver caps.

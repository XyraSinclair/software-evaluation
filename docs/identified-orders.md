# Sharp identified Pareto orders

`seval-frontier identify` adds a conservative partial-identification layer to
the exact mechanical frontier. It answers:

> Across every value assignment compatible with the declared measurement
> bounds, which Pareto orders remain possible, and is any one order necessary?

It does not attach probabilities to missing values, learn criterion weights, or
replace the exact `qualified_order`. The exact comparator, mathematical
identified set, and provenance-qualified identified conclusion are reported
separately.

## Why identified sets

A censored or coverage-limited observation is not always devoid of information.
Two existing frontier states have directional bounds that follow directly from
the underlying instruments:

| Signal state | Identified interval | Reason |
|---|---:|---|
| finite `observed` value `x` | `[x, x]` | exact under the declared mechanical instrument |
| clone density `censored` at reported value `x` | `[x, +∞)` | omitted lower-severity groups can only add reported clone mass |
| symbol working set with `insufficient-coverage` at `x` | `[x, 1]` | unresolved edges can only enlarge resolved-graph reachability; the normalized fraction is at most one |

Every other missing, failed, censored, or coverage-limited state remains
unidentified until a sound bound is preregistered.

This follows the partial-identification discipline: preserve the whole set of
values licensed by evidence instead of selecting an unjustified point. The
necessary/possible distinction is adapted from robust ordinal regression with
imprecise evaluations. The global relation remains Pareto rather than additive.

## Local attainable outcomes

Let the left and right identified intervals for one coordinate be closed
intervals `L = [l₋, l₊]` and `R = [r₋, r₊]`, allowing `+∞` as an upper endpoint.
For a lower-is-better coordinate:

```text
right-better is possible  iff r₋ < l₊
left-better is possible   iff l₋ < r₊
equivalent is possible    iff L and R overlap
```

For a higher-is-better coordinate, the strict inequalities reverse through the
upper endpoints:

```text
right-better is possible  iff r₊ > l₋
left-better is possible   iff l₊ > r₋
equivalent is possible    iff L and R overlap
```

The implementation applies the exact frontier's floating-point equivalence
tolerance at strict and touching boundaries, so singleton intervals recover the
point comparator rather than creating a second numeric semantics.

These conditions are exact for the two intervals. A local coordinate can admit
one, two, or all three outcomes.

## Sharp global order set

For each of the six signals, the comparator obtains its attainable local
outcomes. It then propagates the only two state bits needed by Pareto order:

```text
has any right-better coordinate?
has any left-better coordinate?
```

After all six coordinates:

| State | Pareto order |
|---|---|
| right only | `right-dominates` |
| left only | `left-dominates` |
| both | `tradeoff` |
| neither | `equivalent` |

The dynamic program explores at most four states rather than enumerating
`3^6` assignments. Its output is the exact set of Pareto orders attainable in
the Cartesian product of the coordinate intervals.

`sharp_order_set.necessary_order` is present exactly when this mathematical set
has one member. The report also exposes two weaker necessary relations:

- `right_necessarily_not_worse`: every attainable order is
  `right-dominates` or `equivalent`;
- `left_necessarily_not_worse`: the symmetric condition.

## Qualification is separate from uniqueness

A unique order can be computed from two arrays of intervals even when the
arrays came from incompatible analyzers or unpinned artifacts. That is a valid
mathematical observation about the arrays, but not a valid software-routing
fact.

The report therefore distinguishes:

- `sharp_order_set.necessary_order`: uniqueness over the declared interval box;
- `readiness.qualified_identified_set`: whether schema, configuration,
  analyzer evidence, canonical registries, interval coverage, and artifact
  identities are compatible;
- `qualified_necessary_order`: present only when both of the above hold.

Configuration drift or an unstable artifact can leave the raw interval order
mathematically unique while suppressing `qualified_necessary_order`.

## Why the product-box approximation is conservative

The true joint identified set may contain dependencies between coordinates. The
current layer deliberately does not invent them; it uses the product of the
marginal intervals, an outer approximation.

Consequences:

- the reported possible-order set can be wider than the true set;
- a unique order over the product box is still unique over every subset of that
  box, so a reported necessary order is not made unsound by ignored
  dependencies;
- dependency modeling can later shrink ambiguity without changing the meaning
  of existing certificates.

## Canonical contract validation

Both exact and identified comparison use one shared ordered contract. It checks:

- exact signal IDs and order;
- exact family IDs, order, and membership;
- polarity;
- source analyzer;
- unit;
- ordered JSON projection pointers;
- allowed status transitions;
- finite nonnegative numeric fields and natural `[0, 1]` domains where
  applicable;
- positive supporting denominators for observed or bounded values;
- exact directional coverage ledger;
- exactly four analyzer evidence in canonical order;
- complete, error-free, covered, named, validly digested analyzer evidence;
- valid and equal analysis configuration;
- structurally valid stable Git snapshot identities.

This prevents two mutually consistent forged profiles from changing a signal's
polarity or projection and obtaining either an exact or identified qualified
order.

## Command

```sh
cargo run --release --bin seval-frontier -- identify \
  /tmp/project-before /tmp/project-after

cargo run --release --bin seval-frontier -- identify \
  /tmp/project-before /tmp/project-after --format json
```

Exit status is `0` only when `qualified_necessary_order` exists, `1` when the
identified set is ambiguous, incomplete, or unqualified, and `2` for invalid
input or configuration.

## Non-claims

The intervals describe measurement bounds already implied by the instruments.
They do not quantify semantic error in cognitive complexity, syntactic purity,
symbol reachability, or clone structure. A qualified necessary order is still
only a routing fact over six mechanical proxies.

No probabilistic dominance claim is made. If empirical calibration later
supports distributions over residual uncertainty, that belongs in a separately
versioned empirical instrument.

## Research lineage

This implementation is a narrow engineering adaptation of several mathematical
ideas rather than a claim to reproduce any paper's full model:

- Guido Imbens and Charles Manski, “Confidence Intervals for Partially
  Identified Parameters,” *Econometrica* 72(6), 2004,
  <https://doi.org/10.1111/j.1468-0262.2004.00555.x>.
- Salvatore Corrente, Salvatore Greco, and Roman Słowiński, “Robust Ordinal
  Regression in case of Imprecise Evaluations,” 2012,
  <https://arxiv.org/abs/1206.6317>.
- Rasmus Bokrantz and Albin Fredriksson, “Necessary and sufficient conditions
  for Pareto efficiency in robust multiobjective optimization,” 2013,
  <https://arxiv.org/abs/1308.4616>.
- Ran El-Yaniv and Yair Wiener, “On the Foundations of Noise-free Selective
  Classification,” *JMLR* 11, 2010,
  <https://jmlr.org/papers/v11/el-yaniv10a.html>.

## Next mathematical step

The identified set makes evidence acquisition explicit: ambiguity is represented
as the set of orders that a further probe could eliminate. The next decision
layer should rank candidate probes by expected reduction of this order set per
unit cost, while preserving dependence groups and hard budgets.

Adaptive submodularity gives a principled target for cases where marginal value
of evidence diminishes as observations accumulate. Any approximation guarantee
would require proving the repository-specific objective satisfies the relevant
conditions; the paper is a guide, not a free theorem:

- Daniel Golovin and Andreas Krause, “Adaptive Submodularity: Theory and
  Applications in Active Learning and Stochastic Optimization,” 2010,
  <https://arxiv.org/abs/1003.3967>.

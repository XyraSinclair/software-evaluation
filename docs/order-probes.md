# Order-information probes

`seval-order-probes` compares candidate evidence-acquisition probes without
scoring them. It answers a narrow question: given a finite set of possible
latent worlds, each inducing one Pareto `PartialOrder` on the quality
frontier, which deterministic probes are undominated ways to learn about that
order?

## Model

A JSON model declares:

- `worlds`: named latent states, each with the frontier order it induces
  (`right-dominates`, `left-dominates`, `tradeoff`, `equivalent`,
  `no-comparable-signals`).
- `priors` (optional): raw nonnegative mass per world. Either every world has
  a prior or none does; partial priors are rejected, never filled in. Supplied
  priors are normalized, and the raw mass plus normalized weights are reported
  as evidence. The reported floating-point weights sum to one up to rounding;
  dominance decisions recompute normalization in exact rational arithmetic
  from the raw masses.
- `probes`: named deterministic experiments. Each maps **every** world to an
  outcome label (partial observation functions are rejected) and carries an
  explicit componentwise cost: dollars, latency, invocations — every
  component required, so a missing cost can never masquerade as a free
  probe. No combined cost magnitude exists.

```json
{
  "worlds": [
    { "name": "regression", "order": "left-dominates" },
    { "name": "improvement", "order": "right-dominates" }
  ],
  "priors": { "regression": 1.0, "improvement": 3.0 },
  "probes": [
    {
      "name": "exact-clone-census",
      "cost": { "dollars": 0.05, "latency_ms": 4000, "invocations": 1 },
      "observations": { "regression": "denser", "improvement": "sparser" }
    }
  ]
}
```

## Two frontiers, deliberately different

### Blackwell-cost frontier

For deterministic finite experiments, "more informative than" in Blackwell's
sense is exactly outcome-partition refinement: experiment A is at least as
informative as experiment B iff every preimage block of A is contained in a
block of B, i.e. B's result can be reconstructed by coarsening A's result.
This order is prior-independent and decision-problem-independent.

A probe is removed from this frontier only when some other probe:

1. has a partition that refines it (strictly finer, or observationally
   identical up to outcome relabeling — partitions are compared label-free,
   with a canonical SHA-256 digest per partition), and
2. costs no more in **every** cost component, and
3. is strictly finer or strictly cheaper somewhere.

### Order-information frontier

Blackwell dominance is stronger than usefulness for this decision. A probe
may be maximally informative about worlds while separating only worlds whose
induced Pareto orders coincide — order-useless information. The second
frontier therefore quotients each outcome's surviving world set down to its
set of possible orders and compares:

- worst-case remaining-order count (lower is better),
- each cost component (lower is better),
- and, only when an explicit prior exists, the expected remaining-order
  count (lower is better), evaluated in exact rational arithmetic from the
  declared raw masses so a floating-point ulp can never evict a
  mathematically tied probe.

The expected count is measure-consistent: an order carried only by
zero-prior-mass worlds contributes nothing, so a measure-zero distinction
cannot decide dominance. The purely possibilistic view of the same probe
lives in the worst-case count, which ignores priors entirely.

Two quantities are reported but are never dominance axes: guaranteed
Hartley information — log2(total orders) − log2(worst-case remaining) — is
a monotone function of the worst case for a fixed model, and the mutual
information I(Order; Outcome) in bits is transcendental and therefore not
exactly computable, which disqualifies it from licensing evictions.

## What is refused

- No probe score, ranking, or information-per-dollar ratio.
- No invented uniform prior when none is declared.
- No collapse of the two frontiers into one verdict; a probe can be
  Blackwell-undominated yet order-dominated, and both facts are reported.
- Unresolved cost/information tradeoffs survive on both frontiers.

## CLI

```bash
cargo run --bin seval-order-probes -- analyze model.json --format json
```

Exit code 0 on success; 2 for unreadable, unparsable, or invalid models
(duplicate names, partial observation functions, unknown worlds, partial or
degenerate priors, non-finite or negative costs).

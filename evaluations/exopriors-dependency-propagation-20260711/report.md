# ExoPriors dependency propagation profile — 2026-07-11

**Artifacts:** `exopriors-main@c95daa69575c141e645c1f7d09df0e89cf62fe40` / `exopriors-nucleus@80c973da663b03977aa61aba5ed77d1ecbf8d0a2`  
**Evaluator:** `software-evaluation@4e4161c6336c91dcc4bb7d4dfe14220d01201a3c`  
**Instrument:** deterministic static file-dependency graph proxy; direction-neutral.

## Profile

| Observation | main | nucleus | nucleus − main |
|---|---:|---:|---:|
| Analyzed source files | 1,198 | 98 | -1,100 |
| Reachable non-self pairs | 15,899 / 1,434,006 | 38 / 9,506 | — |
| Non-self propagation fraction | 1.109% | 0.400% | -0.709% |
| Cyclic source files | 231 / 1,198 (19.282%) | 0 / 98 (0.000%) | -19.282% |
| Cyclic SCCs | 18 | 0 | -18 |
| Largest cyclic SCC | 78 / 1,198 (6.511%) | 0 / 98 (0.000%) | -6.511% |
| Reachability protocol | `computed`; work 1,213,574 / 100,000,000 | `computed`; work 3,822 / 100,000,000 | — |

Both exact reachability computations stayed inside the protocol bounds. Main's observed internal file graph reaches 1.109% of possible ordered non-self pairs; nucleus reaches 0.400%. Main has 231 cycle-participating files across 18 SCCs, including one 78-file SCC; nucleus has no observed cyclic SCC. This describes the conservative resolver's graph. It does not prove nucleus has lower maintenance cost or that main's cycles are defects.

## Highest transitive internal out-count

| main path | reachable | direct | nucleus path | reachable | direct |
|---|---:|---:|---|---:|---:|
| `continual_scraping/src/lib.rs` | 102 | 56 | `crates/serve/src/lib.rs` | 16 | 16 |
| `tools/checks/src/main.rs` | 81 | 2 | `crates/opsctl/src/lib.rs` | 8 | 8 |
| `tools/checks/src/checks/blast_radius_audit.rs` | 79 | 1 | `crates/checks/src/lib.rs` | 5 | 5 |
| `tools/checks/src/checks/bundle.rs` | 79 | 1 | `crates/ingest/src/lib.rs` | 4 | 4 |
| `tools/checks/src/checks/checkout_sync.rs` | 79 | 1 | `crates/serve/src/billing/wire.rs` | 1 | 1 |

## Visualization rule

[`propagation-profile.svg`](propagation-profile.svg) uses aligned small multiples for comparable fractions and a separate hotspot table. It deliberately avoids a radar chart, traffic-light colors, a shared bar for unlike units, or a scalar score. General UI rule: show numerator/denominator/status first; use separate direct-vs-transitive in/out scatterplots for drill-down; show cyclic SCCs as discrete components; compare fixed artifacts with aligned rows or slope lines.

## Evidence boundary

The raw reports and exact command receipts are in `deps-main.json`, `deps-nucleus.json`, and `manifest.json`. The instrument certifies these counts for its observed, resolved, file-level graph. It does not certify runtime loading, conditional edges, semantic coupling, causal change impact, maintainability, correctness, or quality. The two branch snapshots have unrelated histories, so this is an artifact-shape comparison rather than a before/after result.

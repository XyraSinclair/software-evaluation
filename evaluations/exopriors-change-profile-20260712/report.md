# ExoPriors change × structure profile — 2026-07-12

**Artifacts:** `exopriors-main@c95daa69575c141e645c1f7d09df0e89cf62fe40` / `exopriors-nucleus@80c973da663b03977aa61aba5ed77d1ecbf8d0a2`  
**Evaluator:** `software-evaluation@d5c6a59298d4748cdf7118c036c60d119e2b22c1`  
**Instrument:** `seval-change-profile-v1`; deterministic committed change-by-structure proxy; direction-neutral.

These snapshots have unrelated histories. This is an artifact-shape comparison, not a before/after result. “More change,” “more complexity,” and “less reachability” have no intrinsic quality direction.

## Coverage and shape

| Observation | main | nucleus |
|---|---:|---:|
| Tracked regular blobs | 2,168 | 191 |
| Analyzed supported source blobs | 1,202 | 98 |
| Current SLOC | 776,598 | 101,486 |
| Current cognitive total | 82,181 | 6,672 |
| History commits | 200 / 200; truncated | 200 / 200; truncated |
| Committer-time span | 323,989 s (3.750 d) | 308,255 s (3.568 d) |
| Unique sampled-history paths | 620 | 83 |
| Current paths matched to history | 239 | 49 |
| Current paths with no sampled history | 963 | 49 |
| History paths without a current analyzed source row | 381 | 34 |
| Textual line mass on current analyzed paths | 90,820 | 99,921 |
| Textual line mass on history-only paths | 112,878 | 64,855 |
| Total sampled textual line mass | 203,698 | 164,776 |

The current-tree denominator is immutable: source bytes were read from the pinned Git objects, not the worktree. Hidden paths and committed paths matching ignore rules remain in scope; untracked files, symlinks, non-UTF-8 paths, and unsupported extensions remain out. The two join partitions close exactly on both snapshots.

## The profiles differ in kind

Nucleus carries 99,921 lines of sampled textual change on 98 current source files; main carries 90,820 on 1,202. The count windows span similar committer time, but commit policy and branch history remain confounders. The defensible observation is concentration: recent nucleus work is concentrated into a much smaller current source surface.

Nucleus also colocates recent change and current structural complexity in its service core:

| Path | Line mass | Cognitive total | Current SLOC | Commits touched |
|---|---:|---:|---:|---:|
| `crates/serve/src/search.rs` | 7,361 | 441 | 5,467 | 13 |
| `crates/serve/src/ch_executor.rs` | 6,921 | 430 | 6,501 | 8 |
| `crates/serve/src/table_router.rs` | 6,455 | 306 | 3,771 | 8 |
| `crates/checks/src/capabilities.rs` | 5,517 | 331 | 4,717 | 19 |

Main’s highest sampled change mass and highest current cognitive totals are less aligned. Its change-mass extrema sit in the continual-scraping framework and source adapters, while its largest cognitive totals include files with little or no activity in this 200-commit window:

| Path | Line mass | Cognitive total | Current SLOC | Commits touched |
|---|---:|---:|---:|---:|
| `continual_scraping/src/framework.rs` | 7,913 | 507 | 5,737 | 5 |
| `continual_scraping/src/sources/ethereum_longform.rs` | 6,540 | 26 | 550 | 2 |
| `continual_scraping/src/sources/sec_edgar.rs` | 4,696 | 451 | 4,696 | 1 |
| `continual_scraping/src/crawler.rs` | 2 | 1,097 | 9,131 | 1 |
| `continual_scraping/src/bin/forum_shape_scout.rs` | 0 | 996 | 9,162 | 0 |

This split is useful operationally. Nucleus asks for close review of a small set of service-core files where change and structure coincide. Main asks two different questions: whether the actively rewritten scraping framework is settling into a clean design, and whether large, currently quiet structural surfaces remain justified and understandable.

## Size-normalized coordinates

Normalization changes the navigation order and therefore stays separate from absolute exposure. Main’s highest complete textual-mass/current-SLOC coordinate is `continual_scraping/src/sources/ethereum_longform.rs` at 11.891; nucleus’s is `crates/serve/src/table_router.rs` at 1.712. These ratios can be inflated by small, replaced, or recently introduced files. They are drill-down coordinates, not defect probabilities or “risk scores.” Binary-touched paths would have null textual normalization; neither fixed snapshot had a binary-touched current analyzed path in the sampled window.

## Relationship to the dependency profile

The earlier [dependency propagation evaluation](../exopriors-dependency-propagation-20260711/report.md) found 15,899 / 1,434,006 reachable ordered non-self file pairs and 231 / 1,198 cycle-participating source files on main, versus 38 / 9,506 reachable pairs and no observed cyclic SCC on nucleus. Taken together, the instruments establish a real structural difference: main is a broad, multi-surface artifact with more observed static propagation and cycles; nucleus is much smaller, less cross-reachable under the conservative resolver, and currently concentrates work in a handful of service files. They do **not** establish that nucleus is more correct, more maintainable, or better fitted to its intended product.

## Missingness is part of the result

Main has no sampled history for 963 / 1,202 current analyzed paths; nucleus has none for 49 / 98. Conversely, 381 main history paths and 34 nucleus history paths have no current analyzed source row. Those sets can contain deleted files, unsupported current files, and renamed paths because rename continuity is deliberately not inferred. No-history is missingness, not measured zero.

## Receipts and visualizations

- [`main.json`](main.json) and [`nucleus.json`](nucleus.json) contain every current and history-only row, coverage partition, limitation, tree receipt, ordered blob-batch receipt, and history receipt.
- [`main.svg`](main.svg) and [`nucleus.svg`](nucleus.svg) provide paired absolute and normalized language facets with raw-value log ticks, explicit omission counts, coordinate extrema, and a bounded history-only ledger.
- [`manifest.json`](manifest.json) binds the fixed revisions, evaluator commit and binary SHA-256, exact argv, timestamps, exit codes, byte counts, and output digests.

## What remains before a quality judgment

This profile closes a previously missing mechanical question: where bounded
recent change and current source structure coexist. It does not close the
quality target. Comfort that either branch is “about as beautiful as can be”
still requires evidence that the mechanically salient files are correct,
conceptually coherent, sharply interfaced, documented, operationally safe, and
fit for the intended product.

One mechanically grounded next tranche is direct review and behavioral testing
of nucleus’s `search.rs`, `ch_executor.rs`, `table_router.rs`,
`agent_surface.rs`, and `capabilities.rs`; main’s distinct active/structural
extrema call for separate review of `framework.rs`, `crawler.rs`, and the
high-change source adapters. Runtime coverage or mutation evidence would answer
a different missing question. This instrument does not rank those tranches by
owner value. Repeating the profile on later fixed snapshots can show whether
concentration is resolving or merely moving; it must not be interpreted as a
trend until the same protocol has multiple time points.

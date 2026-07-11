# ExoPriors main vs nucleus — mechanical baseline — 2026-07-10

**Artifacts:** `exopriors-main@c95daa69575c141e645c1f7d09df0e89cf62fe40` / `exopriors-nucleus@80c973da663b03977aa61aba5ed77d1ecbf8d0a2`  
**Evaluator:** `software-evaluation@fbcb4445721ecb2f221615598bc7d6993f2dd0df`  
**Evaluators:** mechanical tools only; no independent normative judges.

## Kind statement

These are not two points on one Git lineage: no merge base exists, with 5,682 left-only and 231 right-only commits. The comparison therefore measures two unrelated artifact histories rather than a before/after refactor (`r-scope-relationship`).

The main snapshot is a broad repository estate: 2,168 tracked files, 50,901,476 tracked bytes, and 1,198 analyzed source files. The nucleus snapshot is a much smaller, more concentrated artifact: 191 tracked files, 10,876,324 bytes, and 98 analyzed source files (`r-repo-static`, `r-metrics-comparison`). `metrics-compare` matched zero root-relative source paths, so every aggregate delta is cross-corpus rather than file-paired (`r-metrics-comparison`, integrity caveat `scope-unmatched`).

## Axis verdicts

| Axis | Applies | Instrument | Judges | Verdict | Spread | Records |
|---|---|---|---|---|---|---|
| Parsimony | yes | repository shape, AST metrics, dependency/clone/hotspot proxies | none | **split; no ordinal verdict** | Nucleus is much smaller and has lower function-complexity tails, but its source is more concentrated and both artifacts retain severe hotspots | `r-repo-static`, `r-metrics-comparison`, `r-deps-main`, `r-deps-nucleus`, `r-duplicates-main`, `r-duplicates-nucleus`, `r-functions-main`, `r-functions-nucleus`, `r-files-main`, `r-files-nucleus` |
| Consistency | uncovered | none | none | no verdict | naming/idiom/schema consistency was not audited | — |
| Interface sharpness | partial | reachable Rust plus lexical cross-language API proxy | none | **shape only; no direction** | Nucleus has fewer absolute symbols but much higher symbols/kSLOC | `r-api-main`, `r-api-nucleus` |
| Correctness machinery | partial | structural test inventory | none | **split; no adequacy verdict** | main has more test files and test-line mass; nucleus has more discovered cases/kSLOC; neither suite ran | `r-tests-main`, `r-tests-nucleus` |
| Robustness | uncovered | none | none | no verdict | no fault injection, hostile-input probe, security audit, or runtime exercise | — |
| Documentation truth | partial | committed documentation/source byte ratio only | none | **volume shape only** | main carries materially more documentation relative to source; truth was not audited | `r-repo-static` |
| Evolvability | partial | bounded 200-commit topology proxy | none | **split; no ordinal verdict** | nucleus commits touch fewer files and have fewer broad commits, but cross-top-level cochange is higher and source/test cochange is lower | `r-repo-change`, `r-scope-relationship` |
| Operational legibility | uncovered | none | none | no verdict | no cold start, deploy, recovery, observability, or operator-path exercise | — |
| Fitness-to-intent | uncovered | none | none | no verdict | intended capability parity and owner policy were not supplied | — |

## How they differ

### Size and local complexity

Nucleus contains 101,486 analyzed SLOC versus main's 775,003. Its cognitive complexity is 65.74 per kSLOC versus 105.46; function cognitive p90/p99/max are 3/17/108 versus 5/28/226. Those are analyzer-bounded structural observations, not proof of simpler concepts or better behavior (`r-metrics-comparison`).

The tail still matters. Main's top function, `verify_handoff`, is 1,029 SLOC with cognitive complexity 226; nucleus's top function, `validate`, is 465 SLOC with cognitive complexity 108 (`r-functions-main`, `r-functions-nucleus`). Main's top cognitive file is `continual_scraping/src/crawler.rs` at 9,131 SLOC and 1,097 cognitive; nucleus's is `crates/serve/src/search.rs` at 5,467 SLOC and 441 cognitive (`r-files-main`, `r-files-nucleus`). Nucleus reduces the worst observed function tail but does not eliminate large file hubs.

Nucleus is also more concentrated: its largest source file holds 6.64% of source bytes versus 1.28% on main; its top source decile holds 50.39% versus 38.88%. Its source-file p90 is 133,426 bytes versus 50,362 despite a smaller median. This creates a bimodal shape: many small files around a few very large centers (`r-repo-static`).

### Dependencies and duplication

The dependency proxy found 18 cycles and condensation depth 4 on main, versus zero cycles and depth 1 on nucleus (`r-deps-main`, `r-deps-nucleus`). The extractor is conservative and has known Rust resolution and manifest-source classification gaps; absence of reported cycles is not proof of an acyclic runtime architecture.

Both clone scans returned exactly the configured 1,000-group limit. Main's retained groups cover 202,628 duplicated tokens; nucleus's cover 68,167, but neither number is a complete corpus total (`r-duplicates-main`, `r-duplicates-nucleus`, integrity caveat `max-groups-saturated`). No duplication direction is issued.

### Interface surface

After correcting Rust reachability, main exposes 16,316 reachable/proxy public symbols at 23.07 symbols/kSLOC; nucleus exposes 5,598 at 59.41 symbols/kSLOC (`r-api-main`, `r-api-nucleus`). Nucleus therefore has a smaller absolute surface but roughly 2.6 times the surface density. Whether that is sharp, overly broad, or simply the expected shape of library-heavy Rust code is an unresolved interface judgment.

Adjacent documentation covers 2,843 of main's inventoried symbols and 1,185 of nucleus's: approximately 17.4% versus 21.2%. Adjacency is a lexical documentation proxy, not a truth or usability check (`r-api-main`, `r-api-nucleus`).

### Correctness machinery

Main has 198 dedicated test files, 4,833 discovered cases, and 0.0781 test lines per source line. Nucleus has 3 dedicated test files, 796 discovered cases, and 0.0375 test lines per source line (`r-tests-main`, `r-tests-nucleus`). Nucleus's case density is higher—8.14 versus 6.72 cases/kSLOC—but its cases are concentrated in three files and structural counting does not establish execution, coverage, assertion meaning, or mutation resistance (`r-tests-main`, `r-tests-nucleus`).

In the 200-commit windows, source/test cochange is 43.10% on main versus 11.41% on nucleus (`r-repo-change`). This can indicate weaker test coupling, a different test organization, or a different commit discipline; the proxy cannot choose among those explanations.

### Change topology and documentation volume

Nucleus's 200-commit window has 2.46 mean files changed per commit, maximum 17, and 2.5% broad commits; main has 4.545 mean, maximum 180, and 11% broad commits (`r-repo-change`). Nucleus's cross-top-level pair ratio is 39.26% versus 11.40%, while its effective top-level component count is only 1.28 versus main's 3.79; the layouts are not like-for-like (`r-repo-change`, `r-repo-static`).

Committed documentation bytes equal 3.56% of source bytes on nucleus versus 26.20% on main (`r-repo-static`). This is strong evidence of a documentation-volume gap, but says nothing about accuracy, necessity, or operator usefulness.

## Contested axes — owner's judgment required

No judged pass was run, so this report does not manufacture consensus. The consequential open judgments are:

1. whether Nucleus is intended to replace the whole main capability estate or only a deliberately narrower kernel;
2. whether its higher API density is honest composability or surface inflation;
3. whether smaller commits and lower function tails outweigh concentrated file hubs, low source/test cochange, and the documentation-volume gap;
4. which behaviors, operational contracts, and data paths must be preserved before any migration verdict is legitimate.

These judgments belong to the ExoPriors owner, informed by a capability map, behavioral probes, and at least two independent adversarial reads.

## Overall

**No overall winner.** No preregistered feasibility gate, practical-difference Pareto threshold, or owner policy was supplied. The honest mechanical conclusion is: Nucleus is radically smaller and has better observed function-complexity tails, but it is more source-concentrated, far lighter in documentation/test-file mass, much denser in public surface, and still organized around large file hubs (`r-repo-static`, `r-metrics-comparison`, `r-api-main`, `r-api-nucleus`, `r-tests-main`, `r-tests-nucleus`, `r-files-nucleus`).

## Denominators

- Artifacts: 2 fixed commits and trees; unrelated histories (`r-scope-relationship`).
- Mechanical invocations: 14/14 exited 0; exact argv, timestamps, durations, byte counts, and output digests are in `manifest.json`.
- Repository history: 200 non-merge commits per artifact (`r-repo-change`).
- Source metrics: 1,198 main files and 98 nucleus files; syntax-error files 0/0; matched files 0 (`r-metrics-comparison`).
- Dependency inputs: 1,198/98 supported source files and 23/6 manifests (`r-deps-main`, `r-deps-nucleus`).
- Clone inputs: 4,102,985/556,904 normalized tokens; 1,000/1,000 retained groups at the cap (`r-duplicates-main`, `r-duplicates-nucleus`).
- API inputs: 1,198/98 parsed supported files, syntax-error files 0/0 (`r-api-main`, `r-api-nucleus`).
- Test inventory: 1,198/98 supported files, syntax-error files 0/0; no tests executed (`r-tests-main`, `r-tests-nucleus`).
- Taxonomy: 5/9 axes have mechanical proxy evidence; 4/9 are uncovered; 0 empirical probes; 0 independent judges.
- Not covered: behavior preservation, correctness, security, runtime performance, operational legibility, documentation truth, semantic dependency/clone equivalence, test adequacy, capability parity, user value, or fitness-to-intent.

## Record index

`records.jsonl` contains 16 records: `r-scope-relationship`, `r-repo-static`, `r-repo-change`, `r-metrics-comparison`, `r-deps-main`, `r-deps-nucleus`, `r-duplicates-main`, `r-duplicates-nucleus`, `r-api-main`, `r-api-nucleus`, `r-tests-main`, `r-tests-nucleus`, `r-functions-main`, `r-functions-nucleus`, `r-files-main`, and `r-files-nucleus`. Procedures and caveats are in `PROCEDURE.md`; accepted, open, and ruled-out evaluator findings are in `findings.md`.

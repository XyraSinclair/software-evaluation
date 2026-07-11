# Procedure

## Fixed artifacts

- Left: `exopriors-main@c95daa69575c141e645c1f7d09df0e89cf62fe40`, tree `842137570bbb9072f35bd0ea88208ed287cd8a6d`, detached worktree `/private/tmp/seval-exopriors-main`.
- Right: `exopriors-nucleus@80c973da663b03977aa61aba5ed77d1ecbf8d0a2`, tree `dcb792ec7bb5682c121d3cebaa01a0ac66c10033`, detached worktree `/private/tmp/seval-exopriors-nucleus`.
- Evaluator: `software-evaluation@fbcb4445721ecb2f221615598bc7d6993f2dd0df`, `seval 0.1.0`, release-binary SHA-256 `1640f9d0bcf05a3996b6bf5c02a0a3ab0309ddb6ba7306ae1e5f89bece2ed10e`.

The target worktrees were clean and detached at the listed commits. `origin/main` and `origin/nucleus` were fetched before snapshot creation. Mutable branch names were not used by any measurement command after the snapshots were created.

## Artifact relationship

The relationship probe used the full revisions, not mutable refs:

```console
/usr/bin/git merge-base c95daa69575c141e645c1f7d09df0e89cf62fe40 80c973da663b03977aa61aba5ed77d1ecbf8d0a2
/usr/bin/git rev-list --left-right --count c95daa69575c141e645c1f7d09df0e89cf62fe40...80c973da663b03977aa61aba5ed77d1ecbf8d0a2
```

`merge-base` exited 1 with no output; `rev-list` returned 5,682 left-only and 231 right-only commits. These are unrelated histories. The comparison is therefore artifact-shape comparison, not a before/after lineage.

## Mechanical commands

The following exact argv were run with the committed release binary. Each stdout stream was written verbatim to the named JSON file; each stderr stream was written to the matching `.stderr.txt` file. All 14 commands exited 0 and all stderr files are empty. `manifest.json` records exact argv, completion timestamps, elapsed time, byte counts, and SHA-256 digests.

```console
seval repo-compare /private/tmp/seval-exopriors-main /private/tmp/seval-exopriors-nucleus --format json
seval metrics-compare /private/tmp/seval-exopriors-main /private/tmp/seval-exopriors-nucleus --format json
seval deps /private/tmp/seval-exopriors-main --format json
seval deps /private/tmp/seval-exopriors-nucleus --format json
seval duplicates /private/tmp/seval-exopriors-main --min-tokens 40 --min-lines 5 --max-groups 1000 --format json
seval duplicates /private/tmp/seval-exopriors-nucleus --min-tokens 40 --min-lines 5 --max-groups 1000 --format json
seval api /private/tmp/seval-exopriors-main --format json
seval api /private/tmp/seval-exopriors-nucleus --format json
seval tests /private/tmp/seval-exopriors-main --format json
seval tests /private/tmp/seval-exopriors-nucleus --format json
seval functions /private/tmp/seval-exopriors-main --sort cognitive --top 30 --format json
seval functions /private/tmp/seval-exopriors-nucleus --sort cognitive --top 30 --format json
seval files /private/tmp/seval-exopriors-main --sort cognitive --top 30 --format json
seval files /private/tmp/seval-exopriors-nucleus --sort cognitive --top 30 --format json
```

## Interpretation rules

- No composite score and no axis counting.
- Numeric differences are `right - left`; direction is not quality direction.
- Repository and source observations are structural proxies. They do not establish correctness, security, maintainability, semantic equivalence, operational quality, or fitness-to-intent.
- `metrics-compare` matched zero root-relative source paths. Aggregate values compare whole corpora, not corresponding files.
- Both clone scans returned exactly the configured 1,000-group cap. Clone totals describe retained groups only and cannot support a complete cross-branch clone verdict.
- The one-off tools analyze worktrees rather than commits. `manifest.json` binds their exact command paths and output digests to the clean detached snapshots; their native JSON envelopes do not carry commit/tree identities.
- No ExoPriors test suite, runtime scenario, cold start, fault injection, security probe, or benchmark was executed.
- No independent normative judges were run. Conceptual parsimony, interface honesty, operational legibility, documentation truth, and fitness-to-intent remain unresolved judgments.

## Evaluator repair gate

The first pass exposed four defects that could corrupt or weaken this evaluation. They were repaired in `software-evaluation@fbcb444` before the final receipts were generated:

- archive audit accepted symbolic pins and escaping evidence symlinks and did not enforce instrument, agent, or timestamp shape;
- aggregate metrics omitted syntax-error coverage;
- Rust API inventory counted restricted/private-module items and omitted public-trait methods;
- test inventory failed to inherit skipped-suite state and matched same-stem modules across ambiguous paths.

The corresponding integration targets passed after repair: `audit` 17/17, `metrics_cli` 6/6, `api_surface` 3/3, and `tests_analysis` 4/4. These tests certify only their asserted contracts, not the unresolved gaps in `findings.md`.

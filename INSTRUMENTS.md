# Instruments — how each axis is measured

Orthogonal to *what* is measured (TAXONOMY.md) is *how*. Three instrument
classes recur: mechanical, empirical, and judged. They are not a validity
ranking. Choose the instrument that binds the target claim most directly;
then prefer stronger coverage, replayability, independence, lower cost, and
lower latency. A deterministic proxy can be precisely wrong, while an
empirical run can exactly settle a narrow behavioral contract.

Every observation declares its epistemic class separately:

- **exact** — directly decides a finite or formal contract in the stated scope;
- **proxy** — reproducible evidence correlated with a target construct, with
  the uncovered proxy gap named;
- **judgment** — irreducibly normative evidence whose judge, rubric,
  independence structure, and disagreement remain visible.

Instrument class says how evidence was produced. Epistemic class says what it
can certify. Never infer the second from the first.

## 1. Mechanical — computed by tools

Deterministic functions of the artifact at a commit. Strong replay provenance:
the record is `(tool, version, command, commit) → number`, reproducible by anyone;
its construct validity and coverage still require a separate argument.

| Axis | Mechanical instruments |
|---|---|
| Parsimony | AST metrics (cognitive/cyclomatic complexity *distributions*, not means — e.g. `rust-code-analysis`, `lizard`, `scc`); clone detection (`jscpd`, PMD CPD); dependency-graph stats (depth, fan-in/out entropy, cycles — `cargo-modules`, `madge`, import graphs) |
| Consistency | idiom-entropy greps; internal-link checkers; schema validators run on the artifact's own data files |
| Interface sharpness | public-symbol counts (`cargo public-api`, API extractors); surface-to-volume ratios |
| Correctness machinery | mutation testing (`cargo-mutants`, `mutmut`) — the strongest mechanical oracle-strength measure; coverage **with denominator** |
| Evolvability | co-change graph mined from `git log` (files that change together but live apart = hidden coupling); files-per-commit trend; diff blast-radius stats |

Rules: pin tool versions in the record; report distributions and tails, not
means; a metric only enters a report attached to the claim it supports —
naked numbers invite Goodhart.

### Implemented repository proxies

`seval repo-profile` runs two mechanical criterion programs over a clean,
commit-pinned Git snapshot. They are deliberately **proxy**, not exact:

| Program | Observation denominator | What it can establish | What it cannot establish |
|---|---|---|---|
| `repo.static-shape@1` | blobs in the committed tree, classified by `repo-lexical-v1` | byte/path concentration, category ratios, effective source files and top-level components, largest-file and top-decile shares | correctness, semantic complexity, architecture quality, security, maintainability, or user value |
| `repo.git-change-shape@1` | up to N non-merge commits ending at the pinned revision | change-mass concentration, files-per-commit tails, cross-layout cochange, broad commits, source/test/docs cochange | causal coupling, modularity, work hidden by squash/rebase/merges, rename identity, or history outside the window |

Each produces a separate observation and receipt: exact Git argv, Git version,
SHA-256 of raw stdout, measured bytes and wall time, classifier/protocol version,
and explicit limitations. `seval repo-compare` compares only numeric leaves at
identical JSON Pointer paths from matched program versions. It preserves each
dimension and attaches no good/bad direction to a delta.

### Committed change × structure profile

`seval change-profile REPOSITORY` joins two observations without collapsing
their units. The current side is every regular blob at the pinned revision
whose raw Git path is UTF-8 and whose extension is supported; worktree presence,
bytes, ignore files, hidden-path filtering, and untracked files cannot change
that denominator. The analyzer reads selected object IDs through
`git cat-file --batch`, validates object identity, type, size, order, and
framing, then measures the returned bytes directly. The history side is the
first $N$ non-merge commits emitted for the pinned revision plus one sentinel
commit used only to establish truncation. Rename detection is disabled. Current
and historical rows join on exact raw path bytes.

Per current file, the report retains current SLOC, cognitive and cyclomatic
totals, commits touched, actual-window touch fraction, distinct UTC committer
days, text-commit and binary-touch counts, summed textual additions plus
deletions, first/last observed committer timestamps, and the join/history
status. Cognitive density is
$1000\,C/\mathrm{SLOC}$. Textual change density is
$M/\mathrm{SLOC}$ only when SLOC is nonzero and no sampled binary touch makes
$M$ incomplete; otherwise it is null. No-history paths remain missing, never
measured zero. History-only paths remain a separate ledger.

Source coverage, history coverage, and both sides of the join are explicit and
must close arithmetically. The source receipt binds the revision's `ls-tree`
stream, the ordered blob request, and the complete `cat-file` response with
SHA-256 digests and byte counts; the history receipt binds the exact Git log
stream the same way. JSON is complete. Text sorting is a stated operational
view, not a combined rank. SVG uses shared cross-language domains, raw-value
ticks on log1p-positioned axes, separate absolute and normalized planes,
explicit missingness, and independently named coordinate extrema. Point area
encodes current SLOC only in the absolute plane. None of these coordinates
establishes defect risk, maintenance cost, causal coupling, quality, or a
refactoring requirement.

### Implemented one-off source instruments

The one-off source tools run in-process over a shared tree-sitter file walker.
The walker respects ignore files, does not follow symlinks, and returns stable
root-relative paths. The current grammar set is Rust, Python, JavaScript,
TypeScript/TSX, and Go. Syntax-error trees remain visible and increment an
explicit coverage field rather than silently passing as clean.

| Program | Observation denominator | What it can establish | What it cannot establish |
|---|---|---|---|
| `seval metrics PATH` | recognized, non-ignored source files in the current file/worktree | LOC families, cognitive and standard/modified cyclomatic totals, arguments, exits, ABC counts, per-file Halstead/maintainability means, normalized rates, and nearest-rank function tails | correctness, runtime performance, dependency shape, duplication, test adequacy, security, or quality |
| `seval functions PATH` | AST function spaces, including analyzer-recognized closures | deterministic hotspot rankings by cognitive/cyclomatic complexity, SLOC, arguments, exits, maintainability, or Halstead effort | whether a hotspot is wrong, unjustified, or worth changing |
| `seval files PATH` | recognized source files | deterministic file-level hotspot rankings on the same dimensions | architectural boundaries, ownership, coupling, or fitness-to-intent |
| `seval metrics-compare LEFT RIGHT` | independently analyzed sides; files match only at identical root-relative path and language | raw and normalized `right - left` deltas, function-tail shifts, and matched/left-only/right-only file partitions | an overall winner or any intrinsic good/bad direction |
| `seval deps PATH` | import declarations in recognized source plus direct dependency rows in Cargo, npm, Python, requirements, and Go manifests | declaration evidence, conservative internal resolution, all-edge and resolved-internal fan-in/out, SCCs, cycles, components, condensation depth, and bounded exact non-self transitive reachability | runtime loading, feature/alias/build-condition resolution, transitive/lockfile dependencies, causal change impact, or architectural quality |
| `seval duplicates PATH` | normalized AST leaf-token windows meeting explicit token/line thresholds | maximal non-overlapping structural clone groups, occurrences, and duplicated token/line mass | semantic equivalence, intent, whether duplication is justified, or absence of clones below the thresholds |
| `seval api PATH` | externally reachable Rust declarations plus declarations representable under the other languages' documented lexical publicness rules | symbol rows, kinds, visibility basis, parameters, generics, adjacent documentation, and symbols/kSLOC | runtime reachability outside Rust's resolved module visibility, compatibility, stability, usability, or API quality |
| `seval discipline PATH` | function spaces (functions, methods, closures/lambdas/arrows) in recognized source files, each construct attributed to its innermost space | per function: syntactic purity (no nonlocal write, no `&mut`/pointer parameter, no `unsafe`, no call into a documented per-language effect list) and its four components; mutation census (bindings, mutable bindings, reassignments, shadowings, max mutable live range); shape (params, bool params, statements, single-expression body, unit return, max call-chain length); error shape (`?`/Go err-return propagation, `unwrap`/`expect`, panic-like, broad and empty catches, ignored results); lexical type honesty (string-literal conditions, TS `any`, unannotated params, type-ignore comments); per file magic numbers, magic strings, and global mutable state; repo totals, pure fraction, and nearest-rank tails | semantic purity, effect reachability through unresolved calls, whether a mutation or catch is justified, type-level correctness; per-language uncovered fields report 0 by construction (see the module's limitations) |
| `seval tests PATH` | recognized source/test files and supported framework spellings | test/source lines, discovered/ignored cases including skipped-suite ancestry, assertion-like calls, cases/kSLOC, and conservative path-aware same-stem source/test matches | execution, coverage, mutation survival, assertion meaning, correctness, or test adequacy |

Every report names its analyzer, enumerated/analyzed/skipped counts, evidence
rows, and explicit limitations. JSON preserves raw rows; text defaults to
bounded tables. Run optimized builds (`cargo run --release -- ...`) for
representative latency.

### Dependency propagation profile

For $n$ analyzed source files and the unique resolved internal edges $u \to v$:

- `direct_internal_out_degree(u)` counts distinct resolved internal targets;
  `direct_internal_in_degree(v)` counts distinct analyzed sources.
- `transitive_internal_out_count(u)` and `transitive_internal_in_count(v)`
  count distinct reachable files, excluding self even when a cycle returns to it.
- `reachable_nonself_pairs / possible_nonself_pairs` is the observed graph's
  non-self propagation fraction, where the denominator is $n(n-1)$. It is null
  for $n < 2$, never manufactured as zero.
- Cycle observations retain the number of cyclic SCCs, participating files,
  largest cyclic SCC, and both file-count denominators.

These are file-graph topology coordinates. Higher reach means broader static
reach in the analyzer's observed graph; it does not mean worse maintenance,
causal change impact, or a refactoring requirement. Graph granularity, source
classification, and resolver coverage remain confounders. Exact reachability is
reported only when the graph is within both protocol bounds: 10,000 analyzed
files and a checked 100,000,000-unit traversal-work upper bound
$n(1 + |E_{internal}|)$. `size_limit` and `work_limit` retain direct degrees and
cycle measures but return null transitive counts rather than an approximation.

The construct follows dependency-structure-matrix propagation analysis
([MacCormack, Rusnak, and Baldwin, 2006](https://doi.org/10.1287/mnsc.1060.0552))
and keeps coupling direction/counting explicit as required by
[Briand, Daly, and Wüst, 1999](https://doi.org/10.1109/32.748920). Those sources
support the measurement family, not a universal quality direction or causal
maintenance claim.

**Display contract.** Show the propagation numerator, denominator, status, and
coverage before hotspots. Use separate in/out views: direct degree on one axis,
transitive count on the other, with labeled files for drill-down. Show cyclic
SCCs as discrete components. For artifact comparisons, use aligned profile rows
or slope lines with raw deltas; do not use a radar chart, traffic light, shared
bar scale, or area encoding that implies unlike units are commensurate.

### Directory layout profile

Over the same unique resolved internal edges as the propagation profile, treated
as an undirected simple graph (self-loops dropped, each reciprocal $a \leftrightarrow b$
collapsed to one edge with $m$ the resulting edge count):

- Two directory partitions are scored: `top_level` (a file's community is its
  first path component; root files share community `.`) and `parent_directory`
  (community is the immediate parent directory path).
- Per partition it reports the intra- and cross-community undirected edge counts,
  their cross fraction (null when $m = 0$), and Newman–Girvan modularity
  $Q = \sum_c \left[ e_c/m - (d_c/2m)^2 \right]$ with $e_c$ the edges inside
  community $c$ and $d_c$ the degree sum of its nodes (null when $m = 0$).
- Per-community rows carry the file count, intra-community edges, and the
  directed out/in edges that cross the community boundary.
- The closure $\text{intra} + \text{cross} = m$ holds for every partition.

This establishes whether the on-disk tree is a faithful map of actual coupling:
high $Q$ with a low cross fraction means folders concentrate the edges; a legacy
layout over a well-connected graph shows the opposite. It cannot say which
partition is right — $Q$ compares the directory partition only to a configuration
null model ([Newman & Girvan, 2004](https://doi.org/10.1103/PhysRevE.69.026113)),
a single community scores near zero by construction, and over-splitting inflates
cross-community edges, so $Q$ is a coordinate rather than a target. The graph is
file-granularity only, and conservative import resolution leaves unresolved edges
absent, so heavily unresolved code yields a partial partition.

### Co-change layout profile

Over the *same* two directory partitions as the static layout profile, but a
different graph — the git co-change graph rather than resolved imports. The
universe is every source-classified tracked blob with a UTF-8 path at the pinned
revision (the change-profile eligibility rule minus its language-parser
requirement, so a source file in any language participates). Each non-merge
commit that touches $k \geq 2$ in-universe files contributes total pair mass
$1$, spread as $1/\binom{k}{2}$ over every unordered pair of those files
([Geipel & Schweitzer, 2012](https://doi.org/10.1109/TSE.2012.18)):

- The two partitions are the identical `top_level` and `parent_directory`
  communities the static profile scores.
- Per partition it reports the intra- and cross-community pair mass, their cross
  fraction (null when the total mass $W = 0$), and the weighted Newman
  modularity $Q = \sum_c \left[ e_c/W - (d_c/2W)^2 \right]$ with $e_c$ the mass
  fully inside community $c$ and $d_c$ the incident mass (null when $W = 0$).
- Per-community rows carry the file count, intra mass, and the crossing mass
  incident to the community (counted at both of its endpoints).
- Total mass equals the eligible-commit count exactly; pair weights are stored
  as fixed-point integers (scale $2^{40}$), so $\text{intra} + \text{cross} = W$
  closes under integer addition while the reported quantization bound caps the
  truncation. Rename detection is disabled and commits touching more than a
  documented cap (100 in-universe files) are counted and excluded as broad.

This is the maintenance-activity counterpart to the static layout $Q$: it asks
whether the tree maps how the code *changes*, not how it *imports*. Read as a
2-D coordinate against the static $Q$, never subtracted into one congruence
score. **High static / low co-change** is a clean-looking layout that everyday
maintenance keeps crossing; **low static / high co-change** is a tree whose
imports cross boundaries but whose edits stay local. It shares the static
profile's caveats — $Q$ only compares one partition to a configuration null,
a single community scores near zero, over-splitting inflates crossing mass — and
adds its own: co-change is correlation of edits (not causal coupling), squash and
rebase rewrite the observed history, and commit granularity is Goodhart-exposed
because splitting a coupled edit across commits lowers crossing mass for free.

### Metric admission queue

The next useful families are ordered by decision value and instrument honesty
(see [DETERMINANTS.md](DETERMINANTS.md) for the generative/diagnostic split that
sets this order):

1. Add compiler- or language-server-resolved call, type, member, and inheritance
   edges as separate coupling relations; never collapse them into one CBO value.
   This is the common substrate under the generative determinants (reader
   working set, effect depth, module depth, tree-vs-graph modularity) and
   therefore leads the queue.
2. Mutual-reachability fraction from existing SCC sizes; weighted co-change
   directory modularity beside the static layout profile (both operand-ready;
   see DETERMINANTS.md "Graph invariants").
3. Module depth from existing API/metrics operands; branch symmetry and
   cyclomatic–cognitive gap from existing operands.
4. Import native runtime coverage artifacts as executed/coverable counts and
   uncovered locations with run provenance; coverage is not correctness.
5. Add TCC/LCC method-state cohesion only for languages and constructs where
   instance-field access can be resolved, preserving method-pair denominators.
6. Add build/test queue and execution tails when CI telemetry is available;
   those measure developer friction, not intrinsic source quality.

Universal smell thresholds, letter grades, weighted maintainability scores,
LCOM variants without explicit definitions, learned readability scores, and
defect/change probabilities without temporally held-out calibration remain
inadmissible. Size alone can confound apparent OO-metric associations
([El Emam et al., 2001](https://doi.org/10.1109/32.935855)); every future family
must retain size, language, extraction coverage, and missingness beside values.

### Implemented direct benchmark receipts

`seval bench -- PROGRAM ARGS...` invokes exact argv without a shell. It records
every warmup and measured attempt, stdout/stderr bytes, exit or signal,
timeout/termination state, and monotonic elapsed time. The first measured run
is separate; the remaining samples produce nearest-rank p50/p95/p99/min/max/
mean latency and optional units/s or bytes/s. Failed and timed-out samples stay
in the denominator. Peak RSS is currently an honest null: no reliable portable
per-child mechanism is wired to the wait path. One run is not called cold-cache
truth, and the receipt is not a performance score.

## 2. Empirical — exercised behavior

Run the thing and observe. Provenance: the transcript (commands + output),
plus environment description.

- **Cold-start audit** (operational legibility, documentation truth): fresh
  environment, follow only the docs, record time-to-running and every
  deviation the docs forced.
- **Claim audit** (documentation truth): enumerate the README/docs claims as
  propositions — *state the denominator: N claims found, K tested* — and
  test each testable one.
- **Fault injection / edge probing** (robustness): kill it mid-write, feed
  it the hostile input, exhaust the resource; record behavior.
- **Spot-audit against ground truth** (correctness): sample N real
  outputs/records, verify each against source by hand. Report N and the
  sampling method — a spot-audit without its denominator is an anecdote.

Empirical probes obey **probe integrity**: exit codes triaged (error ≠
no-finding), suspect instruments re-run raw, zero-output-on-should-output
aborts the probe rather than passing it.

## 3. Judged — where no oracle exists

Fitness-to-intent, conceptual parsimony of a design, whether an interface is
"honest" — irreducibly judgment. The discipline that keeps judgment from
becoming vibes:

- **≥2 independent judges per axis** — different models, or genuinely
  blind separate passes. Independence is the point; two correlated reads
  are one read. Judges cite concrete evidence (file + content) per verdict;
  uncited verdicts are discarded.
- **Report the spread, never the mean.** Agreement compresses ("both judges:
  A>B on consistency"); disagreement is *the finding* — it localizes where
  the question underdetermines its answer, and it routes to the owner.
- **Adversarial symmetry**: every judge must attack both artifacts (list
  worst defects of each), so a halo on one side has to survive a demolition
  pass on both.
- **Blinding, honestly.** Blind judges to which artifact is "newer/ours"
  where possible; where impossible (changelogs, self-referential state),
  say so in the report and lean harder on evidence-citation and adversarial
  symmetry. A compromised blind stated is worth more than a perfect blind
  claimed.

### Scaling judgment: cardinal-harness

For anything beyond a handful of pairwise calls, use
[cardinal-harness](https://github.com/XyraSinclair/cardinal-harness): it
turns noisy LLM pairwise **ratio** judgments into globally consistent
cardinal scores **with uncertainty**, spends comparisons where they buy the
most information, and prices framing-sensitivity in nats — a judgment only
counts as a *belief* if it survives presentation-order, wording, and
polarity transformations. Every run emits receipts (comparisons, tokens,
cost, stop reason, per-judgment traces): exactly the provenance record
PROVENANCE.md requires. Pattern:

```console
$ cardinal sort artifacts.txt --by "conceptual parsimony: is this the idea, minimally stated?"
```

with one line per artifact@commit (path or précis), axis phrased as the
`--by` criterion, receipts archived under the evaluation's `records/`.

## Instrument honesty table

Every axis verdict in a report carries its instrument class. A report that
supports a strong ordinal claim entirely out of class-3 instruments when
class-1 was available has an instrumentation defect — note it and fix it
before publishing.

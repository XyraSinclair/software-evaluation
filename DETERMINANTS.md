# Determinants of beautiful software — the mechanizable subset

Working thesis: code is beautiful when a competent reader can predict its
behavior with **minimal held state**, and when every surface it presents is
**honest** about what lies behind it. [TAXONOMY.md](TAXONOMY.md) names the
families; this file names the concrete determinants inside them that can be
measured **fast and deterministically** at AST / symbol-graph / type / git
level, and separates the ones a writer can steer by *while writing*
(generative) from the ones that only detect ugliness after the fact
(diagnostic).

Every determinant is a proxy. It enters a report only with its denominator,
its epistemic class, and the Goodhart mode that would satisfy the number
without producing beauty. No composite, no thresholds, no grades.

Status: **have** (in `seval`), **next** (admitted, being built), **later**
(admitted; needs the resolved symbol graph, a type checker, a corpus model, or
an external tool), **out** (declined; reason given).

## The split that matters: generative vs diagnostic

A determinant is **generative** when its optimum is *interior or relational* —
a target you cannot hit without doing the structural work that *is* the
beauty. It is **diagnostic** when its optimum is a *boundary you back away
from* — a floor satisfiable trivially-badly (zero clones via a hyper-DRY
tangle; complexity below N via helper-soup). Diagnostics are necessary, not
sufficient; optimizing them directly yields the absence of named uglinesses,
not the presence of beauty.

The sting: the generative determinants are relational and mostly need a
**resolved symbol graph** (call / type / member edges) that the tree-sitter
substrate does not have; the diagnostic ones are cheap because they are local.
The current tool is an excellent ugliness sieve. The generative half unlocks
from one investment — the symbol graph — which therefore moves to the head of
the admission queue (INSTRUMENTS.md).

## Generative core — hold these in mind while writing

Five structural determinants plus uniformity. A writer who steers by them
produces beautiful functional software with high probability, because the
diagnostic smells become structurally unavailable.

### G1. Minimal joint reader working set

*Definition.* For a function, the transitive set of distinct definitions
(functions, types, constants, across files) a reader must open to predict its
behavior, via the resolved call/type graph; plus within-unit cognitive load.
The quantity to minimize is the **joint peak** of (within-unit load,
cross-unit fan-out): inlining everything detonates the first, helper-soup
detonates the second, the beautiful minimum is interior. This pair is the most
important mutually-policing pair in the suite; cognitive complexity must never
be reported alone as a beauty proxy.

*Denominator.* External definitions per function; distribution and tail.
*Cannot establish* whether the referenced definitions are individually good.
*Feasibility.* **later** (symbol graph). Same-file approximation is **next**.

### G2. Deep modules, honest interfaces (Ousterhout / Parnas)

*Definition.* Per module (file, then directory):
`depth = Σ internal-only cognitive complexity / interface_information`, where
`interface_information` = Σ over public symbols of (params + generics +
return-type token complexity + doc obligation). Report the distribution across
modules and the **shallow tail** — wrappers whose interface costs as much to
learn as their body. Complement with **information-hiding leakage**: the
fraction of public signatures whose types come from the module's own private
imports (uses B, exposes B's types ⇒ has not hidden B).

*Goodhart.* The god-object: one giant module, tiny interface, maximally
"deep". Cross-check with internal cognitive distribution and conceptual
cohesion (D5) — depth is a virtue only conditioned on an organized interior.
Signature compression into opaque `Context`/`Options` blobs is countable
(single-param functions taking a struct with >N fields).

*Feasibility.* Depth from `seval api` + `seval metrics` operands: **next**.
Leakage: type-to-module resolution — Rust/TS tractable, Python/Go partial:
**later**.

### G3. Effects at the edges (functional core, imperative shell)

*Definition.* A function is **syntactically pure** when its body has no write
to a binding it does not own (captured/global/nonlocal write, field assignment
on a parameter, write through `&mut` param/self), no `unsafe`, and no call into
a documented per-language effect namespace (I/O, time, random, process,
network, filesystem, logging). Report the pure fraction and — with the call
graph — the **impurity-depth profile**: beautiful shape is impurity clustered
at entry edges and leaves, a pure core between. Mutation *escape* is the
quantity; a per-function `let mut` census is not (idiomatic Rust mutates locals
in hot loops and is beautiful).

*Goodhart.* One giant `&mut World`/`Context` threaded everywhere — global state
in a trenchcoat. Counter: measure **reachable mutable state size** per
function (fields reachable through `&mut` params), not just syntactic
non-locality.

*Cannot establish* semantic purity (unresolved calls, trait dispatch, closures
passed as arguments, wall-clock through a parameter).

*Feasibility.* Per-function syntactic purity, nonlocal writes, mut params,
effect calls: **have** (`seval discipline`). Depth profile and reachable
mutable state: **later** (symbol graph).

### G4. The tree tells the truth about the graph (Hickey: decomplect)

*Definition.* Over the resolved internal dependency graph (**have** the file
graph): Newman modularity Q of the directory partition; fraction of edges
crossing directory boundaries versus what a detected community partition
(Louvain) leaves crossing; fraction of files inside a dependency cycle
(MacCormack & Sturtevant 2016 tie cycle membership to defects/churn).

*Establishes.* Whether the layout is a faithful map of actual coupling. When
the graph is right and the folders are a legacy accident, that is the finding.
*Goodhart.* One directory "matches" trivially — but Q is defined against a null
model and a single cluster scores badly; over-splitting inflates cross-edges.
Natural interior optimum: among the most Goodhart-resistant here.

*Feasibility.* From `seval deps` output, file granularity: **next**. Symbol
granularity: **later**.

### G5. Invariants live in the types (illegal states unrepresentable)

*Definition.* Lexical proxies now: `bool` fields/parameters, ≥2
`Option`/nullable fields in one struct (phase encoded as optionality),
string-literal comparisons in conditionals (stringly typed), sentinel returns
(`-1`, `""`, `null`), catch-all `_ =>` arms, TS `any`/`unknown` abuse,
`# type: ignore`/`@ts-ignore`, Python annotation coverage, primitive obsession
in signatures (raw `String`/`u64` where a newtype belongs), `unwrap`/`expect`/
`panic!` outside tests. Strong forms — enums with payload, newtypes,
exhaustive matches, `Result`/`Option` returns — counted beside them. Both
counts with denominators (fields, parameters, conditionals, returns).

*Goodhart.* Two-variant enums that are semantically still booleans; newtypes
with no constructor discipline; `any → unknown` everywhere. Tree-sitter cannot
tell constraint from costume — the lexical version is a proxy for a proxy and
the report must say so.

*Feasibility.* Lexical: **have** (`seval discipline`, partial). Real: **later** (type
checker).

### G6. Uniformity — one concept, one place, one spelling

*Definition.* Structural clones (**have**); **idiom entropy** per operation
shape (string formatting, iteration, error propagation, optional handling,
assertion, import ordering); naming-convention conformance per identifier
role; sibling-file skeleton similarity; **single-use indirection** (functions/
types with one reference, pass-through wrappers, traits/interfaces with one
implementor, indirection chain length); optionally **self-consistency
entropy** — cross-entropy of each file under a small model trained on this
repo (Hindle et al. 2012; Ray et al. 2016 show buggier code is less natural),
which catches idiom drift no hand-coded idiom list will, and is an attention
router only (it cannot tell surprising-because-ugly from
surprising-because-brilliant).

*Goodhart.* Benign for entropy/formatting — the gamed state (uniform code) is
the desired state, so these are safe to optimize and gate on. Inlining to kill
single-use wrappers raises clones — report the pair.

*Feasibility.* Idiom entropy, naming: **next**. Single-use indirection:
same-file **next**, cross-file **later**. Self-consistency entropy: **later**
(corpus model, out of the pure-AST lane).

## Diagnostic set — locate ugliness for a human to look at

Cheap, high-precision on "something is wrong *here*", low generative value.
Report as distributions and tails; use as review routers, never gates.

| # | Determinant | Definition sketch | Status |
|---|---|---|---|
| D1 | Local-complexity family | cognitive tail (**have**); **cyclomatic–cognitive gap** per function (flat 12-arm match: high cyclomatic, low cognitive, fine; nested tangle: cognitive ≫ cyclomatic, ugly) — free from existing operands; **max nesting depth** as a headline distribution (language-agnostic, un-gameable-while-staying-ugly) | have / next |
| D2 | Branch symmetry / special-case elimination (Torvalds's "good taste") | per `match`/`switch`/`if-else` chain, pairwise AST-skeleton similarity of arm bodies (reuse clone normalization): near-identical arms ⇒ latent loop/data; one arm markedly more complex than siblings ⇒ the lurking special case. Cross-checked by clone detection (copy-paste uniformity fires there) | next |
| D3 | Type-honesty smells | `unwrap`/`expect`/`panic!`, `any`, `type: ignore`, empty catches, broad `except`, ignored `Result`s (G5's lexical half) | have (`seval discipline`) |
| D4 | Mutation census | mutable bindings, reassignments, shadowing, mutable live range per function — cheap and language-relative; a diagnostic tail-finder, not a target (see G3) | have (`seval discipline`) |
| D5 | Conceptual cohesion (Marcus & Poshyvanyk 2005) | within-module tightness of function identifier vocabularies vs repo baseline; sidesteps LCOM's field-resolution needs | later (embedding/LSA) |
| D6 | Unit-return without disclosure | unit-returning functions whose signature has no `&mut`/receiver and whose body has effect calls (hidden effect); `fn f(&mut self) -> ()` is honest — the signature *is* the disclosure | have (derivable from `seval discipline` JSON: unit_return ∧ mut_params=0 ∧ effect_calls>0) |
| D7 | Magic literals; global mutable state | numeric/string literals outside const/config/test files (excluding 0/1/-1/2); `static mut`, module-level reassigned bindings, `global` writes | have (`seval discipline`, file level; magic strings are a volume, not a smell count) |
| D8 | Commented-out code; TODO/FIXME/HACK density | comment bodies that parse as code; markers per kSLOC with blame age | next |
| D9 | Dead private code | unreferenced non-public definitions | later (symbol graph) |
| D10 | Rework / hidden co-change coupling | lines rewritten within N days; fix-after-fix chains; revert rate; co-change far apart in tree (**have** the co-change half). Best-validated signals in the literature (Nagappan & Ball 2005; Hassan 2009) — against *defects*, not beauty. Evolvability axis only; never near a beauty claim | have / later |
| D11 | Formatter / lint conformance | `rustfmt --check`, `prettier --check`, `ruff`, `clippy` per kSLOC — exact, external, hygiene | later (external tool evidence) |
| D12 | Boolean-expression complexity | operators per condition, negations, mixed `&&`/`||` without parens, nested ternaries; largely subsumed by nesting + symmetry | next (cheap, low weight) |
| D13 | Naming length-vs-scope | identifier length correlated with scope length; vocabulary size vs definition count | later |
| D14 | Encapsulation | public field fraction; `pub` vs `pub(crate)` | next |
| — | Clones, dependency topology, API surface, tests inventory, git change shape | — | have |

## Ruled out or downranked

| Determinant | Reason |
|---|---|
| Token/character count as parsimony | golfing; reader state, not length, is the quantity |
| Definition ("newspaper") order | no consensus direction (kernel C callee-first, Go caller-first); jump-to-definition made physical order cosmetic; consistency-of-choice is legitimate but low value |
| Halstead / maintainability index | poor empirical standing (Shepperd 1988; Hamer & Frewin 1982); redundant with SLOC + cognitive; kept as legacy output, never featured |
| Iterator-vs-index loop style, getter/setter density, out-parameters | language-relative noise; fold loop style into idiom entropy |
| Comment-to-code ratio | direction-free; commented-out code and doc coverage are the honest pieces |
| Learned readability scores without held-out calibration *and* a size partial | Buse & Weimer 2010 is the existence proof that the calibrated version is achievable; Posnett et al. 2011 found the size confound — both conditions are required |
| Composite "beauty score" | quality is a shape; weights are the owner's |

## Validation — earn admission on incremental validity

There is no criterion standard for beauty; every arm is convergent, and the
report must never launder a proxy into the latent variable. The one test that
culls the field: **incremental validity after partialling out SLOC and
cognitive complexity** (El Emam et al. 2001 — most OO-metric associations
dissolved under a size control). Predicted casualties: mutation census,
definition order, loop style, Halstead/MI. Run the partial before building the
instrument where the operands already exist.

1. **Refactor-delta mining (primary; cheapest per bit).** Commits whose
   messages say refactor/simplify/clean/extract/inline and whose test
   assertions do not change; measure each determinant's delta. Human-chosen
   cleanups, un-cherry-picked. Confound to state: cleanups also serve
   performance and fashion.
2. **cardinal-harness convergent arm.** Pairwise ratio judgments — "which
   requires less held state to predict its behavior?" — ≥2 independent judges,
   framing sensitivity priced; report each determinant's *partial* correlation
   with the cardinal axis controlling for size + cognitive.
3. **Refactor-target prediction (temporal hold-out).** Does high-determinant
   code today predict where humans rewrite tomorrow?
4. **Consensus corpora (smoke test only).** SQLite, Redis, Go stdlib, Norvig's
   spell corrector, TigerBeetle, Rust std slices vs documented-painful legacy
   modules — catches backwards metrics, does not confirm plausible ones.

Rework rate is not a validation target — volatile domains rework beautiful
code.

## Challenge to the thesis

1. **Beauty is fit, and fit is invisible to intrinsic metrics** (Alexander;
   Hickey; the fitness-to-intent axis). A gorgeous parser and an
   over-engineered CRUD form can share every intrinsic distribution. The
   maximum an intrinsic suite certifies is *not ugly* — a necessary condition.
   Publish predictive value on ugliness; never claim beauty detection.
2. **Size eats the naive program** — hence the incremental-validity gate.
3. **Paradigm and language relativity** — score within language as
   distributions and percentiles; never absolute thresholds.
4. **Measuring changes the writer.** Once published, the suite measures
   compliance. Defense: keep the generative set small and structural (hard to
   fake while staying ugly), keep diagnostics as review routers, hold the
   no-composite line against every "just give me one number".
5. **The right product is an attention router**: cheap sieve finds the twelve
   functions and three modules a master would wince at, ranked by how many
   independent smells co-fire, each with cited evidence; the generative theory
   is what gets *taught*; the judged instrument adjudicates the flagged spots.
   The irreducible remainder — whether this beautiful structure was the right
   structure for this problem — stays the master's.

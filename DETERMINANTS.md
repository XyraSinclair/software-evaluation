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

*Feasibility.* Interface-width × interior-volume pairs with the shallow
tail: **have** (`seval shape`). Interface-*information* depth (`seval api`
operands): **next**.
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

*Feasibility.* Directory-partition Q, deterministic detected-community
(Louvain) witness comparison, cross-directory edge fraction, and cycle
membership at file granularity: **have** (`seval deps` layout + propagation
profiles). Symbol granularity: **later**.

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
| D1 | Local-complexity family | cognitive tail (**have**); **cyclomatic–cognitive gap** per function (flat 12-arm match: high cyclomatic, low cognitive, fine; nested tangle: cognitive ≫ cyclomatic, ugly) and **max nesting depth** distributions: **have** (`seval shape`) | have |
| D2 | Branch symmetry / special-case elimination (Torvalds's "good taste") | per `match`/`switch`/`if-else` chain, pairwise AST-skeleton similarity of arm bodies (reuse clone normalization): near-identical arms ⇒ latent loop/data; one arm markedly more complex than siblings ⇒ the lurking special case. Cross-checked by clone detection (copy-paste uniformity fires there). Arm-size ratio + large no-else then-arms: **have** (`seval shape`); AST-skeleton arm similarity: next | partial |
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

## Graph invariants — the adjudicated set

Triangulated 2026-08-15 across three independent seats (GPT-5.6 Sol, Claude
Opus, Gemini) on the question "the deepest, simplest graph/spectral invariants
of quality." Unanimous verdict: **spectral scalars are the wrong shape of
number for this tool** — uncertified repository-wide eigenvalue headlines
behave like means, need a null ensemble to mean anything, carry a floating-point determinism tax, and their
eigenvectors are genuinely non-unique under the twin nodes generated code
creates (Davis–Kahan). Spectral machinery earns one narrow role: as an
internal *algorithm* (a partitioner/separator finder) whose reported output is
an integer witness. The later exact per-SCC $\rho$ interval is a narrower
exception: a bounded certificate of closed-walk growth, never a repository-wide
verdict. Killed as headlines, 3/3 seats: λ₂ as a value, unbounded global spectral
radius (zero on every DAG regardless of reach), von Neumann entropy, Estrada /
natural connectivity, Kirchhoff / effective resistance, eigengap module count,
principal-angle tree alignment (dominated by Q), motif censuses as verdicts
(Valverde & Solé 2005 found size/duplication explains most motif prevalence),
persistent homology on unweighted code graphs (β₀ = c, β₁ = m − n + c — it
*is* components plus circuit rank; H1 of a chosen filtration is a complicated
summary of the chosen weight).

Undirected circuit rank b₁ = m − n + c died in adjudication despite one seat
ranking it first: it conflates diamonds (benign reconvergent reuse) with
feedback, is fully determined by (n, m, c) so a degree-preserving null gives
exactly the same value, and McCabe 1976 is the cautionary precedent (cyclomatic
complexity *is* the circuit rank of the CFG, with the same bluntness).
Measured on this repo's src graph as a smoke test, b₁ convicted itself:
undirected n=26, m=23, c=3 gives b₁=0 — "a forest" — while the directed graph
holds a 7-file SCC (F↔ = 42/650 = 0.065). Reciprocal import pairs collapse to
single undirected edges, so b₁ cannot see exactly the feedback that matters.

What survives, in implementation order:

| # | Invariant | One number, one meaning | Status |
|---|---|---|---|
| 1 | **Mutual-reachability fraction** F↔ = Σᵢ sᵢ(sᵢ−1) / n(n−1) over SCC sizes sᵢ, per relation (imports now; calls/types later) | the fraction of ordered pairs with no one-directional explanation — "how much of this can never be read in isolation"; exact integers, decomposes the existing propagation fraction into mutual vs one-way; one 100-node SCC ≫ fifty 2-cycles, correctly. Known confounder to name in reports: Rust `mod.rs` parent↔child declaration edges create reciprocal pairs (this repo's 7-file SCC is exactly its `service/` module). Empirical backing: MacCormack & Sturtevant 2016, Oyetoyan et al. 2013 | have (`seval deps` propagation) |
| 2 | **Co-change directory Q** — weighted Newman Q of the *same* directory partition over the git co-change graph, each k-file commit contributing total pair mass 1 (1/C(k,2) per pair) | does the tree contain actual maintenance activity, not just declared imports (Geipel & Schweitzer 2012: static deps and co-change diverge badly). Read beside static Q as a 2-D shape, never subtracted into a congruence score: high/low = clean-looking layout that maintenance crosses; low/high = imports cross but changes stay local | have (`seval cochange-layout`) |
| 3 | **Symbol working-set reachability tail** — per function, distinct definitions transitively reachable in the resolved call/type graph, p90 over (n−1), traversed via SCC condensation; never shown without the cognitive tail (the joint surface is G1) | the direct operationalization of reader working set; helper-soup raises it while lowering local complexity — the pair polices itself | have (`seval symbols` — heuristic resolution, ledger honest) |
| 4 | **Balanced separator witness** — smallest found vertex set whose removal leaves components ≤ 2n/3; articulation points exactly first, then spectral sweep (normalized Laplacian) as internal machinery; report the *node list* and component sizes as an upper-bound witness, never "the treewidth", λ₂ as solver provenance only | recursive decomposability = the structural form of deep modules; a 1-node separator that is a god-object is caught by reading its G2 interface-information beside it | later (experimental) |
| 5 | **Feedback-arc fraction** (min FAS via deterministic Eades–Lin–Smyth / |E|) and **improved trophic incoherence F0** (exact rational solve, MacKay–Johnson–Asllani 2020) — the edge-deletion and layering complements of F↔; trophic coherence has never been tried on software (finding, not citation) | how much wiring must break to layer the system; how far the directed graph is from clean stratification (the tangle SCCs cannot see) | trophic half lands now (`seval deps` exact F0); FAS half stays later |
| 6 | **Size-stratified AST DAG ratio** — hash-cons every subtree (structural equality confirmed on hash match), report |DAG|/|tree| as a *curve* over minimum subtree size k, alpha-normalized identifiers primary, generated files excluded | global structural-repetition superset of clone windows; the raw scalar is dominated by trivial leaves and by grammar verbosity — the curve is the honest object. One seat would ship it, one would gate it as a corpus experiment; adjudicated: research arm, never a headline, never called Kolmogorov anything. **Second round: downgraded to repetition *locator*** — the ratio has no stable quality direction (200 deliberately uniform protocol encoders and 200 copy-pasted handlers produce the same curve; alpha-normalization erases exactly the names that distinguish them; folding repeats into an opaque macro improves the number while raising reader state). Where the repeats are stays reportable; the global compressibility scalar is not a determinant | locator only (2nd round) |
| 7 | **Condensation depth profile** — per-file longest-path depth d_in(v), d_out(v) on the SCC-condensation DAG, per weak component; distributions + max witness path; exact rational d/(c_W−1) beside the raw integers | the sequential-abstraction-boundary count reachability cannot see: a deep thin chain, a shallow wide fan, and a high-fan-in stable kernel have different depth shapes at equal reach; O(n+m) DP, witness chain instead of a scalar. Path *counts* rejected: exponential, shortcut-sensitive, no stable denominator | have (`seval deps` condensation profile) |
| 8 | **Conductance and SCC spectral certificates** — per connected undirected component, a dyadic lower bound λ₂ ≥ a/2^b from fixed-order rational inertia bisection on L − qD, reported only as its Cheeger consequence φ(C) ≥ a/2^{b+1}; per nontrivial directed SCC, exact Collatz–Wielandt rational bounds on adjacency ρ | conductance supplies the one negative fact no other survivor establishes: no cut exists below its bound. SCC ρ supplies the recirculation growth rate across closed-walk lengths: ρ=1 is a thin simple cycle; larger ρ is braided/dense recirculation. Both are exact bounded certificates, not floating eigensolver headlines and not quality verdicts | have (`seval deps` conductance and spectral certificates) |
| 9 | **Boundary-endpoint dispersion** — per directory and relation: fraction of member files that are cross-boundary endpoints (in and out separately), plus the minimum endpoint set covering 90% of crossing edges, with witness list | interface concentration Q cannot see: identical Q and crossing mass for a module routing everything through two façade symbols vs one exposing every internal file; the structural form of G2 interface sharpness. Goodhart: a god façade — read its interface information beside it | have (`seval deps` layout profile) |
| 10 | **Boundary direction inconsistency** — per unordered directory pair, D = Σ min(e_ab, e_ba) / Σ (e_ab + e_ba) with per-pair rows and edge witnesses, per relation | quotient-level two-way coupling on an acyclic file graph: two directories depending on each other through disjoint files fire no SCC, no FAS, no F↔; this is Parnas one-way-ness at the partition level. Legitimate exception: bidirectional peer protocols — a coordinate, never a target | have (`seval deps` layout profile) |
| 11 | **Static–historical support cross-tab** — cross co-change mass binned by static relation of the pair (direct / transitive-only / unrelated), over total cross mass; and conversely, fraction of cross-directory static edges with any co-change support. Fixed-point mass, same pinned universe both sides | the question neither Q answers: are the pairs that change together the pairs the declared architecture connects? Two repos with identical Q×Q coordinates can have opposite support tables. **Precondition it repairs: the 2-D Q read is invalid until both graphs share one pinned node universe** (static Q sees parser-supported walk files, co-change sees all source-classified tracked blobs; the instrument intersects them and reports all four join counts) | have (`seval cochange-support`) |
| 12 | **Commit Jaccard tail** — pairwise Jaccard similarity of file change-histories, distribution + tail, exact rationals | partition-free coupling detector: files that *only* change together, invisible to directory-binned Q, robust to layout churn | have (`seval cochange-support`) |

Determinism doctrine for this family: integer and rational invariants are
preferred not merely for float-safety but because they are self-interpreting —
an eigenvalue means nothing until compared to a Monte-Carlo null, which
reintroduces the RNG the tool avoids. The entire surviving set needs zero
eigensolver and zero BLAS except the separator's internal sweep (dodgeable
with a combinatorial multilevel partitioner). Trophic F0 now uses bounded exact
rational elimination. Null models, when needed offline: degree-preserving configuration
rewiring (Maslov–Sneppen), never Erdős–Rényi — beating a homogeneous random
graph is flattery, not evidence. Exact rational forms exist for both Q's
(signed integer numerator over 4m²; fixed-point weights for co-change).

Rows 6--8 remain distinct instruments rather than a score. The directed
cross-read is the **loop-spectrum triangle**: exact trophic $F_0$
(linear-algebraic layering), exact files-in-cycle counts over their analyzed-file
denominator (combinatorial membership), and exact SCC bounds on $\rho$
(closed-walk recirculation growth). Johnson and Jones (2017,
10.1073/pnas.1613786114) show coherence controls leading eigenvalues; these
three instruments put the corresponding vertices on the same resolved graph.
$\rho=1$ identifies thin cycles--a single simple cycle is exactly 1--while
larger $\rho$ identifies braided or dense recirculation. Display $\ln(\rho)$
would be the closed-walk entropy rate, but only the rational bounds are
authoritative. None is a quality verdict: pipelines may legitimately be
$\rho$-less and event meshes legitimately $\rho$-heavy.

### Second adversarial round (2026-08-15, Sol xhigh via codexpool + Gemini)

The browser Oracle seat being down, the adjudicated set above was attacked by
two repo-grounded seats. Verdict changes are folded into the table (rows 6–12);
what follows is the residue that is neither a row nor already fixed.

**Fixed same-day**: F↔'s global n(n−1) denominator dilutes a tangle embedded
beside unrelated files — a 100-file SCC vanishes in a monorepo. `seval deps`
now also reports mutual reachability inside the worst weak component
(exact integer argmax; own repo: 42/702 = 0.060 global vs 42/506 = 0.083
scoped).

**Implementation debts, admitted as defects (both seats hit the first)**:
static Q and co-change Q are computed from exactly-accumulated integers but
serialized only as f64 — the integer numerators/denominators (signed Q
numerator over 4m²; raw u128 masses and strengths) should ride beside the
float so the exact rational is reconstructible; F↔'s numerator accumulates in
usize, which can overflow on a 32-bit target while succeeding on 64-bit — the
serialized form should be u128 numerator/denominator with the float display
secondary. The current f64s are deterministic on one architecture (fixed
iteration order, no FMA) but the doctrine claims more than that.

**Trophic F, sharpened prediction (both seats)**: it will correlate with
SCC/FAS operands inside cycles and with depth/shortcut operands on DAGs, and
fail the incremental-validity partial. Its one unique shape is the transitive
triangle (A→B→C plus A→C): F↔ = FAS = 0, depth and reach unchanged, F > 0.
If that is all that survives validation, replace the F0 scalar with exact
shortcut-edge witness rows (which is also Gemini's transitive-reduction edge
ratio — same object, |E_reduced|/|E| on the condensation DAG).

**Standing quadrant caveats for the 2-D Q read** (beyond the universe
precondition in row 11): high-static/low-co-change is produced innocently by
resolver blind spots (path aliases read as external) plus disciplined
vertical-slice commits; low-static/high-co-change is produced by successful
dependency inversion (volatile plugins importing a stable kernel). The
coordinate locates; the diagnosis needs the support cross-tab and a human.

### Third wave (2026-08-15): the adjudicated set closes into instruments

Ten of the twelve survivor rows and all six type-space determinants (T3 landed
last) are now running instruments; only row 4 (task-grounded reader
experimentation) and row 5 (balanced separator witness) remain gated, both
deliberately, on the validation arm. The wave's own findings, in the spirit of
the instruments measuring their maker:

- The conductance certificate's 3-path component certifies λ₂ ≥ 1024/2^10 —
  exactly P3's true normalized eigenvalue, a built-in ground-truth check that
  the bisection and exact inertia agree with textbook spectra.
- Louvain headroom 1896/12544 (~0.15) with directory-pure detected
  communities: `src/` hides coupling subcommunities; the folder tree is not
  wrong, it is coarse.
- T3's generic-washing deflator fired on its own repository: 7 of 8 generic
  parameters are `Into`/`AsRef`-bounded — nominally generic, morally concrete.
- A broken fixture manifest had been silently able to abort `seval deps`
  entirely (zero output on this very repo); unreadable manifests are now named
  skips with reasons, the denominator stays honest, and the contract test pins
  the new behavior.

The standing empirical gate is unchanged: the validation arm (refactor-delta
mining — do the determinants move the way theory predicts across accepted
refactorings?) is what separates a coordinate system from a collection of
numbers. Nothing above claims validity; it claims exactness, closure, and
disclosed limitations.

## Validation arm — literature priors and the adversarial battery

The target is high-dimensional: "healthier repo" is a bundle of outcomes
(defect density, maintenance effort, contributor survival, abandonment) that
no single coordinate captures. The literature was consulted 2026-08-15 through
scry's `academic.catalog` (seven bounded lexical probes over titles, abstracts
hydrated for the shortlist; probe ledger in the session record). The priors,
each carrying its adversarial edge:

| Prior | Source | What it does to our set |
|---|---|---|
| **Size confounds nearly everything.** Most structural metrics' fault associations shrink drastically once size is controlled | El Emam et al. 2001 (10.1109/32.935855); Zhou et al. 2014 (10.1145/2556777) | Battery test B1: every corpus association is computed size-partialed. A determinant that is SLOC in disguise gets demoted to a size proxy, whatever its mathematics |
| **History beats static structure; relative beats absolute.** Relative churn predicts defect density where absolute churn fails | Nagappan & Ball 2005 (10.1145/1062455.1062514); Hall et al. SLR 2011 (10.1109/tse.2011.103) | Validates the co-change arm (rows 2, 11, 12) and the denominator doctrine itself — every seval number is already relative by construction |
| **Change coupling correlates with defects** | Gall et al. 1998 (10.1109/icsm.1998.738508); D'Ambros et al. 2009 (10.1109/wcre.2009.19) | Rows 11–12 have direct published lineage; the Jaccard tail is the partition-free version of what these papers measured |
| **Propagation cost discriminates architectures and moves under deliberate redesign** (Mozilla's purposeful modularization lowered it) | MacCormack et al. 2006 (10.1287/mnsc.1060.0552) | Row 1's ancestor, and a published refactor-delta observation: the coordinate moved when the architecture was deliberately fixed |
| **The static×history cross is where the signal is.** Unstable Interface and Implicit Cross-module Dependency (structure + history combined) pinpoint error-prone files better than either alone | Xiao, Cai et al. 2015 (10.1109/wicsa.2015.12) | Our boundary rows (9–10) × support cross-tab (11) is exactly this cross; the unrelated-mass bin ≈ implicit cross-module dependency |
| **Decoupling Level: option-value of independently replaceable modules, measured against maintainability across 129 projects** | Mo, Cai et al. 2016 (10.1145/2884781.2884825) | Adjacent coordinate to our separator/conductance pair — note the sign discipline: a high φ certificate means a component RESISTS decomposition, which is cohesion within a module and monolithicity at the top; per-component framing is mandatory |
| **Code smells failed their controlled test.** With professionals on real tasks, file size and churn explained maintenance effort; smells added little | Sjøberg et al. 2013 (10.1109/tse.2012.89) | Humility for the discipline/shape arms: they are locators and coordinates, never health verdicts; expect B1 to hit them hardest |
| **Simple + process metrics win; complex models overfit** | Hall et al. 2011, ibid. | Empirical backing for the no-composite-score doctrine |
| **Trophic coherence ties loops, hierarchy, and spectra into one object**: incoherence determines cycle structure and leading eigenvalues; maximally coherent networks are loopless and stable | Johnson et al. 2014 (10.1073/pnas.1409077111); Johnson & Jones 2017 (10.1073/pnas.1613786114) | Rows 5 (trophic half), 7 (depth profile), and 8 (spectral certificate) are one mathematical family; the research arm should test Johnson's loop-spectrum relation on dependency graphs |
| **Abandonment/survival is measurable and predictable** | Coelho & Valente 2017 (10.1145/3106237.3106246); Samoladas et al. 2010 (10.1016/j.infsof.2010.05.001); Avelino et al. truck factor 2016 (10.1109/icpc.2016.7503718) | Outcome side of the corpus: abandonment class labels, activity survival, contributor concentration |

### The adversarial battery

B1 — **size partialing.** Across the corpus, every coordinate↔outcome
association is computed with SLOC partialed out (rank-based). Survivors are
determinants; casualties are size proxies and say so in their docs.

B2 — **transformation algebra (metamorphic laws).** Meaning-preserving
transformations with predicted responses, executed directly (2026-08-15, own
repo, /tmp copies):

- *Rename-with-rewrite invariance:* renaming `src/shape.rs → fnshape.rs` with
  reference rewrite left every `seval deps` number exactly identical
  (fully-normalized outputs byte-equal). Verified.
- *Naive rename is not a refactoring — and the instrument knows:* relocating
  `src` to `core` without reference rewrite collapsed resolution (62→37
  internal edges, depth 4→1, unresolved 7→33). Directory names are the import
  namespace; `deps` correctly detected a semantics-breaking change while
  `symbols` and `typespace` (declaration-keyed) stayed exactly invariant and
  `shape` was invariant modulo row order. Verified.
- *Disjoint-union laws:* duplicating the crate as two sibling trees —
  worst-WCC F↔ invariant (42/756 over 28 files, both sides); files-in-cycle
  fraction intensive (0.219 = 0.219); global mutual reachability diluted by
  exactly the documented denominator (42/992 → 84/4032); conductance
  certificates duplicated per component with identical (a, b). Verified.
  This classifies the set: per-component and worst-WCC coordinates are
  **intensive** (comparable across repo sizes); whole-graph fractions with
  n(n−1) denominators are **dilutive** (comparable only at fixed universe) —
  the second round's WCC-scoping fix is what made row 1 intensive.

B3 — **corpus discrimination.** Two labeled classes of Rust crates
(14 deprecated/unmaintained × 16 active, popularity ranges overlapping so
adoption cannot be the separator), full instrument suite + git outcome proxies
(activity, fix-share, contributor structure), run 2026-08-15 under
`~/projects/scratch/seval-corpus-2026-08-15/`. Cross-sectional association
only — HEAD structure post-dates the outcome for abandoned repos, and the
read is stated as association, never cause.

### B3 read-out (2026-08-15, n=30: 14 abandoned × 16 active)

Statistics: Mann–Whitney rank-biserial (rb) per coordinate vs class, Spearman
ρ, and Spearman partial ρ controlling SLOC rank (battery B1). All exact ranks
over the table at `~/projects/scratch/seval-corpus-2026-08-15/table.jsonl`
(30 rows, 240/240 instrument cells). Everything below is cross-sectional
association at HEAD, never cause; abandoned HEADs are frozen at abandonment.

**Finding 0 — the corpus design is size-confounded, and the battery caught
it.** SLOC separates the classes at rb = −0.911 (functions −0.893): in this
famous-crates sample, abandoned Rust crates are small single-purpose
utilities (atty, tempdir, quick-error — finished, absorbed into std, or
superseded) while the active class is large frameworks (tokio, diesel,
clap). Popularity ranges overlapped as designed; sizes did not. Consequence:
raw class associations are uninformative, and B1 partialing has little
residual class variance to work with — survivors and casualties alike are
weak evidence. This is itself a result: in the Rust ecosystem, abandonment
of well-known crates is dominated by scope (small crates complete or get
superseded), not by structural decay. A follow-up corpus must be
size-stratified.

*Population base rates (probed 2026-08-22).* grep.codemod.com runs ast-grep
rules over one HEAD snapshot of every GitHub repo above ≈25k stars (~1,600
repos; 16 shards; ~300 ms/query; unauthenticated `POST
/api/v1/search/stream`, ndjson, commit-pinned blob URL per match). Any
determinant phrasable as an ast-grep rule can therefore be given a base
rate over the most-adopted population, with PROVENANCE-grade evidence, in
one request. It is survivors-only, so it serves as a base-rate instrument
for the corpus-model rows above, never as an outcome-discrimination corpus;
the B3 follow-up still needs its own size-stratified sample. Probe notes
and request helper: `~/projects/scratch/grep-codemod-2026-08-22/`.

**Finding 1 — El Emam's prior confirmed wholesale.** Fourteen coordinates
showed raw class associations |ρ| ≥ 0.3 (deeper condensations, more cycles,
higher modularity Q, larger working sets, lower φ certificates in active
repos — "more of everything structural"); every one of them collapses below
|ρ|=0.3 after SLOC partialing except `cochange_layout_q` (−0.65 raw →
−0.32 partialed). The demotion list (detected-Louvain Q, transitive-only
mass, φ lower bound, parent-directory Q, working-set p90, layout headroom,
depth p90/max, files-in-cycle, worst-WCC F↔, mutual-reach, cognitive gap,
shallow-corner, direction inconsistency) is recorded so no future reading of
this corpus can mistake them for class discriminators.

**Finding 2 — the fix-share axis is the clean one.** `fix_share` (share of
window commits whose subject matches fix/bug) is nearly size-free
(ρ_SLOC = +0.19) and class-free (medians 0.140 abandoned vs 0.152 active),
so it dodges both confounds. On it:

- `symbol_working_set_p90` — ρ = +0.40, SLOC-partialed +0.38, and positive
  **within both classes** (+0.25 abandoned, +0.48 active). Files that force
  large reader working sets co-occur with fix-heavy histories. The single
  best nomination this corpus produces, and prior-consistent (comprehension
  load ↔ defect proxies; Hall 2011, Sjøberg 2013 both point at
  size/effort-of-understanding over smells).
- `t5_clone_density` — ρ = +0.40, partialed +0.37, but sign-unstable within
  class (+0.59 abandoned, +0.01 active): the association lives entirely in
  the abandoned half. Weak nomination.
- `cochange_layout_q` — the sole B1 class survivor also trends positive vs
  fix_share within both classes (+0.37, +0.17). Its two reads point opposite
  directions (lower Q in abandoned repos; higher Q with higher fix-share),
  which is what a coordinate, not a verdict, looks like. History-based
  lineage (Gall, D'Ambros, Nagappan) says keep watching it.

**Finding 3 — no-signal results are results.** Discipline (pure fraction,
unwrap/panic per function), typespace T1/T2/T6, nesting p90, and symbol
resolution showed no class or fix-share association beyond noise at n=30.
Sjøberg's smells-fail-controlled-tests prior predicted exactly this for the
discipline/shape arms. `revert_share` has tiny numerators and
anti-correlates with size (ρ = −0.44); no claims from it.

**Finding 4 — the within-stratum read (added later the same day).** The
corpus already contains one honest size-matched band: 6,000–28,000 SLOC
holds 6 abandoned × 5 active repos. Inside it, `cochange_layout_q`
separates the classes **perfectly** (rank-biserial −1.000; every abandoned
repo below every active one, threshold ≈ 0.17; exact null probability
1/462 ≈ 0.002), while eligible-commit counts (rb = +0.27) and file counts
overlap freely — it is not a history-volume proxy. `support_jaccard_p90`
shows the consistent companion sign (+0.67, abandoned repos carry heavier
co-change tail coupling). Both are history-based coordinates; every static
coordinate stays within noise in the band. This is the corpus's central
empirical statement, and it is the Nagappan/Gall prior speaking: whether
recent commits cohere within the directory layout or scatter across it
tracks project fate where structure alone does not. n = 11, association
at HEAD, never cause.

Everything above is **nomination, not validation** — B4 (refactor-delta)
remains the gate. What B3 changes: the next corpus must stratify by size,
and `symbol_working_set_p90` earns priority in the B4 mining.

### B3 second pass — the self-edge defect and the Johnson cross-read (2026-08-15)

Adversarial use of the corpus caught a second instrument defect: 23 of 30
repos carried phantom self-loop edges — a bare crate-name `use` next to a
same-named file (`examples/atty.rs` containing `use atty`) and `mod.rs`
files referenced through their own `crate::` path both resolved to the
declaring file. Self-import is impossible in every covered language's
semantics; the loops forced F₀ = 1/1 on atty and inflated files-in-cycle
across the corpus. Fixed at the resolution layer (a self-target resolution
is discarded and the declaration falls through to external/unresolved);
target scoping (examples/tests as separate crates) remains a disclosed
limitation. All deps-derived numbers below and in the read-out above were
recomputed post-fix; every B1 demotion stands (files-in-cycle moved
−0.42 → −0.63 raw and stays a size casualty at −0.28 partialed).

**The Johnson cross-read — the loop-spectrum triangle.** Over the
24 corpus repos with fully computed F₀ (three size_limit, three
trivial/no-edge):

- **F₀ vs files-in-cycle: ρ = +0.919, and +0.881 with SLOC partialed.**
  Trophic incoherence and cycle mass are computed by disjoint machinery —
  an exact rational linear solve versus Tarjan SCC membership — and Johnson
  et al.'s looplessness↔coherence prediction binds them tightly on real
  dependency graphs, independent of size. This is the strongest
  cross-instrument consistency evidence the suite has.
- F₀ vs φ lower bound: ρ = −0.788 (−0.674 partialed) — recirculating
  components are exactly the ones whose spectral certificate is weak.
  Consistent with the loop-spectrum half of the prior, though φ's implicit
  size scaling makes this the softer read.
- F₀ vs depth p90: ρ = +0.425 — mild, as expected: condensation depth
  measures the DAG skeleton, F₀ measures deviation from it.
- F₀ vs class: rb = −0.622 collapsing to −0.23 under SLOC — F₀ is *not*
  a health verdict on this corpus, and per doctrine never claimed to be.

The family reading: F₀, files-in-cycle, and SCC spectral radius $\rho$
triangulate one latent property — how far the file graph is from a layered
feed-forward architecture — from three independent formalisms. The prior
conductance cross-read remains historical evidence about cohesion, not the
triangle's spectral vertex; the exact SCC $\rho$ certificate now measures that
vertex directly on the same graph. Extremes on the corpus: five repos are exactly F₀ = 0/1
(perfectly layered; four abandoned small utilities plus url), stdweb is
F₀ = 0.822 with 68.5% of files in cycles.

B4 — **the standing gate:** refactor-delta mining. Do coordinates move the
predicted direction across accepted refactorings (MacCormack's Mozilla
observation, systematized)? First execution below closed the responsiveness
half; the directional half remains the gate on any validity claim.

### B4 first execution — responsiveness closed, direction open (2026-08-15)

Protocol: from the corpus histories, up to five refactor-intent commits per
repo (subject matching refactor/restructure/reorganize/modularize/decouple/
extract, touching ≥5 .rs files, largest touch first), each paired with a
churn-matched control commit (nearest .rs-touch count, subject not matching).
67 pairs across 18 repos; every commit measured against its parent with
`seval deps` in detached worktrees; deltas per coordinate. Runner and raw
measurements: `~/projects/scratch/seval-corpus-2026-08-15/b4/`. Two runs
produced byte-identical F₀/FIC deltas — determinism held on ~540 historical
measurements.

**Responsiveness — the half of the gate that closed.** Refactor-intent
commits move the coordinates; matched ordinary churn does not:

| coordinate | refactor moved | control moved | Fisher one-sided p |
|---|---|---|---|
| trophic F₀ | 32/60 | 13/55 | 0.0010 |
| parent-directory Q | 31/67 | 17/67 | 0.0094 |
| internal edges | 32/67 | 18/67 | 0.0099 |
| files-in-cycle | 29/67 | 19/67 | 0.0522 |
| depth p90 | 3/67 | 1/67 | 0.31 |

The instruments detect deliberate structural change and stay quiet under
same-sized ordinary churn — sensitivity and specificity at once. Depth p90
is the exception for a stated reason: an integer percentile over the
condensation barely moves per commit; it is a repo-scale coordinate, not a
commit-scale one.

**Direction — the half that stays open.** Among refactor movers, F₀ went
down 18 / up 14 (sign p = 0.60) and files-in-cycle down 13 / up 16:
keyword-mined refactorings reshape structure without uniformly layering it.
Two directional leans worth recording: parent-directory Q rose in 20 of 31
moving refactors (sign p = 0.15, suggestive — restructuring tends to align
layout with structure), and internal edges rose in 24 of 32 (p = 0.007,
read as a split artifact: extracting modules multiplies explicit imports,
not a health direction). MacCormack's Mozilla observation was a *deliberate
modularization*; the generic "refactor" keyword mixes extractions, renames,
API reshuffles, and cleanups with opposing predicted signs.

Limitations, stated: commit subjects are a weak intent proxy; pairs cluster
within 18 repos (not independent); candidates and controls come from each
repo's most recent 200/400 non-merge commits; n = 67. What the directional
gate now requires is intent-classed subsets — decouple/break-cycle commits
predicting F₀ and cycle mass down, extract-module commits predicting Q up —
each class carrying its own sign before measurement.

**Directional pre-registration (2026-08-15, before measurement).** Two
intent classes mined from the corpus histories by tightened subject
patterns (excluding `extractor`/`flatten` API vocabulary): *decycle* —
break/remove/fix cycle-or-circular, decouple, untangle (15 commits);
*extract-module* — move/extract/split targeting a module/submodule/crate/
separate-file noun (243 commits). Predictions, fixed before any
measurement: among decycle commits that move trophic F₀, down exceeds up;
among decycle commits that move files-in-cycle, down exceeds up; among
extract-module commits that move parent-directory Q, up exceeds down.
One-sided exact binomial sign tests on movers; results in the commit that
follows this one.

### B3 extension — the size-stratified corpus (2026-08-15 evening, n=44)

Finding 0's prescription executed: 14 repos added to break the size↔class
confound — six large clearly-discontinued projects (amethyst, xi-editor,
conrod, leaf, exonum, druid — official discontinuation notices or archived
repos) and eight small active crates (cfg-if, bitflags, itoa, ryu,
thiserror, anyhow, heck, is-terminal). way-cooler was selected and dropped
with disclosure: its HEAD is the project's rewrite-in-C tombstone (one .rs
file). druid is kept with disclosure: last commit 114 days old
(post-discontinuation maintenance drift). All deps columns re-measured
under the self-edge fix so the table is version-consistent
(`table.jsonl`, 44 rows; the n=30 original preserved as `table-v1.jsonl`).

Size↔class collapsed from rb = −0.911 to **rb = −0.271 (p = 0.13)** — B1
partialing now has genuine residual variance to work with.

**Class survivors under SLOC partialing (18 coordinates tested — at
Bonferroni 0.05/18 only the strongest survives; exact values, no crowns):**

- `t5_clone_density` — partial ρ = **+0.50** (raw +0.26; partialing
  *strengthens* it — size was masking, not making, the signal). Duplicated
  type-shape mass associates with abandonment at matched size, and it is
  also the strongest fix-share coordinate (+0.43 partialed).
- `symbol_mutual_reach` — partial ρ = +0.37 (raw rb +0.42, the only raw
  class association at p < 0.05): entangled symbol graphs associate with
  abandonment. On the n=30 corpus this died under partialing; with the
  confound broken it survives.
- `symbol_working_set_p90` — partial ρ = +0.39 (raw +0.17; another
  suppression case), consistent with its n=30 fix-share nomination.

The three survivors are one conceptual family — duplication and
comprehension load — and all three also carry the fix-share axis
(+0.43/+0.27/+0.27 partialed). The discipline/shape/typespace-T1/T2/T6
arms stay within noise, as Sjøberg predicted.

**The attenuation that teaches: `cochange_layout_q` weakened** (raw −0.30,
partial −0.12) on the extended corpus, despite its perfect within-stratum
separation on n=30. The new large abandoned repos explain it: druid
(q = 0.286) and amethyst (0.243) sit in the *active* range — well-organized
projects abandoned by organizational decision, not maintenance decay. The
class label bundles two mechanisms: utility crates that complete or get
superseded (where co-change coherence separated perfectly) and
organization-backed projects that die by announcement while still
well-maintained. Co-change coherence tracks the *decay* mechanism, not the
*decision* mechanism — a coordinate meeting a heterogeneous outcome, which
is exactly why the doctrine forbids composite health verdicts. Follow-up
when the corpus grows: mechanism-labeled outcome classes
(decayed / superseded / discontinued).

### B4 directional results — the pre-registered tests (same evening)

Measured after commit `00dddea` fixed the predictions; 258 commits, all
measured (b4/measurements3.jsonl):

- **extract-module → parent-directory Q up: CONFIRMED.** Among 243
  extract/move/split-module commits, 152 moved Q; 95 rose vs 57 fell —
  one-sided exact binomial **p = 0.0013**. First directional prediction to
  pass measurement: extracting modules moves layout-modularity the
  predicted way across 18 real repos' accepted history.
- **decycle: below instrument granularity, not refuted.** 13 of 15
  decouple/break-cycle commits moved neither F₀ nor files-in-cycle (2 F₀
  movers split 1/1). Inspection of the subjects shows why: they decouple
  types and derive logic *within* files — symbol-level surgery invisible
  to the file-edge graph. The prediction stands untested at file
  granularity; the HIR bridge is the instrument that could test it.

Validity ledger after one full battery cycle: exactness (B2) verified,
responsiveness (B4a) verified, one directional law (B4b extract→Q)
verified, class associations (B3) nominate the duplication/comprehension
family under size control. Everything remains association or
history-internal movement — never cause, never a verdict.

### The exact triangle closes on the corpus (2026-08-15, late)

With the spectral-radius certificate landed (Collatz–Wielandt bounds on
ρ(A) per SCC, exact over BigRational), all three vertices of the Johnson
loop-spectrum triangle are exact on the same file graph. Over the 24
corpus repos with a certified ρ and computed F₀ (n=44 corpus, deps
re-run at HEAD e33829d):

| pair | Spearman ρ | SLOC-partialed |
|---|---|---|
| ρ(A) vs trophic F₀ | +0.714 | +0.680 |
| ρ(A) vs files-in-cycle | +0.669 | +0.688 |
| trophic F₀ vs files-in-cycle | +0.846 | +0.840 |

Three formalisms — a rational linear solve, Tarjan membership, and a
Perron–Frobenius growth rate — computed by disjoint code paths, agree
pairwise on real dependency graphs, and the agreement survives size
control. This is Johnson & Jones's prediction (coherence controls the
leading eigenvalue) observed exactly, not estimated. ρ tracks largest-SCC
size at +0.907, so it is a *braidedness* coordinate for the recirculating
core, not a whole-repo one: failure and iron sit at ρ = 1 exactly (a
single 2-cycle), stdweb at 6.71 (65-file braided core), nom at 5.19,
conrod 4.90, actix-web 4.64. Every SCC on the corpus certified within the
128-file bound; none hit size_limit.

**Coeffect grading, first read (n=39 with internal edges).** Plug-edge
fraction, edge-grade p90, and internal glob fraction show no class or
fix-share association beyond noise — the honest null: coupling *width* at
file granularity does not separate these classes. One lead: external
glob-bearing fraction (ambient authority toward dependencies, `use
rand::*`) at class rb = +0.26 → **+0.38 SLOC-partialed** and fix-share
+0.34 raw — the same suppression pattern as the duplication family, and
prior-consistent (unreviewable dependence ↔ maintenance burden). n=43,
nomination only. Own repo for scale: 40/64 plug edges, 1/315 external
glob.

## The effect–coeffect duality — a frame for the whole instrument set

Read 2026-08-15 evening from Shi, Zhang & Cui, *A Programming Paradigm for
Spatiotemporal Composability* (2026, github.com/cordiverse/paper), against
the graded-coeffect line (Petricek, Orchard & Mycroft ICALP 2013 /
ICFP 2014; Gaboardi et al. ICFP 2016; Brachthäuser et al.'s Effekt
capabilities; Krishnaswami's capabilities↔separation-logic bridge, TYPES
list). The duality: **effects** describe how code *modifies* its
environment; **coeffects** describe what code *requires from* it. The
paper lifts both to runtime mechanisms (revertible effects, reactive
coeffects) for dynamic composition; statically, the same duality
organizes our instrument set:

- **Effects side — `discipline`.** Pure-fraction, unwrap/panic surfaces:
  how much of the code modifies or aborts its world, function by
  function, with denominators.
- **Coeffects side — import grading** (instrument landing on branch
  `coeffect-grading`). A file's import surface is a *statically declared
  graded coeffect* over the canonical semiring {0, 1, n, ∞}: unused
  import, single symbol, enumerated set, glob. `use foo::*` is precisely
  the ∞-grade — ambient authority, dependence that cannot be reviewed at
  the declaration site. Per-edge grades separate plug edges (grade ≤ 2,
  swap-friendly) from interface entanglement, and the glob-bearing edge
  fraction is a capability-discipline coordinate in the §6.3 sense of the
  paper: narrow declarations are reviewable capability requests.
- **Spatial composability — the graph arm.** The paper's §6.5 proves the
  useful folk theorem: every mutual dependency decomposes in principle
  into unidirectional cores plus integration components — so row 1's
  worst-WCC F↔ counts exactly the *unfactored* bidirectional couplings,
  and the decomposition's quadratic component-count cost is why they
  persist. Reactive-coeffect cycle detection (their runtime reporting of
  dependency cycles) is our rows 6–8 read at load time instead of HEAD.
- **Temporal composability — the removability reading.** A component is
  removable when its effects can be reverted and its dependents
  re-resolved. The static shadow: per-file transitive in-cone (already in
  `deps` propagation) is the blast radius of removal, and the co-change
  arm (rows 11–12) measures the interface-drift shadow — files that must
  move together when contracts shift (their §6.6 interface drift, seen
  from history).

What this frame adds beyond taxonomy: it says the discipline and grading
instruments are *duals* and should be read as a pair (a file may be pure
but ∞-graded, or effectful but plug-coupled — four quadrants, each an
honest architecture), and it grounds two coordinates (F↔, glob fraction)
in composability theory rather than style preference.

## PL-theoretic determinants — the type-space arm

Seat consult 2026-08-15 (Claude Opus, repo-grounded; full text was distilled
here, the distillation is canonical). The graph range measures coupling
*between* definitions; discipline measures local honesty *inside* one function.
Neither ranges over **type definitions and signatures** — where state-space
cardinality lives. Governing caveat: tree-sitter sees the *spelling* of types,
never their meaning; every row is a proxy over declared syntax until the HIR
bridge (below).

| # | Determinant | Class | Instrument (denominator) | Goodhart / lies hardest on | Dedupe |
|---|---|---|---|---|---|
| T1 | **Algebraic type shape** — sums where the domain branches | generative | data-bearing enums / all type defs; per-struct Option+bool field count, tail of structs with ≥2 (/ structs with ≥2 fields) | two-variant enums that are still bools; hoisting optionals into a sub-struct; builder/config crates (orthogonal optionals are legitimate — read beside T4's builder split) | discipline counts bool *params* only; this is type-definition-scoped |
| T2 | **Dynamic-state surface** — `HashMap<String,Value>`, `dyn Any`, `Record<string,any>`, `interface{}` in fields + public signatures | diagnostic | dynamic-state mentions / type-constructor leaves in those positions | `type Json = Value` aliasing blinds it entirely (resolution failure, not improvement); serialization libs/interpreters where `Value` *is* the domain | adds container forms discipline's bare-`any` count misses; report only the increment |
| T3 | **Signature parametricity degree** — theorems for free | generative | abstract type positions / signature type leaves; return-parametric fraction; trait-bounds per parameter as deflator | generic-washing (`T: Into<Concrete>`); async Rust bound soup; a domain CLI is legitimately near zero — within-language distribution only. Proxy even under rustc: Rust breaks parametricity (TypeId, downcast, specialization) | `seval typespace` T3 adds the abstract/concrete partition, return-parametric split, nearest-rank bounds distribution, and conversion-to-concrete generic-washing census; `seval api` supplies only coarse generics counts |
| T4 | **Endomorphic closure** — the API is an algebra | generative | Self-returning public methods / public methods per impl; owned-endo split from `&mut Self` builder-endo; `(T,T)→T` monoid census | setter-chain builders (separated); closure ≠ lawfulness — laws are property-testing territory, never static | none; new |
| T5 | **Ownership-evasion density** — the compiler-*evaded* mutation half of G3 | diagnostic | `RefCell/Cell/Mutex/RwLock` mentions + borrow/lock sites, `Rc/Arc` separately, `.clone()` / call expressions | hand-rolled `unsafe` interior mutability dodges it (discipline's unsafe count rises — cross-check); concurrent servers legitimately live here | discipline counts compiler-*checked* mutation only; this is the aliased half |
| T6 | **Newtype adoption vs primitive obsession** | generative | bare wide-primitive mentions / public signature type mentions; newtype supply; `pub`-field newtypes counted as costume | numeric/parser code where the primitive is the domain; distribution only | G5 named it *later*; distinct from T1 (leaf width, not composite shape) |
| T7 | **Import-surface coeffect grade** — declared context-dependence width | diagnostic | per-language glob and module-object declarations / use declarations; internal non-glob edge-grade distribution; glob-bearing internal and external edges / their edge denominators; plug edges (grade ≤2) / non-glob internal edges | a glob may be used narrowly; wildcard re-export/prelude idioms are legitimate and still flagged because ambient authority remains ambient; module objects are bounded by an unseen module surface rather than enumerated leaves | adds declaration-width/reviewability to `seval deps`; graph fan-out counts targets, not the width of authority granted at each target |

Implemented 2026-08-15 as `seval typespace` (T1–T6; every ratio an
integer numerator/denominator with a closure identity, all marked proxy over
declared type syntax). Own-repo read: data-bearing enums 21/302; Option+bool tail
51/222 (dominated by analyzer-report structs, where optional counters are the
honest shape); dynamic state 40/5081 mentions (mostly typed maps, not erased
values); T3 abstract positions 8/319, return-parametric 0/7 generic public
functions, bounds min/p50/p90/max 1/1/2/2, and generic-washing 7/8 parameters;
endomorphic methods 14/25 public; shared-mutable concentrated in the service
state; wide primitives 1064/1651 public positions with zero newtype supply — the
instrument measuring metric/report records, a legitimately primitive-heavy
domain.

Implemented 2026-08-16 as the `seval deps` **coeffect-grading instrument**.
The PL lineage is Petricek, Orchard, and Mycroft's unified analysis of
context-dependence (ICALP 2013, DOI 10.1007/978-3-642-39212-2_35) and calculus
of context-dependent computation (ICFP 2014, DOI 10.1145/2628136.2628160),
extended by Gaboardi et al.'s graded effect/coeffect account (ICFP 2016, DOI
10.1145/2951913.2951939). Shi, Zhang, and Cui, *A Programming Paradigm for
Spatiotemporal Composability* (2026, `cordiverse/paper`, §6.3), supply the
runtime-declaration interpretation: narrow statically declared capability sets
are reviewable; ambient authority defeats that review. This coordinate grades
the import declaration stream with finite named-leaf counts, glob/∞, and a
separate module-object class. It measures **declared** context-dependence width,
not runtime behavior. A glob can be exercised narrowly. Wildcard re-export and
prelude patterns are legitimate; flagging them is intentional because the
declaration still grants ambient, statically unenumerated authority.

Culled on the record: annotation-burden ratio (direction-free), totality census
(no Rust checker; the honest partiality signal is already discipline's
unwrap/panic/Result census), Curry–Howard (needs a prover), abstract-interpretation
invariant strength (needs typed CFG — MIR territory), effect gradation
(marginal over discipline).

**The HIR bridge collapses two queue items into one.** The resolved symbol
graph (queue #1) and the type-resolution tooling that de-proxies T1/T2/T5/T6
(alias defeat, inferred locals, transitive field cardinality) are the same
rust-analyzer/HIR investment: type edges and call edges fall out of one
bridge. Do not gate T3/T4 on it (T3 stays a proxy even with full types; T4 is
return-token-visible). Largest irreducible hole: local state that never
crosses a declared boundary is unseeable at any static level short of MIR
dataflow.

**Sharpened challenge resolution** (why the graph does not already capture
this): the graph and co-change see complexity *already paid* — types bound the
state space available to *tomorrow's* edit (a `HashMap<String,Value>` touched
by one function today is a landmine with low fan-in). The coupling shadow is
sign-free — high fan-in is equally a deep module or a stringly god-blob, while
sum-vs-product and newtype-vs-primitive carry sign. And parametricity/closure
have *no* graph shadow at all: `f<T>(Vec<T>)->Vec<T>` and
`f(Vec<Config>)->Vec<Config>` are graph-identical; the definition graph
quotients out exactly the quantifier structure T3/T4 measure. Admission stays
empirical: incremental validity after partialling out SLOC + cognitive +
fan-in, per the standing protocol.

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
6. **The graph's information ceiling is isomorphism** (Sol, 2nd round). Two
   implementations can share every extracted graph — file, call, type,
   co-change — while differing arbitrarily in algorithmic clarity, naming,
   invariant strength, error semantics, and fitness; conversely a façade move
   transforms every graph statistic while changing nothing a reader holds.
   The current set spans nearly every low-order fact the coarse graph
   contains (reach/depth/degree = exposure; SCC/F↔/FAS = feedback; the Q's =
   imposed partitions; separator/conductance = cuts); the marginal graph
   scalar after it is mostly a recombination of n, m, the degree sequence,
   and the reachability relation, and should be presumed near-zero
   incremental validity until proven otherwise. What dominates it:
   **task-grounded reader experimentation** (N real prediction/change tasks ×
   R blinded readers; correctness, files opened, revisits, time; tails over
   the explicit N×R denominator) for comprehensibility claims, and
   mutation/property/differential probes for Assurance claims — new semantic
   evidence rather than another functional of the same graph.
7. **Complexity migrates off the graph into the state space** (Gemini, 2nd
   round). A perfectly decomplected import graph passes everything while
   every function trafficks in `HashMap<String, Any>` or `&mut GlobalContext`
   — the type-space cardinality problem. Partially covered by the discipline
   instrument (any-annotations, global mutable state, unannotated params);
   the PL-theory range (types-as-invariants measurement) is the open arm.

### Fourth wave (2026-08-19): twin census

The Davis–Kahan objection that killed eigenvector methods — generated code
creates twin nodes that make eigenvectors non-unique — is itself the
instrument. `seval twins` reports the degeneracy directly as exact
combinatorics: open twin classes (identical resolved in/out neighborhoods,
an equivalence relation, so classes are canonical), closed twin classes
(identical after self-inclusion, which forces mutual edges — mutually linked
siblings), and near-twin pairs at a declared exact-rational Jaccard threshold
with tagged difference witnesses. A distinct-neighbor class floor (default 2)
absorbs the Rust `mod.rs` parent↔child confounder — a lone parent linked both
ways counts as one shared neighbor — and suppressed classes are counted so the
ledger closes. Twin structure establishes parallel declared shape only: it is
the sharpest cheap locator for template-stamped module families, and it cannot
distinguish them from legitimate plugin registries; that adjudication is the
reader's. Function-granularity twins over the resolved symbol graph remain
later.

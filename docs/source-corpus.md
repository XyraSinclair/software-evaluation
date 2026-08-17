# Shared source-corpus contract

`SourceCorpusSession` removes repeated source acquisition from a frontier
profile without changing any analyzer observation. It materializes one bounded,
immutable snapshot of the supported source subset, then lends cached source-tree
and tree-sitter views to the four frontier analyzers.

## Work contract

For one successful materialization:

```text
filesystem discoveries = 1
file reads              = supported source files
parses                  = supported source files
```

Shape, symbols, discipline, and clone analysis may still traverse their own
logical views and clone compatibility buffers, but they do not perform another
filesystem discovery, file read, or parse while inside the session.

The public `SourceFile.bytes: Vec<u8>` contract is intentionally preserved in
this PR. Consequently a compatibility `SourceTree` clone copies owned byte
buffers. Eliminating those copies requires an explicit versioned typed-corpus
API rather than silently changing mature analyzer internals to `Arc<[u8]>`.

## Thread-scoped selection

A corpus is selected through `SourceCorpusSession::scope`. Selection is stored
on a thread-local stack:

- analyzer threads explicitly enter the intended corpus;
- nested scopes restore the outer corpus when the inner scope exits;
- unwind through panic removes the scoped corpus through a drop guard;
- simultaneous sessions for the same filesystem path cannot observe one
  another's bytes or parse trees.

This avoids a process-global path cache, whose identity would be insufficient
when two concurrent scans materialize different snapshots of the same path.

## Content evidence

`SourceCorpusEvidence` records:

- schema version;
- reported input root;
- manifest SHA-256;
- enumerated, supported, and skipped file counts;
- total supported-source bytes;
- syntax-error file count;
- exact discovery/read/parse work counts;
- one ordered row per supported file with path, language, byte length,
  content SHA-256, and syntax-error status.

The manifest preimage is deterministic and length-delimited:

```text
schema version
NUL separator
enumerated-file count
skipped-file count
for each supported file in lexical path order:
    path length, path bytes
    language-name length, language-name bytes
    byte length
    raw SHA-256 of file bytes
    syntax-error bit
```

The reported root is excluded from the manifest digest. Equivalent analyzed
source subsets at two locations therefore have the same corpus identity.

## Scope of identity

The corpus manifest identifies only the supported source subset observed by the
shared loader. It intentionally does not include unsupported file bodies.
Changes to documentation, configuration, generated assets, or other unsupported
files can leave the source-corpus digest unchanged.

The Git commit and tree digest remain the whole-artifact identity. The corpus
evidence is a subordinate evidence for the exact bytes and parses used by the
source analyzers, not a replacement for Git provenance.

## Output-equivalence ratchet

`tests/source_corpus.rs` runs all four analyzers twice over one polyglot fixture:

1. their ordinary uncached public APIs;
2. concurrently inside one shared corpus.

The serialized observations must be byte-for-byte identical. The same test
requires one read and one parse per supported file and verifies that every
analyzer consumes cached source-tree and parse views.

This is stronger than a benchmark. It makes computational parsimony an
observational refactor: acquisition work may fall, but the instrument semantics
cannot drift.

## Frontier fallback

A corpus-materialization failure does not suppress all four analyzers. The
frontier records an explicit limitation and falls back to their existing
independent paths. Analyzer-specific failure, missingness, and censoring remain
separate from substrate failure.

The frontier profile's total elapsed time includes corpus construction.
Individual analyzer evidence times begin after materialization and therefore
measure instrument work over the prepared corpus.

## Remaining residue

The next versioned change should:

1. add the corpus manifest digest and work evidence directly to the frontier
   schema;
2. require each source analyzer evidence to name that digest;
3. replace thread-scoped compatibility lookup with explicit typed
   `&SourceCorpus` analyzer entry points;
4. use shared immutable byte bodies without changing existing public report
   schemas;
5. materialize committed Git blobs directly, closing the remaining gap between
   pre/post worktree stability and immutable read isolation.

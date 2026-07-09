# Judge procedures — forward-cycle1-20260709

Artifacts: forward@ecb4c6e ("B", pre-dogfood) vs forward@d2e5f03 ("A", post-dogfood),
exported as detached worktrees /tmp/fwd-eval/{A,B}. Labels not disclosed to judges;
blind acknowledged compromised (A contains .forward/cycle.md describing its own fixes).
Mitigation: evidence-citation required per verdict + adversarial symmetry (worst-3
defects demanded for BOTH snapshots).

Judges (independence structure):
- judge-artifact-claude: Claude (fable), fresh subagent session. Artifact A/B comparison, 5 axes.
- codex-artifact-judge: Codex gpt-5.6-sol xhigh, separate model family/vendor. Same prompt.
- judge-process-claude: Claude (fable), fresh subagent session. Process-beauty audit of the
  dogfood cycle itself (5 process axes), adversarial framing ("self-run rituals are usually theater").
- self-read: the authoring session (relation: self-authored — labeled, cannot solely support
  contested axes).

Axis mapping to TAXONOMY.md: conceptual-integrity→parsimony(Form); internal-consistency→
consistency(Form); claim-correctness→documentation truth(Assurance); interface-sharpness→
interface sharpness(Form)+operational legibility(Life); honesty-infrastructure→fitness-to-intent
(Life; the artifact's stated intent IS honesty machinery). Waived for prose-as-program:
AST parsimony (no AST; conceptual parsimony judged instead), correctness machinery
(no code beyond install.sh; claim-audit covers it), robustness (waived: no runtime),
evolvability (too young for co-change mining — one day of history).

Prompts given verbatim to each judge are preserved in this directory's git history
and reproduced in the raw outputs.

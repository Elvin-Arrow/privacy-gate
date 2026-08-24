# Decision: Spec review — Claude + Gemini (no Ollama)

- **Status:** Accepted
- **Date:** 2026-08-23
- **Supersedes:** [0001](./0001-multi-model-spec-review.md) insofar as **which reviewers** are
  invoked. The rest of 0001 (shared review prompt, reconcile against idea/SRS, raw output in
  `docs/notes/reviews/`, no invented requirements) still applies.

## Context

Decision 0001 required three reviewers: Gemini via `agy`, gpt-oss via Ollama Cloud, and
qwen-3.5 via Ollama Cloud. Architecture-spec generation (dev-log 0003) showed Ollama Cloud
returning HTTP 429 (account session limit) and a local qwen substitute truncating the prompt.
The project owner directed that subsequent specs must not invoke Ollama models and must use
Claude and Gemini only.

## Decision

From this decision onward, each spec is reviewed by **two** independent models:

1. **Gemini** via `agy` (`--effort high`, `--print`, flags before the prompt).
2. **Claude** via the `claude` CLI (`-p` / `--print`, tools disabled or unused so the review
   cannot edit the tree).

Do **not** invoke Ollama (cloud or local) for spec review.

The shared review prompt still asks for alignment (spec→SRS, spec→design/architecture as
applicable, spec↔idea), quality/implementability, scope discipline, deferral health, and a
top-N change list. Reconcile both reviews against the idea doc and parent specs. Raw output
lives under `docs/notes/reviews/`.

## Rationale

- Two different labs (Google / Anthropic) still give a cross-check; dropping a third reviewer
  that cannot actually run is better than inventing a review or blocking the spec tree.
- Owner direction is explicit: no Ollama going forward.
- Keeping 0001's *method* (structured prompt, idea-doc as authority, notes vs specs) avoids
  throwing away the process that caught the SRS and design bugs.

## Alternatives Considered

### Keep invoking Ollama and skip when 429

Rejected: produces an incomplete review set that looks complete in the notes tree, and the
owner forbade Ollama.

### Single-model (author only)

Rejected: 0001's rationale still holds — one reviewer shares the author's blind spots.

### Three reviewers but replace Ollama with a second Claude or Gemini pass

Rejected: two passes of the same family are not independent. Claude + Gemini is the pair.

## Consequences

- Decision 0001 remains the method; this decision replaces the reviewer roster.
- Architecture spec notes that record the Ollama 429 stay as history; they are not retroactively
  "filled in."
- API, UI, and testing spec generation follow this roster.

## Related Documentation

- [Decision 0001](./0001-multi-model-spec-review.md)
- [Work item — API spec](../dev-log/0004-api-spec.md)
- [Work item — testing spec](../dev-log/0005-testing-spec.md)

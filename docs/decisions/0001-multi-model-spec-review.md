# Decision: Multi-model spec review for alignment and testability

- **Status:** Superseded in part (reviewer roster) by
  [0005](./0005-review-claude-gemini.md) on 2026-08-23. Method (structured prompt, reconcile
  against the idea doc, raw reviews in `docs/notes/`) remains in force.
- **Date:** 2026-08-23

## Context

Privacy Gate is starting its spec tree from `docs/idea.md`. The SRS is the first downstream spec
and everything later (design, API, architecture, testing, UI) builds on it, so alignment gaps and
untestable requirements here compound downstream. A single reviewer tends to share the author's
blind spots around the source doc.

## Decision

Review each spec with three independent models — Gemini (via `agy`), gpt-oss (via Ollama cloud),
and qwen-3.5 (via Ollama cloud) — using a shared review prompt that asks for four things:
SRS→idea alignment, idea→SRS clarifications, SRS quality/testability, and scope discipline (no
design/API/architecture/UI leakage). Reconcile the three reviews against the idea doc, applying
fixes where the SRS diverges and recording genuine idea-doc gaps as clarifications rather than
inventing requirements.

## Rationale

- Three reviewers with different training surface different misses (Gemini caught the
  "tighten under paranoid default" logical flaw and the export-sanitization gap; qwen-3.5 caught
  UI-leaky language; gpt-oss caught the undefined "source identity" and variant-lifecycle gap).
- Cross-checking against the idea doc keeps the SRS from drifting into scope creep (e.g.,
  rejecting the "manual redaction fallback" suggestion because it is not in the idea doc).
- Putting raw review output under `docs/notes/` keeps canonical specs clean while preserving the
  reasoning trail.

## Alternatives Considered

### Single-model review

Cheaper and faster, but shares one model's blind spots and misses the convergence signal that
makes a finding credible.

### Human-only review

Highest quality per reviewer, but not available on demand at this stage and does not leave a
machine-readable trail.

## Consequences

- Each spec generation now carries a three-model review step and a reconciliation step.
- Raw review output lives in `docs/notes/reviews/` (non-canonical) and is linked from the dev
  log, not duplicated into specs.
- Idea-doc gaps surface as numbered clarifications in the spec's §10, feeding back to the idea
  doc when product decisions are made.

## Related Documentation

- [Spec](../specs/srs.md)
- [Work Item](../dev-log/0001-srs-generation.md)
# [0001] SRS generation and three-model review

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce a requirements-only Software Requirements Specification from `docs/idea.md`, ensuring it
aligns with the idea doc (and vice versa) before downstream specs (design, API, architecture,
testing, UI) are written.

## Implementation

- Read `docs/idea.md` and `docs/user-story.md`.
- Drafted `docs/specs/srs.md`: functional requirements (FR-x.y), non-functional requirements
  (NFR-x), data requirements, constraints, acceptance criteria, scope, traceability table, and a
  clarifications section.
- Ran a three-model review (decision 0001): Gemini 3.7 Flash High via `agy`, gpt-oss:120b-cloud
  and qwen3.5:cloud via Ollama. Raw reviews saved under `docs/notes/reviews/`.
- Reconciled the three reviews against the idea doc and applied fixes.

## Problems Encountered

- `agy --print` flag ordering: flags after the positional prompt are consumed as the prompt.
  Fixed by placing `--model` and `--print-timeout` before `--print`.
- Reviewers flagged "tighten per-import" under the paranoid default as logically impossible —
  this is an imprecision in the idea doc itself, not just the SRS.
- Reviewers suggested adding a manual-redaction fallback; not present in the idea doc, so
  rejected as a v1 requirement and recorded as a clarification (Q10) instead.

## Resolution

- Reworded FR-1.4 to "cannot be loosened to retain" and recorded the idea-doc imprecision as Q9.
- Removed scope creep the SRS had added ("quarantine", undefined "source identity").
- Sharpened FR-5.1 (multi-doc PDF bundle), FR-5.4 (ephemeral lifetime), NFR-S4 (export
  sanitization), NFR-R1 (tamper-evidence), and reframed UI-leaky language as capabilities.
- Reframed FR-9.5/NFR-E1 as architectural constraints verified by the Architecture Spec, not v1
  acceptance tests.
- Added Q9-Q18 for idea-doc gaps the reviews surfaced.

## Follow-on: split clarifications per knowledge-governance skill

The SRS originally carried a §10 clarifications list (Q1–Q18). Per the skill, resolved
clarifications belong in decision records (the *why*) and open questions belong outside the
canonical spec. Split as follows:

- **Resolved** (Q8, Q9, Q10, Q11, Q4-partial) → `docs/decisions/0002-resolved-srs-clarifications.md`.
- **Open** (Q1–Q7, Q12–Q18) → `docs/notes/open-questions.md` (renumbered OQ-1..OQ-18).
- SRS §10 replaced with a See-also pointer; inline `see Q9` refs in FR-1.4 and AC-6 rewired to
  decision 0002; traceability table updated to reference `dec 0002 (Qx)` and `OQ-x`.

## Verification

- Final `docs/specs/srs.md` read in full after edits; traceability table updated to reference
  decision 0002 and the open-questions register.
- Three review files present under `docs/notes/reviews/` and linked from the notes index.
- Knowledge-governance skill applied: SRS in `docs/specs/`, decision records in
  `docs/decisions/`, open questions in `docs/notes/`, root + area indexes created.

## Lessons

- For `agy`, always put model/timeout flags before `--print`, never after the prompt.
- Three-model review convergence is a strong credibility signal; where all three flagged the
  same item (multi-doc bundle, NFR-R1 vagueness, "without rework" testability), the fix was
  clearly warranted.
- Reviewer suggestions can be scope creep; always check the idea doc before accepting.

## Related Documentation

- [Spec](../specs/srs.md)
- [Decision 0001 — review approach](../decisions/0001-multi-model-spec-review.md)
- [Decision 0002 — resolved clarifications](../decisions/0002-resolved-srs-clarifications.md)
- [Open questions](../notes/open-questions.md)
- [Raw reviews](../notes/reviews/)
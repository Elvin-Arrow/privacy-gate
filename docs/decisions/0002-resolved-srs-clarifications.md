# Decision: Resolved SRS clarifications (Q8, Q9, Q10, Q11, Q4-partial)

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

While writing `docs/specs/srs.md`, several gaps in `docs/idea.md` had to be resolved to keep the
SRS binding and testable. These were initially recorded as clarifications inside the SRS. Per the
knowledge-governance skill, resolved clarifications belong in decision records (the *why*), not
in specs (the *what*). This decision records the resolutions so the SRS can state the resulting
requirements without carrying the rationale inline. Unresolved questions remain in
`docs/notes/open-questions.md`.

## Decision

- **One canonical approved version per document (Q8).** Each document has exactly one canonical
  approved version. Named variants are override sets applied on top of the canonical version at
  share time, not separate approved versions. Reflected in FR-3.2 and FR-5.5.
- **Paranoid-default retention semantics (Q9).** Under a global "never retain originals"
  (paranoid) default, per-import overrides may not loosen the default to retain. The idea doc's
  phrase "per-import can only tighten" is imprecise because zero retention has no stricter state;
  the operative rule is "cannot be loosened." Reflected in FR-1.4 and AC-6.
- **Manual redaction fallback is out of v1 scope (Q10).** The idea doc describes detection plus
  per-field approval only; it does not provide for user-drawn redaction spans when the on-device
  model misses a field. A manual-redaction capability is therefore not a v1 requirement. It may
  be revisited in a later phase if the idea doc is amended. Design/UI specs must assume detection
  is the sole source of redaction candidates in v1.
- **Export redaction is true removal, not visual overlay (Q11).** The idea doc's trust thesis
  ("a stolen data file is useless"; "no private originals left the device") implies redacted
  content must be genuinely absent from exports, not merely visually hidden. Exported files must
  not contain redacted text or metadata recoverable from the file. The precise mechanism
  (text-stream stripping, re-rendering) is a design decision. Reflected in NFR-S4 and AC-2.
- **Multi-document export is a single PDF bundle (Q4-partial).** The user story specifies
  exporting multiple approved documents as a single PDF bundle. v1 export shall support
  selecting one or more approved documents and producing a single combined bundle. The remaining
  export-format questions (single-document format, ordering, naming, same-as-source vs. PDF) stay
  open in `docs/notes/open-questions.md`. Reflected in FR-5.1.

## Rationale

- Q8 and Q9 remove genuine ambiguity/imprecision that would have made the SRS untestable.
- Q10 is scope discipline: the idea doc is authoritative, and a capability it does not mention is
  not silently promoted into v1 requirements.
- Q11 follows directly from the idea doc's trust thesis; leaving it open would let a visual-only
  implementation satisfy the letter of "redacted" while violating the spirit.
- Q4-partial resolves only what the user story explicitly states; everything else about export
  format stays open rather than being guessed.

## Alternatives Considered

### Leave all clarifications inside the SRS

Rejected per the knowledge-governance skill: specs hold current truth, not rationale. Inline
clarifications also blur which questions are settled and which remain open.

### Promote Q10 to a v1 requirement (manual redaction)

Rejected because it would add scope not present in `docs/idea.md`. The idea doc is authoritative;
adding it would be scope creep of the kind the three-model review was run to catch.

## Consequences

- The SRS's §10 clarifications section is removed; the SRS states the resulting requirements and
  links to this decision and to `docs/notes/open-questions.md` for what remains open.
- A future amendment to `docs/idea.md` that adds manual redaction would supersede the Q10 portion
  of this decision and require a new SRS requirement.
- Design/API/UI specs can rely on Q8, Q9, Q11, and the Q4-partial resolution as settled.

## Related Documentation

- [Spec](../specs/srs.md)
- [Open questions](../notes/open-questions.md)
- [Decision 0001](./0001-multi-model-spec-review.md) — the review that surfaced these gaps.
- [Work Item](../dev-log/0001-srs-generation.md)
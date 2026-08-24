# [0006] Resolve OQ-14 — retention default discard + first-import confirm

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Record the owner's product answer to OQ-14 and make idea.md, the Aisha story, SRS, design,
API, testing, and indexes say the same thing.

## Implementation

- Amended `docs/idea.md`: factory default is discard; first upload asks the user to set the
  policy (discard pre-selected).
- Updated `docs/user-story.md`: Aisha confirms discard on first import and overrides to retain
  for the bank statement (was: global retain).
- Decision 0007. SRS FR-1.4 + AC-7. API: `confirmed` flag, `retention_policy_unset`. Design
  Config/Importer refuse while unconfirmed. Testing §6.7.

## Resolution

OQ-14 is resolved. Remaining open item in the register is OQ-4 save-dialog chrome (UI spec).

## Related Documentation

- [Decision 0007](../decisions/0007-retention-default-discard.md)
- [idea.md](../idea.md)
- [Spec — SRS](../specs/srs.md)
- [Spec — API](../specs/api.md)
- [Open questions](../notes/open-questions.md)

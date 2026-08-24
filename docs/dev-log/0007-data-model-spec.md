# [0007] Data model spec generation and Claude+Gemini review

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce the v1 data model as the **single source** for named types, SQLCipher schema, envelope
plaintext, and identifiers — without adding a server database or a sync protocol.

## Implementation

- Drafted `docs/specs/data-model.md`. Added envelope kind **8 `document_meta`** to
  architecture AAD. Reviewed with Gemini (`agy`) and Claude (`claude -p`); raw reviews in
  `docs/notes/reviews/`. Reconciled, then updated parent specs and indexes.
- **Follow-up (same day):** moved logical types out of `design.md` §3 and `architecture.md`
  (`KeystoreItem`, `LocalAccount`, artifact kind list, audit field comments) into
  `data-model.md`. Those specs now link here and do not restate field lists. API DTOs remain
  IPC projections. SRS D-2/D-4 stay requirements and point at this schema.

## Problems Encountered

- `DetectedFieldRecord.text` was described in prose but missing from the JSON schema (Claude).
- `originals_flag` lacked semantics (Claude).
- FK `ON DELETE RESTRICT` vs destroy-artifact order unspecified (Gemini).
- `never_retain` vs per-document `retain`|`discard` (Gemini).
- Unconfirmed import could be misread as creating rows (Claude).
- Types were defined in three places (design structs, architecture keystore/account, data-model
  envelopes), so implementers could fork a second schema.

## Resolution

- Full `DetectedField` including nested `span.text`; approved artifact keeps a field snapshot;
  variants stay `field_id`+decision.
- `originals_flag` maps architecture §6.1 / share `no_originals_left_device`.
- Delete sequence: variant rows → document row → artifacts. Partial unique indexes on
  kind 4 and 5. RFC 4648 padded Base64. Unconfirmed import: no rows.
- **C-DM-7:** field lists and envelope kind codes live only in this spec.

## Related Documentation

- [Spec — data model](../specs/data-model.md)
- [Spec — architecture](../specs/architecture.md)
- [Spec — design](../specs/design.md)
- [Raw reviews](../notes/reviews/)

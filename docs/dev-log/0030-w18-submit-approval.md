# [0030] W18 — `submit_approval`

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship `submit_approval` (api.md §5.4): write the canonical kind=1 `ApprovedVersion`
(including `redacted_content` from the W17 overlap rule), emit audit `approve` without
span text, drop the RAM approval session and discard-path original on Vault ack, and
make a second `open_approval` return `already_approved`.

Explicitly **not** in this chunk: `abort_approval` / lock-vs-retention catalog deletion
(W19), PDF export (W23), variants, share.

Per the [agent roster](../agent-roster.md), W18 is Opus tier (AC-1 core).

## Implementation

### Catalog

- `FieldDecisionKind` / `FieldDecision` / `RedactedDocument` / `ApprovedVersion` live on
  `core/src/catalog.rs` so envelope JSON does not create a catalog→session cycle.
  `session` re-exports `FieldDecisionKind` for IPC DTOs.
- `DocumentStore::store_approved` / `load_approved` / `load_original`. Vault insert of
  kind=1 plus `UPDATE … WHERE approved_artifact_id IS NULL` (C-DM-4).

### Command

- Requires `lifecycle == decided`; otherwise `approval_bad_state`.
- `overlap::redact_document` omits redacted bytes (not overlay). Coordinate space is
  page-span offsets so PDF `raw_bytes` length does not pollute extracted-text ranges.
- On Vault ack: drop the RAM session (C-DES-1: `get_approval_view` → `not_found`) and
  `pending_bodies` (design §2.1). Discard never wrote kind=2; retain leaves it.
- Audit `approve`: `{ decisions: [{ field_id, label, decision }] }` — no span text.

## Resolution

- `core/tests/submit_w18.rs`: AC-1 text and PDF paths; lock/unlock metadata; discard
  original not decryptable; retain original still decrypts; `already_approved`; approve
  audit payload without canaries.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W19 — `abort_approval` and lock vs retention.

## Related Documentation

- [Development Plan — W18](../dev-plan.md#w18--submit_approval-ac-1-core)
- [Agent roster — W18](../agent-roster.md)
- [Spec — api.md §5.4](../specs/api.md)
- [Spec — data-model §6.3 / §8](../specs/data-model.md)
- [Spec — testing §6.1 AC-1](../specs/testing.md)
- [Dev log 0029 — W17 overlap](./0029-w17-overlap.md)

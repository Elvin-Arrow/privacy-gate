# [0032] W20 — `delete_document` (DEK destroy)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship irrevocable `delete_document` (FR-4.6): overwrite-and-drop wrapped DEKs then delete
artifact and catalog rows (architecture §4.3 / data-model §7). Vault load fails afterwards;
audit `delete`. Do not treat decrypt-with-a-pre-copied-DEK as the NFR-R2 oracle.

Explicitly **not** in this chunk: OS secure-erase; `delete_retained_original` (W21).

Per the [agent roster](../agent-roster.md), W20 is Opus tier (gated DEK destroy).

## Implementation

- `zeroize_key_material` + `destroy_document_in_tx`: UPDATE wrapped_dek/nonce/ciphertext
  to zeros, then delete variant → document → artifact (FK order).
- `drop_unapproved` (W19) now routes through the same destroy path after refusing an
  approved document.
- `delete_document` command: `not_found` if missing; drops a matching RAM approval
  session; audit `{ doc_id }` with no span text.

## Resolution

- `core/tests/delete_w20.rs`: gating; get/open `not_found`; artifact count 0; retained
  original unreadable; audit `delete`.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W21 — `delete_retained_original`.

## Related Documentation

- [Development Plan — W20](../dev-plan.md#w20--delete_document-dek-destroy)
- [Spec — architecture §4.3](../specs/architecture.md)
- [Spec — testing §8 DEK destroy](../specs/testing.md)
- [Dev log 0031 — W19 abort/lock](./0031-w19-abort-lock.md)

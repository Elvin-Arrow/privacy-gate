# [0033] W21 — `delete_retained_original`

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Drop the retained original only, leaving the canonical approved version byte-identical.
Idempotent if already discarded. Audit `discard_original` only when an original existed.

Explicitly **not** in this chunk: variants (W22); changing approved content.

## Implementation

- `DocumentStore::destroy_original`: overwrite-and-drop kind=2, `original_artifact_id =
  NULL`. Returns whether one was present.
- `delete_retained_original` command: `not_found` if the document is missing; otherwise
  always returns the updated summary.

## Resolution

- `core/tests/delete_original_w21.rs`: approved remains (including kept canary); original
  unreadable; second call does not append a second audit; discard-path import does not
  audit.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W23 — PDF re-render (true removal).

## Related Documentation

- [Development Plan — W21](../dev-plan.md#w21--delete_retained_original)
- [Spec — api.md §5.3](../specs/api.md)
- [Dev log 0032 — W20 delete_document](./0032-w20-delete-document.md)

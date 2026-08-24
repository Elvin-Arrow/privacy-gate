# [0034] W22 — Variants

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Store, list, get, and delete named variants of an approved document. No edit-in-place;
names unique per document; `get_variant` carries field_id + decision only (no span text).
Apply-on-share is W26.

## Implementation

- Envelope kind=3 `VariantRecord` (`name`, `created_at_unix_ms`, `overrides`). `doc_id` is
  AAD; `variant_id` is SQL.
- C-DM-4: `save_variant` requires a canonical approved artifact (`not_approved` otherwise).
- Duplicate name → `variant_name_conflict`. SQL `name` is a cache; decrypt mismatch is not
  served.
- `delete_variant` overwrite-and-drops the kind=3 artifact (architecture §4.3).

## Resolution

- `core/tests/variants_w22.rs`: gating, unapproved/unknown, name validation, uniqueness,
  C-API-2 JSON, delete leaves approved.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W23 — PDF re-render (true removal).

## Related Documentation

- [Development Plan — W22](../dev-plan.md#w22--variants)
- [Spec — api.md §5.5](../specs/api.md)
- [Spec — data-model.md §6.4](../specs/data-model.md)
- [Dev log 0033 — W21 delete original](./0033-w21-delete-original.md)

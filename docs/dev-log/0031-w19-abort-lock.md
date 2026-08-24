# [0031] W19 — `abort_approval` and lock vs retention

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship `abort_approval` and the lock/abort retention rules (api.md §5.4, data-model §8):
retain keeps the catalog (and encrypted original) so the user may `open_approval` again;
discard zeroizes the RAM body and deletes the catalog row. Core must not serve the
approval view after abort.

Explicitly **not** in this chunk: UI copy (W33), `delete_document` DEK destroy (W20).

Per the [agent roster](../agent-roster.md), W19 is Sonnet tier.

## Implementation

- `abort_approval`: drop the RAM session; on discard, `DocumentStore::drop_unapproved`
  (document row then kind=8/2 artifacts, FK order). On retain, catalog stays.
- `lock` walks unapproved documents and drops those whose meta retention is discard,
  then drops `OpenSession` as before.
- `open_approval` reconstructs page IR from a retained original when `pending_bodies`
  is empty after lock (api.md: retain may open again).

`catalog_w10`'s newest-first-survives-lock test now uses retain — discard rows are not
supposed to survive lock.

## Resolution

- `core/tests/abort_w19.rs`: both retention paths; view gone after abort; lock drops
  unapproved discard and keeps approved discard + unapproved retain.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W20 — `delete_document` (DEK destroy).

## Related Documentation

- [Development Plan — W19](../dev-plan.md#w19--abort_approval-and-lock-vs-retention)
- [Spec — api.md §5.4](../specs/api.md)
- [Spec — data-model §8](../specs/data-model.md)
- [Dev log 0030 — W18 submit](./0030-w18-submit-approval.md)

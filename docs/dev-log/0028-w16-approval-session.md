# [0028] W16 — Approval session

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship `open_approval`, `get_approval_view`, and `set_field_decisions` (api.md §5.4): one
RAM approval session per process, span text only on those commands (C-API-2 / C-DES-1),
lifecycle `awaiting_decisions` | `decided`.

Explicitly **not** in this chunk: `submit_approval` (W18), `abort_approval` and lock vs
retention catalog deletion (W19), overlap rule (W17), share preview, variants.

Per the [agent roster](../agent-roster.md), W16 is Sonnet tier (session/state management).

## Implementation

### RAM session on `OpenSession`

`ApprovalSession` (data-model §5.10) lives on `OpenSession`, so `lock` drops it with the
master key. Page IR is not in `document_meta` (data-model §6.1); `import_document` keeps
the in-memory `Document` on `pending_bodies` for this unlock so discard-path approval can
still show pages.

### Commands

- `open_approval`: `not_found` → `already_approved` (when `has_approved_version`) →
  `approval_busy`. View always includes `span.text` and page body text.
- `get_approval_view` / `set_field_decisions`: unknown `approval_session_id` is
  `not_found`; unknown `field_id` is `invalid_input`; `Committed`/`Aborted` is
  `approval_bad_state` (those states are reached in W18/W19).
- Partial `set_field_decisions` leaves `awaiting_decisions`; all fields decided →
  `decided`. Overwriting a decision is allowed until submit.

`get_document` is unchanged: `DocumentSummary` only, no field text.

## Resolution

- `core/tests/approval_w16.rs`: gating, not_found, C-API-2 span text, `approval_busy`,
  partial vs complete decisions, unknown field_id.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W17 — overlap / nested fields.

## Related Documentation

- [Development Plan — W16](../dev-plan.md#w16--approval-session)
- [Agent roster — W16](../agent-roster.md)
- [Spec — api.md §5.4](../specs/api.md)
- [Spec — design §2.3, C-DES-1](../specs/design.md)
- [Spec — data-model §5.10](../specs/data-model.md)
- [Spec — testing C-API-2](../specs/testing.md)
- [Dev log 0027 — W15c backend selection](./0027-w15c-backend-selection.md)

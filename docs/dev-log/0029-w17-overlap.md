# [0029] W17 — Overlap / nested fields

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Implement design §3.5 as a pure function: innermost explicit Keep wins over an outer
Redact; partial (non-nested) overlaps are redact-wins; one byte-offset rule at export.
Table-driven tests plus `proptest`. Not wired into `submit_approval` (W18).

Explicitly **not** in this chunk: changing the SRS (no manual draw-redact), share render,
`submit_approval`.

Per the [agent roster](../agent-roster.md), W17 is Opus tier (precedence logic).

## Implementation

### `core/src/overlap.rs`

- `offset_is_redacted` / `redacted_ranges` (merged half-open `[start, end)`).
- A covering `Redact` hides a byte unless a covering `KeepVisible` is **more specific**:
  strict span containment, or `parent_field_id` ancestry (cycles are ignored).
- Fields without a decision, and zero-length spans, do not cover.

This is a testing.md §5.3 gated module; the PR mutants job is W38.

## Resolution

- `core/tests/overlap_w17.rs`: nested keep-inside-redact (with and without parent id);
  nested redact-inside-keep; partial overlap; nested keep inside a partial-overlap
  intersection; parent-id on equal spans; ranges match per-byte; two property tests for
  the nested-keep and partial-overlap invariants.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W18 — `submit_approval`.

## Related Documentation

- [Development Plan — W17](../dev-plan.md#w17--overlap--nested-fields-design-35)
- [Agent roster — W17](../agent-roster.md)
- [Spec — design §3.5](../specs/design.md)
- [Spec — testing §5.3, §8](../specs/testing.md)
- [Dev log 0028 — W16 approval session](./0028-w16-approval-session.md)

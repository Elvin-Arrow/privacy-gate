# [0035] W23 — PDF re-render (true removal)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Re-render a new PDF from `ApprovedVersion.redacted_content` so redacted plaintext cannot
survive in a content stream. No source-PDF mutation; no incremental `/Prev`.

Explicitly **not** in this chunk: save dialog; plaintext `.txt` export; `preview_share`
(W24).

## Implementation

- `core/src/export.rs` `render_redacted_pdf`: `pdf-writer` 0.14, Helvetica, uncompressed
  content streams, Producer/Creator `Privacy Gate`, Author/Subject/Keywords omitted.
- Input is remaining spans only — the function has no source-PDF parameter.

## Resolution

- `core/tests/export_w23.rs`: redacted canary absent in UTF-8/UTF-16 raw bytes and
  extracted text; keep canary present; no `/Prev`; empty and multi-page docs.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W24 — share preview + commit (export).

## Related Documentation

- [Development Plan — W23](../dev-plan.md#w23--pdf-re-render-true-removal)
- [Spec — architecture.md §11](../specs/architecture.md)
- [Dev log 0034 — W22 variants](./0034-w22-variants.md)

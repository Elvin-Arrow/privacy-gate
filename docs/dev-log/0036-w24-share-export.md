# [0036] W24 — Share preview + commit (export)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Person-export `preview_share` / `commit_share`: one live preview token (10 min / lock /
replace), byte-identical commit PDF, suggested filename and PDF info dictionary (api.md §7).

Explicitly **not**: Cloud AI (W27); applying overrides/variants at share (W26); writing
files from core; save dialog (W34).

## Implementation

- Preview lives on `OpenSession` (dropped on lock). Second preview replaces the first.
- Filename: Unicode letters/digits, collapse punctuation, `{stem}-redacted-{YYYYMMDD}.pdf`.
- PDF Title/CreationDate/ModDate from that filename and export timestamp.
- `share_to_ai` → `cloud_ai_not_configured` before assemble.
- Audit `share` only on commit.

## Resolution

- `core/tests/share_w24.rs`: `not_approved`, token expiry/replace/lock, byte-identical
  commit, filename, redacted canary absent, no Author / display name in metadata.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W26 — ephemeral overrides + variants on share.

## Related Documentation

- [Development Plan — W24](../dev-plan.md#w24--share-preview--commit-export)
- [Spec — api.md §5.6, §7](../specs/api.md)
- [Dev log 0035 — W23 PDF re-render](./0035-w23-pdf-rerender.md)

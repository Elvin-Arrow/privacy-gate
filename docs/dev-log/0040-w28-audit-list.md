# [0040] W28 — `list_audit_events` (AC-4)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Let a user answer "what did I share?" (FR-7) by reading back the audit chain every earlier
chunk (W10, W12, W18, W20, W21, W24, W27) already writes to — filtered, paginated, and with
the degraded-session "verified prefix only" rule (api.md §5.8) — without ever decrypting a
document artifact or re-verifying the HMAC chain in the webview (that already happened once,
at `unlock`).

## Implementation

- `core/src/audit.rs`: `EventType` gained `Serialize`/`Deserialize` (snake_case, data-model
  §2.1) — the only piece of `crate::audit` that had no wire form yet, since nothing before
  this chunk put it directly on a DTO.
- `core/src/session.rs`:
  - `ListAuditEventsIn` / `AuditEventDto` / `ListAuditEventsOut` (api.md §5.8). `payload` is
    `serde_json::Value` — whatever JSON object the originating command already wrote; W28
    doesn't reshape it; the per-`event_type` shapes (§5.8.1) were already correct by
    construction at every `record_audit_append` call site from earlier chunks.
  - `list_audit_events`: new session-table row, `unlocked` **and** `degraded_integrity`
    (unlike the generic config/document row — same posture as `get_integrity_report`).
    Filters `doc_id` → `event_type` → `after_sequence` (exclusive cursor) → `limit`
    (1..=200, `invalid_input` outside that range). While degraded, additionally clamps to
    `sequence <= integrity_report.tail_sequence` — using `tail_sequence` rather than
    re-deriving from `first_bad_sequence` handles both `VerifyOutcome::Failure` shapes
    (a pinpointed bad row, or an internally-valid chain that just doesn't reach the
    persisted head) with one rule. Never touches `self.documents`/`master` — audit rows
    are plain SQLCipher-protected text, not a second envelope layer, so "does not decrypt
    document artifacts" holds structurally rather than by a special case.
  - `audit_row_to_dto`: the one fallible step is decoding `payload_jcs` back into JSON,
    which only fails on a corrupt/tampered row (every writer produces it via
    `serde_json::to_string`).

## Resolution

- `core/tests/audit_list_w28.rs`: 8 tests — session gating (`unlocked`/`degraded` only,
  refused before unlock); `limit` boundary (0 and 201 rejected, 1 and 200 accepted); the
  AC-4 flow end to end (import → approve with a keep + a redact decision → export share →
  a *failed* AI share against an unreachable endpoint, since dev-plan's "failed HTTP still
  audits attempt" means AC-4 needs to show attempts, not only successes) — asserting every
  event type appears, `no_originals_left_device` is present only on share rows, the approve
  payload is exactly `{field_id, label, decision}` with no extra keys, and neither redact
  nor keep canary text nor the AI instruction nor the API key appears anywhere in a
  whole-flow JSON dump (C-API-1/2); `event_type` filtering; an unknown `doc_id` filter
  returning empty rather than an error; `after_sequence`/`next_sequence` pagination across
  exactly two pages; and a degraded session (three rows appended directly via
  `crate::audit::append`, the second corrupted, same technique `audit_w5.rs` uses) seeing
  only the one row that verified before the break, with `next_sequence: None`.
- `cargo test -p pg-core`: full suite green (`audit_list_w28` 8/8; every earlier suite,
  including `audit_w5.rs`, unmodified and green). `cargo clippy -p pg-core --lib --test
  audit_list_w28 --test cloud_ai_w27`: clean (the pre-existing, unrelated `ollama_w15b.rs`
  `--all-targets` clippy warning from before W27 is still there, still untouched).

Next: W29 — Tauri IPC, CSP, events.

## Related Documentation

- [Development Plan — W28](../dev-plan.md#w28--list_audit_events-ac-4)
- [Spec — srs.md FR-7](../specs/srs.md)
- [Spec — api.md §5.8 `list_audit_events` / `AuditEventDto`](../specs/api.md)
- [Spec — data-model.md §5.8.1 `EventPayload`](../specs/data-model.md)
- [Spec — testing.md §6.4 AC-4](../specs/testing.md)
- [Dev log 0039 — W27 Cloud AI](./0039-w27-cloud-ai.md)
- [Dev log 0015 — W5 audit chain](./0015-w5-audit-chain.md)

# [0040] W28 — `list_audit_events` (AC-4)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Deliver `list_audit_events` (api.md §5.8, FR-7, AC-4): a filtered, paginated read path over
the audit chain W5 already writes and verifies, so a user can answer "what did I share?" —
per document, what was detected, approved, redacted, exported, and sent to AI, with no
redacted field text, keys, or raw chain-integrity material ever reaching the DTO. This chunk
adds no new write to the chain; it is purely a projection of `AuditRow` (via `replay()`)
into `AuditEventDto`.

## Implementation

- `core/src/audit.rs`: `EventType` gained `Serialize`/`Deserialize` (`#[serde(rename_all =
  "snake_case")]`) using the exact api.md §5.8 wire strings (`"import"`, `"detect"`,
  `"approve"`, `"share"`, `"discard_original"`, `"delete"`). This is the one mapping —
  `session.rs`'s `event_type` filter input and `AuditEventDto.event_type` both reuse it
  rather than inventing a second string mapping.
- `core/src/session.rs`: `ListAuditEventsIn { doc_id, event_type, after_sequence, limit }`
  (all `Option`), `ListAuditEventsOut { events, next_sequence }`, and `AuditEventDto {
  sequence, event_type, doc_id, produced_at, no_originals_left_device, payload }` — built
  only from `AuditRow`'s already-public fields (`sequence`/`event_type`/`doc_id`/
  `produced_at_unix_ms`/`originals_flag`/`payload_jcs`); `entry_signature` and
  `prev_entry_hash` are never read by the mapper (`audit_row_to_dto`), so they structurally
  cannot reach the DTO (C-API-1/2/5; dev-plan "Do not: webview HMAC verify"). `payload` is
  `payload_jcs` parsed back to a `serde_json::Value` — not re-derived — matching what each
  producing command (`import_document`, `submit_approval`, `commit_share`,
  `delete_document`, `delete_retained_original`) already writes via `serde_json::json!`.
  `SessionManager::list_audit_events`: gated via the new `SESSION_TABLE` row (`Unlocked` +
  `DegradedIntegrity`); validates `limit` (`None` → 50, `Some(0)` or `Some(n>200)` →
  `invalid_input`, following the `ai_instruction`/`save_variant`-name precedent of
  rejecting explicit out-of-range values rather than silently clamping them); replays the
  chain once, filters by `doc_id`/`event_type`/`after_sequence` (exclusive cursor), and
  while degraded additionally drops any row at or past `first_bad_sequence` before
  filtering — the verified-prefix rule applies ahead of the caller's own filters, not
  after, so a degraded session can never see a row integrity verification could not vouch
  for regardless of what it asks for. Pagination: rows are collected until `limit` is
  reached; if a further matching row exists beyond that point the fetch stops there and
  `next_sequence` is `Some(last_returned.sequence)`; if the filtered set runs out first,
  `next_sequence` is `None` — proven by a test with 9 rows across a 4-row page size that
  pages to completion and checks every sequence was seen exactly once, in order.

## Ambiguities resolved

- **Degraded-session availability:** not actually ambiguous — api.md §2's table already has
  an explicit `list_audit_events` row (`no | no | yes | yes (verified prefix only)`), so no
  `get_integrity_report`-style inference was needed; the row was transcribed directly, same
  as every other `SESSION_TABLE` entry's comment convention.
- **Out-of-range explicit `limit`:** api.md §5.8 states the range ("1..=200, default 50")
  without saying what happens outside it. Followed the existing codebase precedent (W22
  `save_variant` name length, W27 `ai_instruction` 1..=4000) of rejecting an explicit
  out-of-range value as `invalid_input` rather than clamping it silently — pinned down with
  both a `Some(0)` and a `Some(201)` test so the choice is deliberate, not accidental.
- **Pagination cursor semantics:** `after_sequence` is an exclusive cursor and `next_sequence`
  is set only when the page was cut short by hitting `limit` with more matching rows left —
  not merely "the page happened to be exactly `limit` rows long." The implementation checks
  this by looking one row past `limit` inside the same filtered iteration (not by comparing
  `events.len() == limit`), so a filtered set that ends exactly on a page boundary correctly
  reports `next_sequence: None`.

## Resolution

- `core/tests/audit_list_w28.rs` (new, 11 tests): session gating
  (`first_run`/`locked` refused, `unlocked`/`degraded_integrity` allowed); AC-4 end-to-end
  through `import_document` → `open_approval`/`set_field_decisions`/`submit_approval` →
  `preview_share`/`commit_share` (export), asserting `import`/`detect`/`approve`/`share`
  appear in ascending order for that `doc_id`, `no_originals_left_device` is `Some` only on
  the share event, and every payload shape spot-checks against api.md §5.8 (`retention`/
  `source_filename`/`detector_id: null` on import; `field_ids`/`labels` arrays on detect;
  `field_id`/`label`/`decision` per approve entry; `kind`/`has_ai_instruction`/`doc_ids` on
  share) with no `PG-CANARY` or passphrase substring anywhere in any payload; the same flow
  through W27's `share_to_ai` (own mock HTTP server, same pattern as `cloud_ai_w27.rs`),
  proving the read side surfaces `has_ai_instruction: true` with no instruction text and no
  API key in the returned payload; pagination across 9 rows with a 4-row page size, seen
  exactly once each in ascending order; `doc_id` filter isolating one of two documents;
  `event_type` filter isolating `Approve` out of two documents' rows; a degraded-integrity
  session (built by corrupting the `detect` row's payload the same way `audit_w5.rs`'s
  `flip_a_payload_byte_causes_degraded_integrity` does, then unlocking) returning only
  sequence 1 (`import`), not sequence 2 or later; explicit `limit: Some(0)` and
  `Some(201)` both `invalid_input`; absent `limit` defaulting to ≤ 50; `not_in_session`
  before any account exists.
- `cargo test -p pg-core` green: 416 passed / 2 ignored / 0 failed across every suite
  (11 new tests; all W0–W27 suites unmodified and green, including `audit_w5.rs`,
  `session_gating_w4.rs`, `share_w24.rs`, `cloud_ai_w27.rs`). `cargo test --workspace` also
  green (`privacy-gate` binary crate unaffected — no Tauri wiring in this chunk).
  `session_gating_w4.rs`'s `every_api_md_2_cell_is_covered` only enumerates the five W4-era
  commands per its own module docs (unchanged by W16/W22/W24/W27 either), so it needed no
  new row. `npm run check` not run — this chunk is core-only (dev-plan W28 "Integrate: read
  path"; no webview change).

Next: W29 — Tauri IPC, CSP, events.

## Related Documentation

- [Development Plan — W28](../dev-plan.md#w28--list_audit_events-ac-4)
- [Spec — api.md §5.8 Audit](../specs/api.md)
- [Spec — srs.md FR-7 / AC-4](../specs/srs.md)
- [Spec — testing.md §6.4 AC-4](../specs/testing.md)
- [Spec — audit.rs / architecture.md §6 (chain this chunk reads, does not write)](../specs/architecture.md)
- [Dev log 0039 — W27 Cloud AI](./0039-w27-cloud-ai.md)

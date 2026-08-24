# [0015] W5 — Audit chain and integrity

- **Status:** Complete (core green; Opus review pass complete, 3 blocking findings fixed and
  each verified against its own mutation; 5 nits addressed — see Review below)
- **Date:** 2026-08-24

## Objective

Deliver architecture §6's audit chain: append-only rows, the canonical HMAC encoding v1,
replay verification against the keystore-persisted `AuditHead`, and the three §6.3 unlock
outcomes (clean, crash-window fast-forward, integrity failure). Makes
`SessionState::DegradedIntegrity` reachable for the first time — W2/W3/W4 only ever declared
the variant — and delivers `get_integrity_report` (api.md §5.1).

Explicitly **not** in this chunk (dev-plan.md W5 "Do not: UI integrity screen (W35); user
vault restore"): no concrete `EventPayload` shapes for import/detect/approve/share (those
commands don't exist yet — `crate::audit`'s module doc states this fence directly), no UI,
no restore flow.

Per the [agent roster](../agent-roster.md), W5 is Opus tier ("Hash-chain + HMAC +
degraded-session logic is exactly the 'two-phase commit isn't atomic' class of bug Gemini
already caught once") with a mandatory adversarial review pass, mutation-testing-focused
rather than a read-through.

## Implementation

### `core/src/audit.rs` — the new module

- **Canonical encoding v1** (`canonical_bytes`) — architecture §6.1's exact byte layout:
  version, `sequence` BE, `event_type`, `produced_at_unix_ms` BE, `doc_id_present` +
  `doc_id_len` + bytes, `originals_flag`, `payload_len` + JCS bytes, `prev_entry_hash`.
  `entry_signature` is structurally excluded (it's computed *over* this string). One private
  `head_hash_of` function (`sha256(canonical_bytes(row))`) is the single source both
  `append` and `verify_against_head` use for "what does this entry hash to" — the append
  side and the verify side can never independently drift into agreeing on the wrong thing,
  because there is only one implementation of that computation.
- **`append`** — the only place `prev_entry_hash`/`entry_signature` are computed; rejects
  oversized `doc_id`/payload up front rather than let the `u16`/`u32` length-prefix casts
  silently wrap (review nit N3).
- **`verify_against_head`** — one linear replay pass checking sequence contiguity,
  `prev_entry_hash` chaining, and `entry_signature` against `mac_key` for every row, then
  classifies the fully-verified tail against the persisted `head`: `T == H` (both sequence
  *and* head_hash) → `Clean`; `T.sequence` in `H.sequence + 1..=32` **and** the entry at
  `H.sequence` actually hashes to `H.head_hash` → `FastForward`; otherwise → `Failure` with
  `Truncation` (`T.sequence < H.sequence`) or `Modification` (everything else, including a
  mid-chain HMAC break).
- **`AuditStore`** trait — `append_row`/`replay`, implemented by `SqlCipherVault` (below).

### `core/src/vault.rs` — `AuditStore` for `SqlCipherVault`

Reads/writes the `audit_entry` table W3's schema already creates, over the same shared
connection as `AccountStore`/`VaultBackend`. Two new test-support methods,
`test_only_corrupt_payload`/`test_only_truncate_after`, simulate an attacker who edited the
DB file directly (architecture §2.4) — narrowly scoped to corrupting or shrinking existing
rows, never forging a new one.

### `core/src/session.rs` — the integration

- `unlock` now actually runs architecture §3.3's "verify the audit chain against
  `audit_head`" step, previously a W2-era stub (`verify_integrity_on_unlock`) that always
  returned `None`. On a fast-forward, the new head is persisted to the keystore
  **immediately** (not deferred to `lock`), so a second crash before any new append cannot
  widen the same gap further.
- `OpenSession` gained `degraded: bool` and `integrity_report: IntegrityReport`; `state()`
  is now degraded-aware instead of hardcoding `Unlocked` whenever a session is open.
- New `get_integrity_report` command + `SESSION_TABLE` row (`unlocked` /
  `degraded_integrity`, matching api.md §2).
- `NullAuditStore` + `new_with_vault_and_audit` constructor preserve every W2/W3/W4 test
  call site unmodified — `SessionManager::new`/`new_with_vault` still work exactly as before
  (an empty replay against `AuditHead::GENESIS` is always `Clean`).

### `core/Cargo.toml`

`hmac` promoted from a W1-era dev-only dependency (the HKDF known-answer reference) to a
real production dependency for HMAC-SHA-256.

## Review (roster-mandated Opus pass)

Verdict: **REQUEST CHANGES** on the first submission. The reviewer's headline finding: the
production verification algorithm itself was correct on every case checked, but this is a
`testing.md` §5.3 gated module (S = 1.00 required) and the **test suite** had a real,
demonstrated gap — a mutant survived. Every fix below is backed by a mutation the reviewer
(and I, re-verifying independently) actually ran: apply the mutation, confirm the new test
fails; revert, confirm it passes again.

**Blocking 1 — `head_entry_matches` (architecture §6.3's third fast-forward condition) had
zero test coverage.** Every prior fast-forward test started from `AuditHead::GENESIS`, where
the check degenerates to a trivial comparison against the zero digest. Deleting `&&
head_entry_matches` from the fast-forward condition left the full 15-test suite green.
Fixed with `head_hash_mismatch_inside_the_crash_window_is_not_fast_forwarded`: a chain whose
persisted head sits inside the crash window by sequence but carries a `head_hash` that
doesn't match what the chain actually has at that sequence. Verified: fails under the
mutation, passes on the real code.

**Blocking 2 — no test proved a wholesale-forged chain is rejected.** `prev_entry_hash` is
unkeyed SHA-256, so an attacker without `audit_mac_key` can still build a chain with perfect
sequence contiguity and perfect chaining — the *only* thing that can catch it is
`entry_signature`. No existing test constructed such a chain; the closest one
(`flip_a_payload_byte...`) turned out to be caught by a *different* check (a head-hash
mismatch against the persisted head from before the flip), not by the HMAC check it was
meant to exercise — so even that test didn't isolate what it claimed to test. Fixed with
`a_chain_forged_without_the_real_mac_key_is_not_fast_forwarded`: appends a fully
self-consistent 3-row chain under a key that is deliberately not the session's real
`audit_mac_key`, from a fresh (genesis-head) vault — the sharpest form of the property,
with no other check able to save it. Verified against the reviewer's own mutation (`sig_ok =
true`).

**Blocking 3 — a failed fast-forward persist left the vault open behind a `Locked`-reporting
session.** In `unlock`, `self.vault.open(...)` runs before the fast-forward persist; if
`self.keystore.store(...)` then failed, the function returned early without closing the
vault it had just opened. `self.open` stayed `None`, so `state()` reported `Locked` while
the SQLCipher connection was still live — a partial-open state architecture §3.3 explicitly
forbids ("no partial open"), and inconsistent with how carefully `create_account`'s own
rollback paths already handle this class of failure. Fixed: close the vault before
propagating the keystore error. Regression test
`failed_fast_forward_persist_does_not_leave_the_vault_open` uses the existing
`InMemoryKeystore::fail_next_store()` fault injection to force exactly this window, asserts
`vault.is_open() == false` immediately after the error, and confirms a retry unlocks
cleanly. Verified against a hand-reverted mutation of the fix.

**Nits fixed:** two clippy warnings (`manual_range_contains` in the crash-window check,
`type_complexity` on the raw-row SQL tuple — both in files this chunk touches, matching the
"zero warnings in the chunk's own files" bar the last two dev-logs set); oversized
`doc_id`/payload now rejected in `append` rather than silently truncated by the length-prefix
casts; an empty-chain-against-a-non-genesis-head boundary test
(`fully_truncated_chain_against_a_non_genesis_head_causes_degraded_integrity`). Left as
documentation-only (not code changes, per the reviewer's own framing): `sqlcipher_key()`/
`audit_mac_key()` remaining accessible during a degraded session is necessary (the replay
needs the open DB) and is already correctly enforced at the `SESSION_TABLE` layer, not at
those accessors — worth a doc note for whichever W8+ chunk next reaches for them, not a W5
change.

## Resolution

- `cargo test -p pg-core`: **140 passed, 1 ignored** — 16 in `audit_w5.rs` (12 original +
  4 from the review pass), all 8 lib unit tests (4 new: `audit.rs`'s canonical-encoding
  tests), and W1 (35) / W4 (18) / W2 (53) / W3 (10) entirely unmodified and green.
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings on every file this chunk touches.
- dev-plan W5 "Tests first" line, verified: happy append
  (`append_with_correctly_persisted_head_unlocks_clean`); flip payload byte → degraded
  (`flip_a_payload_byte_causes_degraded_integrity`); truncation vs head
  (`truncated_tail_below_persisted_head_causes_degraded_integrity` +
  `fully_truncated_chain_against_a_non_genesis_head_causes_degraded_integrity`); crash-window
  fast-forward still `unlocked`, boundary-tested at both 32 (yes) and 33 (no)
  (`crash_window_covers_up_to_32_unpersisted_appends`,
  `thirty_three_unpersisted_appends_is_not_a_crash_window`); HMAC break not fast-forwarded
  (`hmac_break_is_not_fast_forwarded_even_within_the_crash_window` +
  `a_chain_forged_without_the_real_mac_key_is_not_fast_forwarded`); degraded session cannot
  reach import/approve/share (`degraded_session_cannot_reach_unimplemented_document_commands`
  — vacuously true today since those commands are unregistered, exactly as dev-plan's
  parenthetical anticipates, "those commands fail even if not fully implemented").
- `get_integrity_report` matches unlock outcome for all three cases, verified directly
  against `IntegrityReport.kind`/`ok`/`head_sequence`/`tail_sequence`/`first_bad_sequence`.
- Scope held: no `EventPayload` variants, no UI, no restore, `rusqlite`/`hmac`/`sha2` usage
  confined to `audit.rs`/`vault.rs`.
- Modules to add to the W38 mutation-gate list alongside W1/W2/W3's:
  `core/src/audit.rs` in full (canonical encoding + verification is exactly what
  testing.md §5.3 means by "Audit canonical encoding v1 + HMAC verify + crash-window
  fast-forward vs integrity failure").

Next: W6 — retention config (AC-7 core).

## Related Documentation

- [Development Plan — W5 specification](../dev-plan.md#w5--audit-chain-and-integrity)
- [Agent roster — W5](../agent-roster.md)
- [Spec — Architecture §6 (audit-trail integrity, all subsections), §2.4 (threat model), §3.1 (HKDF label)](../specs/architecture.md)
- [Spec — Data model §5.8 (`AuditEntry`/`EventPayload`), §5.9 (`KeystoreItem`, `AuditHead`)](../specs/data-model.md)
- [Spec — API §2 (session table), §5.1 (`get_integrity_report`)](../specs/api.md)
- [Spec — Testing §8 (NFR-R1 tamper/truncation, Crash window, Integrity vs crash, C-API-6)](../specs/testing.md)
- [Decision 0004 — v1 architecture (crash-window finding)](../decisions/0004-v1-architecture.md)
- [Dev log 0013 — W3 empty vault](./0013-w3-empty-vault.md)
- [Dev log 0014 — W4 session gating table](./0014-w4-session-gating-table.md)

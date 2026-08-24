# [0016] W6 — Retention config (AC-7 core)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Deliver `get_retention_default` / `set_retention_default` (api.md §5.2) and the storage
underneath them: the global `Config` artifact (data-model §5.5), envelope-encrypted as the
`artifact` table's unique `kind=4` row (W3's schema). Factory `discard`, `confirmed: false`
(decision 0007); first `set_retention_default` call confirms.

Explicitly **not** in this chunk (dev-plan.md W6 "Do not: first-import modal UI (W32);
per-import override (W10)"): no `import_document`, no `retention_policy_unset` gate (W11
owns wiring the confirmed-check into import), no UI, no `detector_preference` command
(W15c — the field exists in `Config`'s on-disk shape now so that chunk needs no format
bump, but nothing here reads or writes it).

Per the [agent roster](../agent-roster.md), W6 is Sonnet tier, no mandatory second review
("Business-rule logic (decision 0007), not TCB-crypto").

## Implementation

### `core/src/config.rs` — the new module

`Config` is the **first** real envelope-encrypted artifact this codebase writes — W3's
`LocalAccount` is SQLCipher-only (no DEK layer), so this chunk is also the first user of
the `artifact` table's `wrapped_dek`/`nonce`/`ciphertext` columns for their intended
purpose. Two AEAD layers per architecture §3.1's diagram:

- A fresh per-write `Dek` wraps the JSON plaintext under AAD kind 4 (`ArtifactKind::Config`).
- `vault_master_key` wraps that `Dek` under AAD kind 7 (`ArtifactKind::WrappedDek`) — the
  same two-layer shape `crate::keys::wrap_master_key` already uses for the keystore's
  wrapped master key, one level down.

`seal_config`/`open_config` are the pure crypto (no SQL); `ConfigStore` is the storage
trait, `NullConfigStore` the W2–W5-era no-op default (mirrors `NullVault`/`NullAuditStore`).

### `core/src/vault.rs` — `ConfigStore` for `SqlCipherVault`

One implementation detail the spec leaves open: `artifact.wrapped_dek` has no sibling
nonce column (unlike the artifact's own `nonce`/`ciphertext` pair), so wrapping a DEK — a
full AEAD operation with its own random nonce — needs somewhere to put that nonce. This
chunk's choice: the wrapped DEK's own 24-byte nonce is prepended to its ciphertext, one
self-contained blob in the single `wrapped_dek` column (`pack_wrapped_dek`/
`unpack_wrapped_dek`) — analogous to how `crate::keystore`'s mirror struct already made its
own storage-format choice (hex encoding) for a spec-unconstrained detail.

`store()` deletes-then-inserts inside one transaction (the W3-review pattern applied to
`AccountStore::store` now reused here) — `uq_artifact_config`'s unique index enforces at
most one `kind=4` row regardless, but the transaction keeps a failed insert from leaving
zero rows.

### `core/src/session.rs` — the two commands

- `SessionManager` gained a `config: Arc<dyn ConfigStore>` field and a `new_full`
  constructor (keystore + accounts + vault + audit + config); every prior constructor
  (`new`, `new_with_vault`, `new_with_vault_and_audit`) now delegates down to it with
  `NullConfigStore`, so every W2–W5 test call site is untouched.
- `SESSION_TABLE` gained two rows, both `unlocked`-only — api.md §2's generic "All
  document / approval / share / config / cloud-ai / variant / delete" row (`no | no | yes |
  no`): unlike `lock`/`get_account`/`get_integrity_report`, config commands are **not**
  available while `degraded_integrity` (C-API-6).
- Both commands read-modify-write through `Config::default()` as the fallback for a
  missing row — the same fail-open-to-factory posture `crate::keystore` uses for
  `first_run`, so a config row that hasn't been written yet (the normal state right after
  `create_account`) reads as exactly the factory values without a special case.
- `set_retention_default` has **no** paranoid-loosening restriction of its own — api.md
  §5.2 is explicit that `never_retain → retain` is allowed at the global-default layer;
  that restriction belongs to `import_document`'s per-import override, which doesn't exist
  yet (W11).

## Resolution

- `cargo test -p pg-core` green: **11/11** new in `config_w6.rs`, all prior tests (W1
  through W5, 141 total) unmodified and green.
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings on every file this chunk touches.
- dev-plan W6 "Tests first" line, verified: factory values
  (`factory_retention_default_is_discard_and_unconfirmed`, plus a lock/unlock-survival
  variant proving it's actually persisted-or-defaulted, not just an in-memory happy path);
  set confirms (`set_retention_default_confirms_and_persists`, checked again after a
  lock/unlock cycle); `never_retain → retain` global change allowed
  (`global_default_may_loosen_from_never_retain_to_retain`, plus a repeated-changes test);
  import still absent (no `import_document` anywhere in this diff, matching the
  parenthetical "so only config tests").
- C-API-6 (config unavailable while degraded) asserted directly against
  `command_allowed`, same pattern `audit_w5.rs` already established for document commands.
- Envelope-encryption sanity: `config_artifact_is_unreadable_without_the_correct_vault_key`
  confirms a wrong SQLCipher key can't open the file at all — the config plaintext never
  becomes reachable, the same "stolen data file" property architecture §2.4 requires
  everywhere.
- Scope held: no `import_document`, no UI, no `detector_preference` command, no plaintext
  config field on disk (the whole `Config` struct is envelope-encrypted, not just
  SQLCipher-protected like `LocalAccount`).

Next: W7 — Linux keystore fallback.

## Related Documentation

- [Development Plan — W6 specification](../dev-plan.md#w6--retention-config-ac-7-core)
- [Agent roster — W6](../agent-roster.md)
- [Decision 0007 — retention default is discard; first import must confirm (OQ-14)](../decisions/0007-retention-default-discard.md)
- [Spec — API §5.2 (Config commands)](../specs/api.md)
- [Spec — Data model §5.5 (`Config`)](../specs/data-model.md)
- [Spec — Testing §6.7 (AC-7), C-TEST-6](../specs/testing.md)
- [Dev log 0015 — W5 audit chain](./0015-w5-audit-chain.md)

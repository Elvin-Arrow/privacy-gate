# [0013] W3 — Empty vault (SQLCipher)

- **Status:** Complete (core green; full-workspace `cargo test` re-confirmed; Opus review
  pass complete, both blocking findings fixed — see Review below)
- **Date:** 2026-08-24

## Objective

Deliver the SQLCipher vault of [architecture §4](../specs/architecture.md) and
[data-model §7](../specs/data-model.md): create/open the database in raw-key form (not a
passphrase-KDF path), apply the v1 schema (tables may exist empty), and integrate it so
`create_account` / `unlock` open the DB and `lock` closes it — consuming W2's
`SessionManager::sqlcipher_key()` and replacing `InMemoryAccountStore` with a real,
persistent `LocalAccount` row.

Explicitly **not** in this chunk (dev-plan.md W3 "Do not: import, audit HMAC (can insert
later), detector"): no artifact/document/variant/plugin_secret writes beyond schema DDL,
no audit chain (W5), no importer, no detector.

Per the [agent roster](../agent-roster.md), W3 is Sonnet-authored with a mandatory Opus
review pass on the diff before merge (rationale: "Mostly plumbing over a well-defined
schema, but touches at-rest encryption — cheap to have Opus sanity-check the diff").

## Implementation

### `core/src/vault.rs` — the new module

- **`VaultError`** — `WrongKey` (the negative case of the "stolen data file" property,
  architecture §2.4) and `Backend(&'static str)` (everything else), same fixed-class
  discipline as `KeystoreError`/`AccountStoreError`.
- **`VaultBackend`** trait — `open(&self, key) -> Result<(), VaultError>` /
  `close(&self)` / `is_open(&self) -> bool`. `crate::session::SessionManager` holds one and
  calls `open` from `create_account`/`unlock`, `close` from `lock`.
- **`NullVault`** — a trivial no-op `VaultBackend` so `SessionManager::new` (the two-arg
  W2 constructor used by every existing `session_w2.rs` test) keeps working unmodified.
  W3 adds `SessionManager::new_with_vault(keystore, accounts, vault)` alongside it rather
  than breaking the old signature.
- **`SqlCipherVault`** — the real backend. `open()`:
  1. `Connection::open(path)` (creates the file if absent).
  2. `PRAGMA key = "x'<64 lowercase hex>'"` — raw 256-bit key form, never a passphrase
     string (architecture §3.1's explicit warning: a passphrase string would additionally
     run SQLCipher's ~256k-iteration PBKDF2 on top of HKDF and blow the unlock budget).
     `PRAGMA key` alone cannot fail on a wrong key — SQLCipher only detects a mismatch on
     the first real read — so `open()` immediately follows it with a `SELECT count(*) FROM
     sqlite_master` and maps that failure to `VaultError::WrongKey`.
  3. `ensure_schema()` — the data-model §7 DDL verbatim, every `CREATE TABLE`/`CREATE
     INDEX` as `IF NOT EXISTS`, so reopening an already-initialized file (the normal
     lock→unlock path) is a no-op rather than an error.
- **`AccountStore` for `SqlCipherVault`** — `crate::account`'s module doc named this
  exactly: "W3 swaps in a SQLCipher-backed one without touching `SessionManager`." Backed
  by the same `Mutex<Option<Connection>>` the `VaultBackend` half uses, so one
  `Arc<SqlCipherVault>` coerced to both trait objects shares one live connection instead of
  two independently-lifecycled ones. `store()` deletes-then-inserts (v1 holds at most one
  account row, per architecture §7); `load()`/`delete()` are direct queries.

### `core/src/session.rs` — the integration seam

- `SessionManager` gained a `vault: Arc<dyn VaultBackend>` field.
- `create_account`: opens the vault (fresh file) **before** writing the account or
  keystore item, so a vault-open failure leaves nothing to roll back; a downstream
  account-store or keystore-store failure now also closes the vault on its rollback path,
  matching the existing "leave a clean `first_run` on failure" contract.
- `unlock`: opens the vault on the just-recovered `master.sqlcipher_key()` immediately
  after passphrase verification succeeds, before `verify_integrity_on_unlock` (W5's future
  extension point). A vault-open failure at this point is `internal`, not `unlock_failed`
  — the passphrase was already proven correct, so folding a corrupt/desynced DB into the
  same error as a wrong passphrase would misreport the failure class.
- `lock`: closes the vault immediately after dropping `OpenSession`, so "close the DB" from
  architecture §3.3's lock contract is now a real effect, not a comment.

## Problems Encountered

1. **Where does `AccountStore` get its `Connection`?** The trait was designed in W2 as a
   swappable seam, but its methods take `&self`, not a connection — and the connection
   doesn't exist until `open()` runs, inside `session.rs`, after `AccountStore` has already
   been constructed and handed to `SessionManager::new`. Resolved by having
   `SqlCipherVault` implement *both* traits over one shared `Mutex<Option<Connection>>`:
   the caller constructs one `Arc<SqlCipherVault>`, passes clones of it as `accounts` and
   as `vault`, and `SessionManager`'s `self.vault.open(...)` populates the connection that
   `self.accounts.load()/store()` then reads through — with no coupling between the two
   trait implementations beyond the shared field.
2. **Reopening an idempotent-schema database vs. re-keying.** `SqlCipherVault::open()` is
   a no-op if a connection is already held (this process never needs to re-key a live
   connection — `change_passphrase` rotates the KEK, not the DB key, and never touches the
   vault). A second, independent `SqlCipherVault` instance pointed at the same file (the
   "process restart" test) still goes through the full open path, so the wrong-key and
   schema-idempotency properties are both exercised for real, not bypassed by the
   already-open short-circuit.
3. **`PRAGMA key` never fails, even on the wrong key.** SQLCipher only surfaces a bad key
   on the first attempted read of encrypted content, not at `PRAGMA key` time. Missing this
   would have made `open()` silently "succeed" against a wrong key and only fail later,
   confusingly, on whatever the first real query happened to be. `verify_key()` makes the
   check explicit and immediate: a `SELECT count(*) FROM sqlite_master` right after the
   pragma, mapped to `VaultError::WrongKey`.
4. **`LocalAccount.created_at` is an RFC 3339 string; the SQL column is
   `created_at_unix_ms`.** `crate::account` has no public parser (only
   `format_rfc3339(unix_secs) -> String`, the write direction used by `now_rfc3339()`).
   Rather than add a second public time API to `crate::account` for one caller, `vault.rs`
   owns a private inverse (`rfc3339_to_unix_ms`, Howard Hinnant's `days_from_civil`, the
   textbook inverse of `civil_from_days`) with its own round-trip unit test against the
   same known values `account.rs`'s `rfc3339_known_values` test uses.

## Resolution

- `cargo test -p pg-core` green: **9/9** new tests in `core/tests/vault_w3.rs`, all 53 of
  W2's `session_w2.rs` tests still green **unmodified** (no existing test file was edited
  to accommodate W3 — the `SessionManager::new` two-arg constructor is untouched), W1's 35,
  plus lib unit tests (including a new one for the RFC 3339 ↔ Unix-ms round trip).
- Full-workspace `cargo test` (both `pg-core` and `privacy-gate`/`src-tauri`) and `npm run
  check` both re-confirmed green in the dev container — closing the "not re-confirmed" gap
  dev-log 0012 left open after the host-disk incident. `rusqlite`'s `bundled-sqlcipher`
  feature (vendored SQLCipher + OpenSSL, compiled from source) built cleanly against the
  container's existing `build-essential`/`libssl-dev`/`pkg-config`.
- `cargo clippy -p pg-core --all-targets`: zero warnings in `vault.rs` or `vault_w3.rs`.
  (Pre-existing warnings in `session_w2.rs`'s `record!` test macro predate this chunk and
  are unrelated to it.)
- dev-plan W3 "Tests first" line, verified: create vault on first account
  (`create_account_creates_the_vault_file_and_schema`); reopen after lock/unlock
  (`account_persists_across_lock_and_unlock_in_the_same_process`, plus the stronger
  process-restart form, `account_survives_a_fresh_session_manager_over_the_same_files`,
  which closes the exact gap dev-log 0012 problem #1 flagged for W2's
  `InMemoryAccountStore`); stolen file without the wrap key cannot query
  (`stolen_file_without_the_correct_key_cannot_be_queried`); schema_version = 1
  (`fresh_vault_has_schema_version_1`).
- testing.md §5.3 gated-module property ("SQLCipher opened with raw key form, not
  passphrase KDF") has a dedicated test, `raw_key_bytes_round_trip_exactly`, using
  non-ASCII-printable key bytes so a passphrase-string code path would behave differently
  than the raw hex-blob path.
- Scope held: no artifact/document/variant writes, no audit chain, no `rusqlite` calls
  outside `vault.rs`, no new command name, no Tauri IPC, no UI.
- Module to add to the W38 mutation-gate list alongside W1's and W2's:
  `core/src/vault.rs`'s `open_raw_key_pragma`/`verify_key` pair (testing.md §5.3 names this
  exact property).
## Review (roster-mandated Opus pass)

Verdict: **REQUEST CHANGES** on the first submission — 2 blocking issues, both
empirically demonstrated (a live mutant that survived the full suite, and a runnable
probe reproducing a stuck state), plus 6 non-blocking nits. Both blocking issues are
fixed below; the diff was re-verified green after fixing.

**Blocking 1 — the gated-module property (raw key, not passphrase KDF) was not actually
tested.** `raw_key_bytes_round_trip_exactly`'s doc comment claimed a passphrase-string
swap would fail it; the reviewer mutated `open_raw_key_pragma` to drop the `x'...'`
wrapper (i.e. feed SQLCipher the 64 hex characters as a passphrase, the exact bug
architecture §3.1 warns against) and the full 9/9 suite still passed. The bug: every test
in the file only ever reads back through its own `open()`, so any self-consistent key
transform round-trips regardless of which PRAGMA form was used. Fixed by adding
`open_uses_raw_key_form_not_passphrase_kdf` (`core/src/vault.rs` `mod tests`), which opens
a file `SqlCipherVault` created through *both* PRAGMA forms independently via a bare
`rusqlite::Connection` and asserts the raw form succeeds **and** the passphrase form
fails — the second assertion is what actually kills the mutant. Also added
`key_pragma_hex_is_lowercase` for the residual `{:02x}`→`{:02X}` mutant the reviewer
flagged (SQLCipher parses hex case-insensitively, so this needed an explicit assertion
rather than relying on architecture §3.1's prose alone), and corrected the misleading doc
comment on the original round-trip test to point at the new one.

**Blocking 2 — an aborted `create_account` permanently wedged the vault path.**
`create_account` opened (and thereby created) the vault file *before* writing the
keystore item, but its rollback paths only called `vault.close()`, not delete. On retry,
`state()` was still `first_run` (correct — the keystore was never written), so
`create_account` generated a **fresh** master key and tried to open the *orphaned* file
from the failed attempt, which the new key cannot decrypt → `VaultError::WrongKey` →
`internal`, forever, with no in-app recovery path. Reachable by any real crash between
vault-open and keystore-write, not just an injected fault. Fixed by adding
`VaultBackend::destroy()` (close + remove the file, idempotent) and using it instead of
`close()` on every `create_account` rollback path, plus an unconditional `destroy()`
immediately before the fresh `open()`: `first_run` is answered by the keystore alone, so
reaching that line already proves no keystore item exists anywhere, which means nothing
at the vault path can ever be recovered by any means (architecture §3.1) — a file found
there is provably an orphan, never live data, so it is safe to clear unconditionally
rather than try to distinguish "pre-existing" from "created this call." `unlock`'s vault
open failures were deliberately left as plain `internal` (not `destroy()`d) — `unlock`
only runs when a keystore item genuinely exists, so a vault-open failure there is a real
desync worth investigating, not an orphan. Regression test:
`create_account_retries_cleanly_after_the_keystore_write_fails`, using
`InMemoryKeystore::fail_next_store()` to force exactly the failure window, across two
independent `SessionManager`/`SqlCipherVault` instances to simulate a UI-driven retry.

**Nits fixed:** lock-poisoning recovery in `close()`/`is_open()` (`unwrap_or_else(|e|
e.into_inner())` instead of silently no-op-ing / reporting the safe-looking `false`);
key hex material now built into a `Zeroizing<String>` via direct nibble lookup instead of
32 unwiped `format!` allocations; `AccountStore::store()` wrapped in a real
`Connection::transaction()` instead of an unguarded delete-then-insert; `VaultError`'s
real class now propagates into `AccountStoreError` instead of being collapsed to a fixed
`"vault not open"` string; `VaultBackend::open`'s doc corrected to say a key is not
re-checked while already open (previously promised a check that didn't exist — unreachable
in practice today since `lock` always closes first, but a real doc/code mismatch).
`schema_has_every_data_model_7_table_index_and_foreign_keys_on` added per the reviewer's
observation that only `schema_version` was ever asserted, not the schema itself.

Net new tests from the review pass: 4 (`key_pragma_hex_is_lowercase`,
`open_uses_raw_key_form_not_passphrase_kdf`, and
`schema_has_every_data_model_7_table_index_and_foreign_keys_on` in `core/src/vault.rs`'s
unit tests; `create_account_retries_cleanly_after_the_keystore_write_fails` in
`core/tests/vault_w3.rs`) — **104 passed, 1 ignored** across `pg-core` (35 W1 + 53 W2 + 10
W3 integration + 6 lib unit, all green), up from 100 passed before this review pass.
Clippy clean on every touched file; full workspace `cargo test` and `npm run check`
re-confirmed.

Next: W4 — session gating table (api.md §2 command-availability matrix as a table-driven
test), then W5 — audit chain and integrity, which is the next consumer of
`SessionManager::audit_mac_key()` and the first thing to write into `vault.rs`'s
`audit_entry` table.

## Related Documentation

- [Development Plan — W3 specification](../dev-plan.md#w3--empty-vault-sqlcipher)
- [Agent roster — W3](../agent-roster.md)
- [Spec — Architecture §4 (storage, SQLCipher), §2.4 (stolen-file threat model), §3.1 (raw-key opening)](../specs/architecture.md)
- [Spec — Data model §7 (schema v1), §5.6 (`LocalAccount`)](../specs/data-model.md)
- [Spec — Testing §5.3 (gated module: SQLCipher raw key)](../specs/testing.md)
- [Dev log 0012 — W2 account, keystore, session](./0012-w2-account-keystore-session.md)

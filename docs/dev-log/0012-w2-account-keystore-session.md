# [0012] W2 — Account, keystore, session

- **Status:** Complete (core green; full-workspace `make test` blocked by a host-disk
  failure — see Problems Encountered #5)
- **Date:** 2026-08-23

## Objective

Deliver the six session and account commands of [api.md §5.1](../specs/api.md) —
`get_session_state`, `create_account`, `unlock`, `lock`, `change_passphrase`,
`get_account` — as in-process functions on top of W1's envelope primitives, plus the
`KeystoreItem` and the backends that hold it ([architecture §3.2](../specs/architecture.md)),
the Argon2id passphrase → `wrap_key` step (§3.1), and the local-only `LocalAccount` (§7).

Explicitly **not** in this chunk (dev-plan.md W2 "Do not: integrity report, documents,
UI"): `get_integrity_report` and audit-chain verification (W5), the SQLCipher vault (W3),
Secret-Service probing / backend selection (W7), any document / approval / share / config /
variant command, Tauri IPC (W29), and any screen.

## Implementation

### `core/src/api.rs` — the error model (api.md §3)

`ErrorCode` carries the **whole** api.md §3 list, not just the six codes W2 produces, so
later chunks do not fork a second list. `ApiError { code, message }` is constructed only
through helpers that take a `&'static str` class — there is no `format!` site anywhere in
the core that interpolates caller input into a message. That is what turns C-API-1
("passphrase … never appear[s] in outputs") into a structural property rather than a
review checklist.

### `core/src/keys.rs` — Key Manager material (architecture §3.1)

The two things W1 deliberately left out:

- `derive_wrap_key(passphrase: &str, &Argon2idParams) -> Zeroizing<[u8; 32]>` — Argon2id
  **v1.3** (`Version::V0x13`, named explicitly; Argon2i/Argon2d are not interchangeable),
  with the cost parameters read from the stored item rather than hardcoded at the call
  site.
- `VaultMasterKey`, a newtype over W1's `Dek` (so it inherits `ZeroizeOnDrop`, the
  redacting `Debug`, and the deliberate absence of `Clone`), with
  `sqlcipher_key()` = `HKDF(master, "pg-db-v1")` and
  `audit_mac_key()` = `HKDF(master, "pg-audit-mac-v1")`, both returned as `Zeroizing`.
- `wrap_master_key` / `unwrap_master_key`, bound to `Aad::global(WrappedMaster, 1)` —
  AAD kind 6, per data-model §5.9.

No struct in this module has a passphrase field, so there is no `Debug` that could leak
one. `unwrap_master_key` returns `Option`, collapsing wrong passphrase, tampered blob and
unusable stored parameters into one indistinguishable `None` (architecture §3.3:
"Passphrase failure zeroizes and refuses (no partial open)").

### `core/src/keystore/` — the `KeystoreItem` and its backends (architecture §3.2)

- **`mod.rs`** — `Argon2idParams { m_cost, t_cost, p_cost, salt }`, `AuditHead
  { sequence, head_hash }`, `KeystoreItem { account_id, kdf, wrapped_master_key,
  audit_head }`, exactly data-model §5.9. Plus the `KeystoreBackend` trait,
  `KeystoreBackendKind` (architecture §3.2 requires the Key Manager to *record* which
  backend is in use) and `KeystoreError`.
- **`memory.rs`** — `InMemoryKeystore`, the mock dev-plan W2 asks for, with injectable
  store/load failures so the rollback and fail-safe paths are testable. It round-trips
  through the real codec, so the mock is not a shortcut past the encoding.
- **`file.rs`** — `FileKeystore`, the Linux fallback: `0600` from the moment the temp file
  exists (`OpenOptions::mode`, not a create-then-chmod window another local user could
  slip through), `write_all` → `sync_all` → `rename` → best-effort directory `fsync`.
- **`os.rs`** — `OsKeystore` over the `keyring` crate.

`Argon2idParams::OWASP_FLOOR` = `m_cost 19456 KiB (19 MiB)`, `t_cost 2`, `p_cost 1` —
**OWASP minimum current at implementation** for Argon2id with a 32-byte output, which is
the floor architecture §3.1 names. `CURRENT` (what new accounts get) is currently exactly
the floor and is the single knob to raise. Because the parameters travel *inside* the
`KeystoreItem`, raising it later never locks an existing vault out. W2 deliberately does
not assert a wall-clock number against design.md §7's ≤ 1 s unlock budget: a headless
container on shared CI hardware is not "the mainstream laptop of design.md §7", and a
timing assertion there would be a flaky test pretending to be a performance gate. The
test asserts the floor is met; the tuning pass belongs with real hardware.

Two design points worth recording:

1. **The keystore slot is not keyed by `account_id`.** The keystore is the only thing
   readable while locked — `LocalAccount` lives in the SQLCipher DB (data-model §5.6),
   which cannot be opened without the key the keystore holds. So `first_run` vs `locked`
   has to be answerable from a fixed, well-known slot, and `account_id` is a *field of*
   the item rather than a lookup key for it. This matches architecture §3.2 ("a single
   `KeystoreItem` per local account") and §7 (v1 accounts are local-only and singular).
2. **`Ok(None)` from a backend means "there is genuinely no account", and nothing else.**
   Every read failure returns `Err`. A corrupt item is `KeystoreError::Corrupt`, never
   "no account". Reporting either as `first_run` would offer a first-run flow over a live
   vault and overwrite the only copy of the wrapped master key — the same bricking class
   decision 0004's dev-log flagged for the crash window.

### `core/src/account.rs` — `LocalAccount` (architecture §7, data-model §5.6)

`LocalAccount { id, display_name, created_at }` behind an `AccountStore` trait with a
process-local implementation. data-model §5.6 puts this record in the SQLCipher vault, and
**W3 owns that database**, so W2 keeps it behind the trait and W3 swaps the implementation
without touching `SessionManager`. That is also why api.md §2 makes `get_account`
`unlocked`-only: the record is inside the vault.

`new_account_id()` is a UUID v4 from the OS CSPRNG — nothing derived from hostname, MAC or
a counter, which architecture §7 forbids. `format_rfc3339` is a hand-rolled
`civil_from_days` conversion (Howard Hinnant's) rather than a date-time dependency: v1
needs one format, UTC, second resolution. Unit-tested against known values including a
leap day and a century boundary.

### `core/src/session.rs` — the six commands (api.md §2, §5.1)

`SessionManager { keystore, accounts, open: Option<OpenSession> }`. In/Out DTOs use
api.md §5.1's exact field names (`display_name`, `passphrase`, `account_id`, `state`,
`current`, `new_passphrase`, `ok`, `integrity`, `created_at`).

- **`SessionState`** has all four api.md §2 variants. `DegradedIntegrity` is unreachable in
  W2 by construction — nothing constructs it — and is documented as W5's, so wiring it in
  later is not a breaking type change.
- **`verify_integrity_on_unlock`** is the named extension point. Its doc comment restates
  architecture §6.3's three outcomes (clean / crash-window fast-forward / integrity
  failure) and says in as many words that returning `None` is a *missing step*, not a
  decision that integrity checking is unnecessary. `IntegrityReport` is declared with
  api.md §5.1's shape but W2 never constructs one, and `get_integrity_report` does not
  exist as a function.
- **`create_account`** writes the account record *first* and the `KeystoreItem` *last*,
  because the keystore item is what flips `first_run` → `locked`: the session is only ever
  advertised as having an account once everything it needs is present. If the keystore
  write fails, the account record is rolled back so a retry sees a clean `first_run`.
- **`change_passphrase`** validates the new passphrase *before* testing the current one, so
  a rejected new passphrase is never an oracle for the current one; verifies `current`
  against the **stored item** (not the live session — holding an open session is not proof
  of knowing the passphrase); constant-time-compares the recovered master against the
  session's; generates a fresh salt; re-wraps the same `vault_master_key`; and carries
  `audit_head` across untouched, since resetting it would silently defeat W5's
  anti-truncation check (architecture §6.2).

## Problems Encountered

1. **`get_account` needs a record that W2 has nowhere to put.** data-model §5.6 says
   `LocalAccount` is SQLCipher-only, and W3 owns SQLCipher. Inventing a plaintext
   account file would have been a new on-disk format nobody specified — and a new
   plaintext-to-disk path, which dev-plan §3's definition of done forbids. Resolved with
   the `AccountStore` trait plus a process-local implementation, so W3 supplies the real
   one. Consequence to be aware of in W3: in W2 the record does not survive process
   restart, so a `SessionManager` built over a persistent keystore in a fresh process
   unlocks correctly (the master key comes from the keystore) but `get_account` reports
   `internal`. That is a W2-only gap, not a behaviour to preserve.

2. **A first attempt at the anti-enumeration test asserted the wrong thing.** It deleted
   the `KeystoreItem` while locked and expected `unlock_failed`. But with no item the
   state genuinely *is* `first_run`, so api.md §2's availability table makes the correct
   answer `not_in_session` — and api.md §3 explicitly permits that much: "no
   account-enumeration **beyond `first_run` vs `locked`, which is visible from
   `get_session_state`**". Rewritten to assert what the spec actually promises (no
   distinct `not_found`-style code), and the genuine read-failure branch is now covered
   separately by injecting a backend failure, which also pins the fail-safe-to-`locked`
   behaviour and the refusal of `create_account` while the keystore is unreadable. Two
   real properties came out of one bad assertion.

3. **`keyring` 4.x is a different crate from the 2.x/3.x most examples show.** It is now a
   thin facade over `keyring-core` and needs a mode feature; the default `v1` feature is
   the cross-platform `Entry::new` / `get_secret` / `set_secret` API and pulls the
   pure-Rust `zbus` Secret Service store, so it compiles in the container without a native
   D-Bus library. Verified against the vendored crate sources inside the container rather
   than guessing. `Entry::store_status()` is what W7 will probe with.

4. **Adding serde derives to W1's `WrappedBlob` was the tempting shortcut.** Rejected: the
   public types belong to the data model, and the on-keystore encoding is a storage concern
   that has to be versionable independently. The keystore serializes through a private
   mirror struct with hex blobs and a `v` field, and W1's `core/src/crypto/` is untouched
   by this chunk.

5. **The host machine ran out of disk during the full-workspace `make test`.** The
   `pg-core` suite is green (output below, 53/53, captured from a completed run). The
   subsequent whole-workspace `cargo test` — which additionally compiles `src-tauri`'s
   Tauri/GTK dependency tree — exhausted the host volume (`/System/Volumes/Data` at 100%,
   ~125 MB free), which produced cascading `Input/output error` failures inside the
   container and then took Docker Desktop's daemon down with it. This is an environment
   failure with no relationship to W2's code: nothing in this chunk is reachable from
   `src-tauri`, whose `main.rs` is still W0's stub. **Action required before the next
   chunk:** free host disk, restart Docker Desktop, and re-run `make test` to confirm the
   `privacy-gate` target too. W3 (SQLCipher, bundled, compiled from source) will need
   materially more disk than W2 did.

## Resolution

- `cargo test -p pg-core` green: **53/53** in `core/tests/session_w2.rs`, plus W1's 35 in
  `core/tests/crypto_w1.rs` and the in-module unit tests, with the OS-keystore smoke test
  correctly reported as ignored. No new warnings.
- Full-workspace `make test` **not re-confirmed** — see Problems Encountered #5.
- The "lock zeroizes session key material" property is asserted **structurally**, which is
  the stronger form: `lock()` drops the whole `OpenSession`, so `SessionManager` has no
  field holding key material afterwards, and `has_resident_key_material()` is `false`
  because the `Option` is `None` — not because bytes were overwritten in a struct that
  still exists. `VaultMasterKey` is `ZeroizeOnDrop` underneath, so the bytes go too, but
  the test does not depend on that. The behavioural half is asserted through the commands,
  per dev-plan W2's "assert via subsequent decrypt/command failure, not log lines":
  `sqlcipher_key()` and `audit_mac_key()` return `not_in_session` after `lock`, and a blob
  sealed before the lock is unopenable through the session until a correct `unlock`
  restores the same master key. This also holds after a *failed* unlock (no partial open).
- Scope held: no `get_integrity_report`, no SQLCipher, no `rusqlite`, no document /
  approval / share / config / variant command, no Tauri command, no UI, no new command
  name outside api.md §5.1.
- Real-backend coverage: `OsKeystore` has an `#[ignore]`d smoke test on a throwaway slot,
  runnable by hand on a desktop (`cargo test -p pg-core -- --ignored os_keystore_smoke`).
  dev-plan W2 asks for a real backend "when practical", and it is not practical here — the
  dev environment is a headless Linux container (CONTRIBUTING.md) with no Keychain, no
  Credential Manager and no D-Bus session bus. The `KeystoreBackend` trait is what makes
  that split honest: `SessionManager` cannot tell the three implementations apart, so
  everything above the seam is fully covered through the mock.
- Modules to list for the W38 mutation gate alongside W1's: `core/src/session.rs` (the
  state machine and validation floors) and `core/src/keys.rs` (the wrap/unwrap path).

Next: W3 — empty vault (SQLCipher), which consumes `SessionManager::sqlcipher_key()` and
replaces `InMemoryAccountStore` with the real `LocalAccount` row.

## Related Documentation

- [Development Plan — W2 specification](../dev-plan.md#w2--account-keystore-session)
- [Spec — API §2 (session model), §3 (error model), §5.1 (commands), §9 (C-API-1)](../specs/api.md)
- [Spec — Architecture §3.1–§3.4 (key hierarchy, key storage, unlock/lock, first-run), §6.3 (verification), §7 (account)](../specs/architecture.md)
- [Spec — Data model §5.6 (`LocalAccount`), §5.9 (`KeystoreItem`)](../specs/data-model.md)
- [Spec — Design §7 (≤ 1 s unlock budget)](../specs/design.md)
- [Decision 0003 — v1 tech stack (OS keystore)](../decisions/0003-v1-tech-stack.md)
- [Decision 0004 — v1 architecture (crash window, two-phase commit)](../decisions/0004-v1-architecture.md)
- [Decision 0006 — TDD and mutation testing](../decisions/0006-tdd-and-mutation-testing.md)
- [Dev log 0011 — W1 envelope crypto](./0011-w1-envelope-crypto.md)

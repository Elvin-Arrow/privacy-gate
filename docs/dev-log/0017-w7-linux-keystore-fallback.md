# [0017] W7 — Linux keystore fallback

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Deliver the piece W2 explicitly deferred: choosing between `OsKeystore` and `FileKeystore`
by probing whether a platform credential store is actually usable (architecture §3.2). Both
backends themselves — file mechanics, round-trip, corrupt-content handling — already
shipped in W2 (`core/src/keystore/file.rs`, `os.rs`); this chunk is purely the selection
function and proving the switch doesn't weaken anything `SessionManager` already
guarantees.

Explicitly **not** in this chunk (dev-plan.md W7 "Do not: change the threat model to claim
coordinated rollback detection"): nothing here claims that a stolen `vault.db` **and** the
fallback file, rolled back together, is detectable — that residual gap is already documented
in architecture §2.4/§3.2 and stays exactly as documented.

Per the [agent roster](../agent-roster.md), W7 is Sonnet tier, no mandatory second review
("Platform-integration work, contained blast radius").

## Implementation

### `core/src/keystore/mod.rs` — `select_backend` / `select_backend_with`

- `select_backend(app_data_dir) -> Arc<dyn KeystoreBackend>` — the production entry point:
  probes `OsKeystore::is_available()` and returns `OsKeystore` or `FileKeystore` at
  `app_data_dir/keystore.json` (`FALLBACK_FILE_NAME`, already a W2 constant).
- `select_backend_with(is_os_keystore_available, app_data_dir)` — the same logic with the
  probe injected as a closure. Necessary, not just nice-to-have: the dev container
  (`CONTRIBUTING.md`) has no D-Bus session bus, so `OsKeystore::is_available()` always
  returns `false` here — without the injectable seam, the "OS keystore selected" branch
  would be structurally untestable in this environment, the same problem W2's `os.rs`
  already solved for the backend's own I/O by `#[ignore]`-ing its real-backend test.

Both `file::FileKeystore` and `FALLBACK_FILE_NAME` are unchanged; `FALLBACK_FILE_NAME` is
now re-exported from `crate::keystore` (it was previously only reachable via the private
`file` submodule) so `select_backend` and this chunk's tests can both name it.

## Resolution

- `cargo test -p pg-core` green: **6/6** new in `keystore_fallback_w7.rs`, all prior tests
  (W1 through W6, 151 total) unmodified and green.
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings on every file this chunk touches.
- dev-plan W7 "Tests first" line, verified: fallback backend reported
  (`selects_the_file_fallback_when_the_os_keystore_is_unavailable`, plus the OS-keystore
  branch for symmetry and `the_fallback_file_lands_under_the_given_app_data_dir` proving
  the selected backend actually writes where it claims to); wrong passphrase still fails on
  the fallback backend specifically, not just FileKeystore in isolation
  (`wrong_passphrase_still_fails_on_the_fallback_backend`); stolen dir without passphrase
  cannot decrypt (`stolen_fallback_file_cannot_be_decrypted_without_the_passphrase`, plus a
  belt-and-braces raw-bytes grep for the passphrase itself, architecture §3.2: "The
  passphrase is never written to disk").
- Scope held: no coordinated-rollback detection claim, no new keystore backend, no
  changes to `FileKeystore`/`OsKeystore` themselves — W7 only adds the function that
  chooses between them.
- Not yet wired to a real "app-data directory" concept — that's W29 (Tauri IPC), the first
  chunk that has an actual OS app-data path to hand `select_backend`. This chunk's tests
  all use a temp dir standing in for it, same as every prior chunk's vault tests.

Next: W8 — Import plain text.

## Related Documentation

- [Development Plan — W7 specification](../dev-plan.md#w7--linux-keystore-fallback)
- [Agent roster — W7](../agent-roster.md)
- [Spec — Architecture §3.2 (key storage, Linux fallback)](../specs/architecture.md)
- [Spec — Testing §6.5 (AC-5), §8 "Linux fallback" row](../specs/testing.md)
- [Dev log 0012 — W2 account, keystore, session (the two backends themselves)](./0012-w2-account-keystore-session.md)
- [Dev log 0016 — W6 retention config](./0016-w6-retention-config.md)

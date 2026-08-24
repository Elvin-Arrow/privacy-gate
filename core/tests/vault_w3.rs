//! W3 — Empty vault (SQLCipher).
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §4.2 (one SQLCipher database, raw-key open, no
//!   plaintext document columns)
//! - `docs/specs/data-model.md` §7 (schema version 1, table DDL, `schema_meta`)
//! - `docs/specs/testing.md` "SQLCipher raw key" row (§ open uses `x'<64 hex>'` /
//!   `sqlite3_key_v2`, not a passphrase-KDF path) and the gated-module list (§5.3:
//!   "SQLCipher opened with raw key form (not passphrase KDF)")
//! - `docs/dev-plan.md` W3 ("Tests first: create vault on first account; reopen after
//!   lock/unlock; stolen file without wrap key cannot query (prelude to AC-5); schema_version
//!   = 1")
//!
//! Out of W3 scope and deliberately absent here: import, audit HMAC (W5), detector.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{CreateAccountIn, SessionManager, UnlockIn};
use pg_core::vault::{SqlCipherVault, VaultBackend, VaultError, SCHEMA_VERSION};

const PASSPHRASE: &str = "correct horse battery staple";

/// A fresh temp-dir path for a vault DB file. `tempfile::TempDir` deletes on drop, so the
/// caller must keep it alive for the duration of the test.
fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    (dir, path)
}

/// A `SessionManager` wired to a real `SqlCipherVault` at a temp-dir path, sharing the
/// same object as both the `AccountStore` and the `VaultBackend` (architecture §4.2: one
/// database; dev-log 0012 problem #1: W3 supplies the real `AccountStore`).
fn fresh_with_vault() -> (SessionManager, Arc<InMemoryKeystore>, tempfile::TempDir, std::path::PathBuf) {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path.clone()));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault;
    let mgr = SessionManager::new_with_vault(keystore.clone(), accounts, backend);
    (mgr, keystore, dir, path)
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

// ---------------------------------------------------------------------------
// dev-plan W3: "create vault on first account"
// ---------------------------------------------------------------------------

#[test]
fn create_account_creates_the_vault_file_and_schema() {
    let (mut mgr, _ks, _dir, path) = fresh_with_vault();
    assert!(!path.exists(), "vault file must not exist before create_account");

    mgr.create_account(create_in())
        .expect("first-run create_account must succeed");

    assert!(path.exists(), "create_account must create the vault DB file");
}

/// dev-plan W3: "schema_version = 1".
#[test]
fn fresh_vault_has_schema_version_1() {
    let (mut mgr, _ks, _dir, path) = fresh_with_vault();
    mgr.create_account(create_in()).expect("create_account");

    // Open a second, independent connection with the same key material to read the
    // schema_meta row without going through SessionManager (component-level check).
    let key = mgr.sqlcipher_key().expect("session must be unlocked");
    let raw = SqlCipherVault::new(path);
    raw.open(&key).expect("reopen with the correct key");
    assert_eq!(raw.schema_version().expect("schema_version readable"), SCHEMA_VERSION);
}

/// dev-plan W3: "Integrate: create_account / unlock open the DB".
#[test]
fn create_account_leaves_the_vault_open() {
    let (mut mgr, _ks, _dir, path) = fresh_with_vault();
    mgr.create_account(create_in()).expect("create_account");
    // Observable via a command that needs the open DB: get_account must succeed.
    let out = mgr.get_account().expect("get_account while unlocked");
    assert_eq!(out.display_name, "Alex");
    drop(path);
}

// ---------------------------------------------------------------------------
// dev-plan W3: "reopen after lock/unlock"
// ---------------------------------------------------------------------------

#[test]
fn account_persists_across_lock_and_unlock_in_the_same_process() {
    let (mut mgr, _ks, _dir, _path) = fresh_with_vault();
    mgr.create_account(create_in()).expect("create_account");
    mgr.lock().expect("lock");

    mgr.unlock(UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock with the correct passphrase");

    let out = mgr.get_account().expect("get_account after unlock");
    assert_eq!(out.display_name, "Alex");
}

/// The harder version of the same requirement: a **new** `SessionManager` (simulating a
/// process restart) over the same keystore backend and the same vault file must recover
/// the account. This is exactly the W2 gap dev-log 0012 problem #1 flagged: "in W2 the
/// record does not survive process restart... That is a W2-only gap, not a behaviour to
/// preserve."
#[test]
fn account_survives_a_fresh_session_manager_over_the_same_files() {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());

    {
        let vault = Arc::new(SqlCipherVault::new(path.clone()));
        let accounts: Arc<dyn AccountStore> = vault.clone();
        let backend: Arc<dyn VaultBackend> = vault;
        let mut mgr = SessionManager::new_with_vault(keystore.clone(), accounts, backend);
        mgr.create_account(create_in()).expect("create_account");
        mgr.lock().expect("lock");
    }

    // Fresh SessionManager, fresh SqlCipherVault instance, same keystore + same DB file.
    let vault2 = Arc::new(SqlCipherVault::new(path.clone()));
    let accounts2: Arc<dyn AccountStore> = vault2.clone();
    let backend2: Arc<dyn VaultBackend> = vault2;
    let mut mgr2 = SessionManager::new_with_vault(keystore, accounts2, backend2);

    mgr2.unlock(UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock in the new process-like session");

    let out = mgr2.get_account().expect("get_account after restart-like unlock");
    assert_eq!(out.display_name, "Alex");
    assert!(path.exists());
    drop(dir);
}

#[test]
fn lock_closes_the_vault() {
    let (mut mgr, _ks, _dir, _path) = fresh_with_vault();
    mgr.create_account(create_in()).expect("create_account");
    mgr.lock().expect("lock");

    // get_account is unlocked-only regardless of vault state, but the structural claim
    // this test protects is that the DB handle itself is gone, not merely that the
    // session-layer gate rejects the call. Assert both: the api-level gate fires, and
    // sqlcipher_key (which only exists on an open session) is also refused.
    assert!(mgr.get_account().is_err());
    assert!(mgr.sqlcipher_key().is_err());
}

// ---------------------------------------------------------------------------
// dev-plan W3: "stolen file without wrap key cannot query (prelude to AC-5)"
// ---------------------------------------------------------------------------

#[test]
fn stolen_file_without_the_correct_key_cannot_be_queried() {
    let (dir, path) = temp_db_path();
    {
        let vault = Arc::new(SqlCipherVault::new(path.clone()));
        vault
            .open(&zeroize::Zeroizing::new([0x11u8; 32]))
            .expect("create with key A");
        vault.close();
    }

    // Attacker has the file (`path`) but not the key. A wrong 32-byte key must fail to
    // open the database for querying — not silently return an empty/garbage result.
    let stolen = SqlCipherVault::new(path.clone());
    let result = stolen.open(&zeroize::Zeroizing::new([0x22u8; 32]));
    assert_eq!(result, Err(VaultError::WrongKey));
    assert!(!stolen.is_open());

    drop(dir);
}

/// The correct key on the same file must still work — the wrong-key test above is not
/// merely detecting "any reopen fails."
#[test]
fn same_file_reopens_with_the_same_key() {
    let (dir, path) = temp_db_path();
    let key = zeroize::Zeroizing::new([0x33u8; 32]);
    {
        let vault = Arc::new(SqlCipherVault::new(path.clone()));
        vault.open(&key).expect("create with key");
        vault.close();
    }

    let reopened = SqlCipherVault::new(path.clone());
    reopened.open(&key).expect("reopen with the same key must succeed");
    assert!(reopened.is_open());

    drop(dir);
}

// ---------------------------------------------------------------------------
// Opus review (dev-log 0013) blocking issue #2: an aborted create_account must not
// permanently wedge the vault path. `first_run` guarantees no keystore item exists, so
// nothing left at the vault path by an aborted attempt is ever recoverable data — the next
// attempt must succeed against a clean file, not fail forever with `internal`.
// ---------------------------------------------------------------------------

#[test]
fn create_account_retries_cleanly_after_the_keystore_write_fails() {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());

    // First attempt: vault gets created, then the keystore write is made to fail.
    keystore.fail_next_store();
    {
        let vault = Arc::new(SqlCipherVault::new(path.clone()));
        let accounts: Arc<dyn AccountStore> = vault.clone();
        let backend: Arc<dyn VaultBackend> = vault;
        let mut mgr = SessionManager::new_with_vault(keystore.clone(), accounts, backend);
        let result = mgr.create_account(create_in());
        assert!(result.is_err(), "the injected keystore failure must surface");
    }

    // Second attempt, fresh SessionManager and fresh SqlCipherVault instance over the same
    // path and the same (still first_run) keystore — exactly what a UI retry looks like.
    // Before the fix this failed forever with `internal` because the orphaned vault file
    // from the first attempt was encrypted under a master key that no longer exists
    // anywhere, and the second attempt generates a *different* fresh master key.
    let vault2 = Arc::new(SqlCipherVault::new(path.clone()));
    let accounts2: Arc<dyn AccountStore> = vault2.clone();
    let backend2: Arc<dyn VaultBackend> = vault2;
    let mut mgr2 = SessionManager::new_with_vault(keystore, accounts2, backend2);
    mgr2.create_account(create_in())
        .expect("retry after a rolled-back create_account must succeed, not wedge forever");

    let out = mgr2.get_account().expect("get_account after the successful retry");
    assert_eq!(out.display_name, "Alex");
    drop(path);
    drop(dir);
}

// ---------------------------------------------------------------------------
// testing.md gated module: "SQLCipher opened with raw key form (not passphrase KDF)"
// ---------------------------------------------------------------------------

/// A plain round-trip sanity check: arbitrary (non-ASCII-printable) 32-byte key material
/// reopens the same database it created. This does **not** by itself distinguish the raw
/// `x'...'` key form from a passphrase-string form — both forms are internally
/// self-consistent, so a self-swap of one for the other would still pass a test shaped
/// like this one. The actual raw-key-vs-passphrase-KDF property (testing.md §5.3's gated
/// module) is `pg_core::vault`'s own `open_uses_raw_key_form_not_passphrase_kdf` unit
/// test, which opens a file created by `SqlCipherVault` through each PRAGMA form
/// independently and asserts only the raw form succeeds.
#[test]
fn raw_key_bytes_round_trip_exactly() {
    let (dir, path) = temp_db_path();
    // A key whose bytes are not printable ASCII, so a passphrase-string code path would
    // behave differently (lossy conversion or PBKDF2) than the raw-key path.
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(0xA5);
    }
    let key = zeroize::Zeroizing::new(bytes);

    {
        let vault = Arc::new(SqlCipherVault::new(path.clone()));
        vault.open(&key).expect("create with raw key bytes");
        vault.close();
    }

    let reopened = SqlCipherVault::new(path);
    reopened
        .open(&key)
        .expect("the exact same 32 raw bytes must reopen the same database");
    drop(dir);
}

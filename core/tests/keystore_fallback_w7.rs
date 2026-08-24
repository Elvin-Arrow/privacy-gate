//! W7 — Linux keystore fallback.
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §3.2 (Secret Service unavailable → `0600` wrap file next
//!   to the DB; wrap still holds; anti-truncation does not survive a coordinated rollback
//!   of `vault.db` + the fallback file)
//! - `docs/specs/testing.md` §6.5 (AC-5 — stolen data file, vault locked), the "Linux
//!   fallback" §8 row ("Key Manager reports fallback backend; AC-5 still holds;
//!   coordinated rollback of DB+file is **not** asserted as detectable")
//! - `docs/dev-plan.md` W7 ("Tests first: fallback backend reported; wrong passphrase
//!   still fails; stolen dir without passphrase cannot decrypt.")
//!
//! `FileKeystore` and `OsKeystore` themselves — file mechanics (0600, atomic write),
//! round-trip, corrupt-content handling — are W2's (`core/tests/session_w2.rs`). This
//! chunk's own scope is narrower and was explicitly deferred by W2's module docs:
//! "Choosing between them is W7." What's tested here is [`pg_core::keystore::select_backend_with`]
//! itself, plus that a `SessionManager` actually built on the fallback backend behaves
//! exactly like one built on any other `KeystoreBackend` for the properties that matter —
//! wrong passphrase still fails, a stolen file still can't be decrypted.
//!
//! Explicitly **not** claimed (dev-plan W7 "Do not: change the threat model to claim
//! coordinated rollback detection"): nothing here asserts that stealing `vault.db` *and*
//! the fallback file together, then rolling both back to an earlier state, is detectable.

use std::sync::Arc;

use pg_core::account::InMemoryAccountStore;
use pg_core::keys::unwrap_master_key;
use pg_core::keystore::{
    select_backend_with, FileKeystore, KeystoreBackend, KeystoreBackendKind, FALLBACK_FILE_NAME,
};
use pg_core::session::{CreateAccountIn, SessionManager, UnlockIn};

const PASSPHRASE: &str = "correct horse battery staple";
const WRONG_PASSPHRASE: &str = "a different passphrase entirely";

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

// ---------------------------------------------------------------------------
// dev-plan W7: "fallback backend reported"
// ---------------------------------------------------------------------------

#[test]
fn selects_the_file_fallback_when_the_os_keystore_is_unavailable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let backend = select_backend_with(|| false, dir.path());
    assert_eq!(backend.kind(), KeystoreBackendKind::FileFallback);
}

#[test]
fn selects_the_os_keystore_when_available() {
    let dir = tempfile::tempdir().expect("temp dir");
    let backend = select_backend_with(|| true, dir.path());
    assert_eq!(backend.kind(), KeystoreBackendKind::OsKeystore);
}

#[test]
fn the_fallback_file_lands_under_the_given_app_data_dir() {
    let dir = tempfile::tempdir().expect("temp dir");
    let backend = select_backend_with(|| false, dir.path());
    assert!(!dir.path().join(FALLBACK_FILE_NAME).exists(), "not written until first store");

    let mut mgr = SessionManager::new(backend, Arc::new(InMemoryAccountStore::new()));
    mgr.create_account(create_in()).expect("create_account");

    assert!(
        dir.path().join(FALLBACK_FILE_NAME).exists(),
        "select_backend_with's chosen path must be where the fallback actually writes"
    );
}

// ---------------------------------------------------------------------------
// dev-plan W7: "wrong passphrase still fails" (on the fallback backend specifically —
// not a re-test of W2's passphrase logic, but proof the backend switch doesn't bypass it)
// ---------------------------------------------------------------------------

#[test]
fn wrong_passphrase_still_fails_on_the_fallback_backend() {
    let dir = tempfile::tempdir().expect("temp dir");
    let backend = select_backend_with(|| false, dir.path());
    assert_eq!(backend.kind(), KeystoreBackendKind::FileFallback);

    let mut mgr = SessionManager::new(backend, Arc::new(InMemoryAccountStore::new()));
    mgr.create_account(create_in()).expect("create_account");
    mgr.lock().expect("lock");

    let err = mgr
        .unlock(UnlockIn {
            passphrase: WRONG_PASSPHRASE.to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::UnlockFailed);

    // The correct passphrase still works — this isn't a backend that's simply broken.
    mgr.unlock(UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("correct passphrase must still unlock");
}

// ---------------------------------------------------------------------------
// dev-plan W7: "stolen dir without passphrase cannot decrypt"
// testing.md §6.5 AC-5 / §8 "Linux fallback": "coordinated rollback of DB+file is not
// asserted as detectable" — this test only claims the passphrase-wrap property, nothing
// about anti-truncation surviving a stolen-and-rolled-back directory.
// ---------------------------------------------------------------------------

#[test]
fn stolen_fallback_file_cannot_be_decrypted_without_the_passphrase() {
    let dir = tempfile::tempdir().expect("temp dir");
    let backend = select_backend_with(|| false, dir.path());

    {
        let mut mgr = SessionManager::new(backend, Arc::new(InMemoryAccountStore::new()));
        mgr.create_account(create_in()).expect("create_account");
        mgr.lock().expect("lock");
    }

    // "Stolen": read the fallback file directly, the way an attacker with a copy of the
    // app-data directory would, entirely independent of `SessionManager`.
    let stolen = FileKeystore::new(dir.path().join(FALLBACK_FILE_NAME));
    let item = stolen
        .load()
        .expect("stolen file must still be structurally readable")
        .expect("an item was written");

    assert!(
        unwrap_master_key(WRONG_PASSPHRASE, &item).is_none(),
        "wrong passphrase must not unwrap the stolen item"
    );
    assert!(
        unwrap_master_key(PASSPHRASE, &item).is_some(),
        "sanity: the correct passphrase must still work on the very same stolen bytes — \
         proving the previous assertion tested wrapping, not a broken fixture"
    );
}

/// The stolen file's raw bytes must not contain the passphrase itself — architecture
/// §3.2: "The passphrase is never written to disk, keystore, logs, or audit payloads."
/// Belt-and-braces on top of `unwrap_master_key` returning `None`: even a naive
/// grep-the-file attack must fail.
#[test]
fn stolen_fallback_file_bytes_do_not_contain_the_passphrase() {
    let dir = tempfile::tempdir().expect("temp dir");
    let backend = select_backend_with(|| false, dir.path());
    let mut mgr = SessionManager::new(backend, Arc::new(InMemoryAccountStore::new()));
    mgr.create_account(create_in()).expect("create_account");
    mgr.lock().expect("lock");

    let raw = std::fs::read(dir.path().join(FALLBACK_FILE_NAME)).expect("read fallback file");
    let text = String::from_utf8_lossy(&raw);
    assert!(
        !text.contains(PASSPHRASE),
        "the passphrase must never appear in the stolen file's bytes"
    );
}

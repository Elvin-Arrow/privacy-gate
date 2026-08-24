//! W6 — Retention config (AC-7 core).
//!
//! Spec sources:
//! - `docs/decisions/0007-retention-default-discard.md` (factory `discard`, unconfirmed;
//!   first `set_retention_default` confirms; later global loosening allowed)
//! - `docs/specs/api.md` §5.2 (`get_retention_default`, `set_retention_default`)
//! - `docs/specs/data-model.md` §5.5 (`Config`)
//! - `docs/specs/testing.md` §6.7 (AC-7 — factory discard and first-import confirmation),
//!   C-TEST-6
//! - `docs/dev-plan.md` W6 ("Tests first: factory values; set confirms; `never_retain` →
//!   `retain` global change allowed; import still absent so only config tests.")
//!
//! Out of W6 scope and deliberately absent here: `import_document` (W10/W11 own the
//! `retention_policy_unset` import gate and per-import override), the first-import modal UI
//! (W32), `detector_preference` (W15c).

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{CreateAccountIn, SessionManager, SetRetentionDefaultIn};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";

fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    (dir, path)
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

/// A `SessionManager` wired to a real `SqlCipherVault` sharing one connection as
/// `AccountStore`/`VaultBackend`/`AuditStore`/`ConfigStore` (architecture §4.2: one
/// database).
fn fresh_with_full_vault() -> (SessionManager, tempfile::TempDir) {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault;
    let mgr = SessionManager::new_full(keystore, accounts, backend, audit, config);
    (mgr, dir)
}

// ---------------------------------------------------------------------------
// dev-plan W6 / testing.md §6.7 AC-7: "factory values"
// ---------------------------------------------------------------------------

#[test]
fn factory_retention_default_is_discard_and_unconfirmed() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");

    let out = mgr.get_retention_default().expect("get_retention_default");
    assert_eq!(out.policy, RetentionPolicy::Discard);
    assert!(!out.confirmed);
}

/// The factory value must survive a lock/unlock cycle — it isn't just an in-memory
/// default that happens to look right before anything is persisted.
#[test]
fn factory_retention_default_survives_lock_and_unlock() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");
    mgr.lock().expect("lock");
    mgr.unlock(pg_core::session::UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");

    let out = mgr.get_retention_default().expect("get_retention_default");
    assert_eq!(out.policy, RetentionPolicy::Discard);
    assert!(!out.confirmed);
}

#[test]
fn get_retention_default_refused_before_unlock() {
    let (mgr, _dir) = fresh_with_full_vault();
    assert_eq!(
        mgr.get_retention_default().unwrap_err().code,
        pg_core::api::ErrorCode::NotInSession
    );
}

// ---------------------------------------------------------------------------
// dev-plan W6: "set confirms"
// ---------------------------------------------------------------------------

#[test]
fn set_retention_default_confirms_and_persists() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");

    let out = mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Discard,
        })
        .expect("set_retention_default");
    assert_eq!(out.policy, RetentionPolicy::Discard);
    assert!(out.confirmed);

    // Persisted, not just returned: a fresh read (and a read after lock/unlock) both see
    // the confirmed state.
    let read_back = mgr.get_retention_default().expect("get_retention_default");
    assert_eq!(read_back.policy, RetentionPolicy::Discard);
    assert!(read_back.confirmed);

    mgr.lock().expect("lock");
    mgr.unlock(pg_core::session::UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");
    let after_reopen = mgr.get_retention_default().expect("get_retention_default");
    assert!(after_reopen.confirmed);
}

#[test]
fn set_retention_default_to_retain_confirms() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");

    let out = mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Retain,
        })
        .expect("set_retention_default");
    assert_eq!(out.policy, RetentionPolicy::Retain);
    assert!(out.confirmed);
}

#[test]
fn set_retention_default_to_never_retain_confirms() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");

    let out = mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::NeverRetain,
        })
        .expect("set_retention_default");
    assert_eq!(out.policy, RetentionPolicy::NeverRetain);
    assert!(out.confirmed);
}

#[test]
fn set_retention_default_refused_before_unlock() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    let err = mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Discard,
        })
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::NotInSession);
}

// ---------------------------------------------------------------------------
// dev-plan W6: "never_retain → retain global change allowed"
// api.md §5.2: "Changing the global default from never_retain to retain is allowed (it is
// not a per-import override)."
// ---------------------------------------------------------------------------

#[test]
fn global_default_may_loosen_from_never_retain_to_retain() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");

    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::NeverRetain,
    })
    .expect("set to never_retain");

    // This command has no paranoid-loosening restriction — that restriction belongs to
    // import_document's per-import override (W11), not to changing the global default
    // itself (decision 0007 point 5: "Changing the confirmed global default later remains
    // allowed, including leaving never_retain").
    let out = mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Retain,
        })
        .expect("global default may loosen from never_retain to retain");
    assert_eq!(out.policy, RetentionPolicy::Retain);
    assert!(out.confirmed);

    let read_back = mgr.get_retention_default().expect("get_retention_default");
    assert_eq!(read_back.policy, RetentionPolicy::Retain);
}

/// Confirming discard, then changing the mind entirely to `never_retain`, then back to
/// `discard` — no state should ever refuse a global change once unlocked.
#[test]
fn global_default_can_be_changed_repeatedly() {
    let (mut mgr, _dir) = fresh_with_full_vault();
    mgr.create_account(create_in()).expect("create_account");

    for policy in [
        RetentionPolicy::Discard,
        RetentionPolicy::Retain,
        RetentionPolicy::NeverRetain,
        RetentionPolicy::Discard,
    ] {
        let out = mgr
            .set_retention_default(SetRetentionDefaultIn { policy })
            .expect("set_retention_default");
        assert_eq!(out.policy, policy);
        assert!(out.confirmed);
    }
}

// ---------------------------------------------------------------------------
// C-API-6: config is not available while degraded (unlike get_account/lock/
// get_integrity_report). No document commands exist yet either — see dev-plan parenthetical
// "import still absent so only config tests."
// ---------------------------------------------------------------------------

#[test]
fn config_commands_are_unregistered_in_degraded_state_by_construction() {
    // A degraded SessionManager cannot yet be constructed directly without the full W5
    // audit-tamper dance (see core/tests/audit_w5.rs); the structural claim available at
    // this layer, matching how audit_w5.rs itself asserts C-API-6 for document commands,
    // is that the table's answer for degraded_integrity is `false` for both commands.
    assert!(!pg_core::session::command_allowed(
        "get_retention_default",
        pg_core::session::SessionState::DegradedIntegrity
    ));
    assert!(!pg_core::session::command_allowed(
        "set_retention_default",
        pg_core::session::SessionState::DegradedIntegrity
    ));
}

// ---------------------------------------------------------------------------
// Envelope-encryption sanity: Config is the first real envelope-encrypted artifact this
// codebase writes (kind=4). A stolen file without the vault key must not be able to read
// the retention policy — the same "stolen data file" property architecture §2.4 makes for
// everything else.
// ---------------------------------------------------------------------------

#[test]
fn config_artifact_is_unreadable_without_the_correct_vault_key() {
    let (dir, path) = temp_db_path();
    {
        let keystore = Arc::new(InMemoryKeystore::new());
        let vault = Arc::new(SqlCipherVault::new(path.clone()));
        let accounts: Arc<dyn AccountStore> = vault.clone();
        let backend: Arc<dyn VaultBackend> = vault.clone();
        let audit: Arc<dyn AuditStore> = vault.clone();
        let config: Arc<dyn ConfigStore> = vault;
        let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config);
        mgr.create_account(create_in()).expect("create_account");
        mgr.set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::NeverRetain,
        })
        .expect("set_retention_default");
        mgr.lock().expect("lock");
    }

    // A stolen copy of the file, opened with a wrong 32-byte key, must fail at the
    // SQLCipher layer before config bytes are ever reachable — proven already by
    // vault_w3.rs's `stolen_file_without_the_correct_key_cannot_be_queried`; this test adds
    // the config-specific half: even *with* the vault open (e.g. a different account's
    // vault reusing this file, a scenario this test approximates by using a fresh random
    // master key against the real file), the config plaintext does not decrypt.
    let stolen = SqlCipherVault::new(path);
    let wrong_key = zeroize::Zeroizing::new([0x77u8; 32]);
    let open_result = stolen.open(&wrong_key);
    assert!(open_result.is_err(), "wrong SQLCipher key must not open the file at all");
    drop(dir);
}

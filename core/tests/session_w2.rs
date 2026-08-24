//! W2 — Account, keystore, session.
//!
//! Spec sources:
//! - `docs/specs/api.md` §2 (session model + command-availability table), §3 (error model),
//!   §5.1 (command signatures), §9 (C-API-1: passphrase never in an output)
//! - `docs/specs/architecture.md` §3.1 (Argon2id → `wrap_key`, HKDF labels), §3.2 (key
//!   storage, Linux 0600 fallback, passphrase never on disk), §3.3 (unlock / lock /
//!   change passphrase), §3.4 (first-run), §7 (local-only account)
//! - `docs/specs/data-model.md` §5.9 (`KeystoreItem`, `Argon2idParams`, `AuditHead`),
//!   §5.6 (`LocalAccount`)
//! - `docs/dev-plan.md` W2 ("Tests first: first_run → create → unlocked; lock → locked;
//!   wrong passphrase `unlock_failed`; `account_exists`; passphrase min length 8;
//!   change_passphrase wrong current `passphrase_mismatch`; outputs never contain
//!   passphrase (C-API-1)")
//!
//! Out of W2 scope and deliberately absent here: SQLCipher / vault DB (W3),
//! `get_integrity_report` and audit-chain verification (W5), Secret Service probing
//! (W7), documents, Tauri IPC (W29), UI.

use std::sync::Arc;

use pg_core::account::{AccountStore, InMemoryAccountStore};
use pg_core::api::ErrorCode;
use pg_core::crypto::{derive, Aad, ArtifactKind};
use pg_core::keystore::{
    Argon2idParams, AuditHead, FileKeystore, InMemoryKeystore, KeystoreBackend, KeystoreBackendKind,
    KeystoreError, KeystoreItem,
};
use pg_core::session::{
    ChangePassphraseIn, CreateAccountIn, SessionManager, SessionState, UnlockIn,
};

const PASSPHRASE: &str = "correct horse battery staple";
const OTHER_PASSPHRASE: &str = "a different passphrase entirely";

/// A `SessionManager` on a fresh in-memory keystore + account store, plus a handle to
/// the keystore so a test can inspect or corrupt the persisted `KeystoreItem` directly.
fn fresh() -> (SessionManager, Arc<InMemoryKeystore>) {
    let keystore = Arc::new(InMemoryKeystore::new());
    let accounts = Arc::new(InMemoryAccountStore::new());
    let mgr = SessionManager::new(keystore.clone(), accounts);
    (mgr, keystore)
}

fn created() -> (SessionManager, Arc<InMemoryKeystore>) {
    let (mut mgr, ks) = fresh();
    mgr.create_account(CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("first-run create_account must succeed");
    (mgr, ks)
}

fn unlock_in(p: &str) -> UnlockIn {
    UnlockIn {
        passphrase: p.to_string(),
    }
}

// ---------------------------------------------------------------------------
// api.md §2 — session model
// ---------------------------------------------------------------------------

/// api.md §2: `SessionState = "first_run" | "locked" | "unlocked" | "degraded_integrity"`.
///
/// All four variants must exist *now* so W5 can reach `degraded_integrity` without a
/// breaking type change (dev-plan W2: "Degraded path stub returns `unlocked` until W5").
#[test]
fn session_state_has_all_four_wire_variants_api_2() {
    assert_eq!(SessionState::FirstRun.as_str(), "first_run");
    assert_eq!(SessionState::Locked.as_str(), "locked");
    assert_eq!(SessionState::Unlocked.as_str(), "unlocked");
    assert_eq!(SessionState::DegradedIntegrity.as_str(), "degraded_integrity");

    assert_eq!(
        serde_json::to_string(&SessionState::DegradedIntegrity).unwrap(),
        "\"degraded_integrity\""
    );
}

/// api.md §2: "`get_session_state` is callable in every state (including before first run)."
#[test]
fn get_session_state_is_first_run_before_any_account_api_2() {
    let (mgr, _) = fresh();
    assert_eq!(mgr.get_session_state().state, SessionState::FirstRun);
}

/// dev-plan W2: "first_run → create → unlocked".
#[test]
fn create_account_from_first_run_returns_unlocked_api_5_1() {
    let (mut mgr, _) = fresh();
    let out = mgr
        .create_account(CreateAccountIn {
            display_name: "Alex".into(),
            passphrase: PASSPHRASE.into(),
        })
        .unwrap();

    assert_eq!(out.state, SessionState::Unlocked);
    assert!(!out.account_id.is_empty());
    assert_eq!(mgr.get_session_state().state, SessionState::Unlocked);
}

/// dev-plan W2: "lock → locked".
#[test]
fn lock_from_unlocked_returns_locked_api_5_1() {
    let (mut mgr, _) = created();
    let out = mgr.lock().unwrap();
    assert_eq!(out.state, SessionState::Locked);
    assert_eq!(mgr.get_session_state().state, SessionState::Locked);
}

/// architecture §3.3 unlock: the same master key comes back, so `sqlcipher_key` is stable
/// across a lock/unlock cycle.
#[test]
fn unlock_with_correct_passphrase_returns_unlocked_api_5_1() {
    let (mut mgr, _) = created();
    let before = mgr.sqlcipher_key().unwrap();
    mgr.lock().unwrap();

    let out = mgr.unlock(unlock_in(PASSPHRASE)).unwrap();
    assert_eq!(out.state, SessionState::Unlocked);
    // dev-plan W2: degraded path is a stub until W5, so `integrity` is always null here.
    assert!(out.integrity.is_none());

    let after = mgr.sqlcipher_key().unwrap();
    assert_eq!(*before, *after);
}

// ---------------------------------------------------------------------------
// api.md §2 command-availability table → `not_in_session`
// ---------------------------------------------------------------------------

/// api.md §2: `unlock` is not available while already `unlocked`.
#[test]
fn unlock_while_unlocked_is_not_in_session_api_2() {
    let (mut mgr, _) = created();
    let err = mgr.unlock(unlock_in(PASSPHRASE)).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

/// api.md §2: `unlock` is not available in `first_run` either.
#[test]
fn unlock_in_first_run_is_not_in_session_api_2() {
    let (mut mgr, _) = fresh();
    let err = mgr.unlock(unlock_in(PASSPHRASE)).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

/// api.md §2: `lock` is only available while `unlocked` (or degraded, unreachable in W2).
#[test]
fn lock_when_not_unlocked_is_not_in_session_api_2() {
    let (mut mgr, _) = fresh();
    assert_eq!(mgr.lock().unwrap_err().code, ErrorCode::NotInSession);

    let (mut mgr, _) = created();
    mgr.lock().unwrap();
    assert_eq!(mgr.lock().unwrap_err().code, ErrorCode::NotInSession);
}

/// api.md §2: `change_passphrase` is only available while `unlocked`.
#[test]
fn change_passphrase_when_not_unlocked_is_not_in_session_api_2() {
    let (mut mgr, _) = fresh();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.into(),
            new_passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);

    let (mut mgr, _) = created();
    mgr.lock().unwrap();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.into(),
            new_passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

/// api.md §2: `get_account` is not available in `first_run` or `locked`.
#[test]
fn get_account_when_not_unlocked_is_not_in_session_api_2() {
    let (mgr, _) = fresh();
    assert_eq!(
        mgr.get_account().unwrap_err().code,
        ErrorCode::NotInSession
    );

    let (mut mgr, _) = created();
    mgr.lock().unwrap();
    assert_eq!(
        mgr.get_account().unwrap_err().code,
        ErrorCode::NotInSession
    );
}

// ---------------------------------------------------------------------------
// api.md §3 — error model
// ---------------------------------------------------------------------------

/// dev-plan W2 + api.md §3: "wrong passphrase `unlock_failed`".
#[test]
fn unlock_with_wrong_passphrase_is_unlock_failed_api_3() {
    let (mut mgr, _) = created();
    mgr.lock().unwrap();
    let err = mgr.unlock(unlock_in("wrong passphrase here")).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnlockFailed);
}

/// api.md §3: "Wrong passphrase and unknown account are the same `unlock_failed` (no
/// account-enumeration **beyond `\"first_run\"` vs `\"locked\"`, which is visible from
/// `get_session_state`**)."
///
/// With no `KeystoreItem` the state genuinely *is* `first_run` — the one signal the spec
/// permits — so `unlock` is refused by the api.md §2 availability table. What must not
/// happen is a distinct "unknown account" code such as `not_found`.
#[test]
fn unlock_without_a_keystore_item_reveals_nothing_beyond_first_run_api_3() {
    let (mut mgr, ks) = created();
    mgr.lock().unwrap();
    ks.delete().unwrap();

    assert_eq!(mgr.get_session_state().state, SessionState::FirstRun);
    let err = mgr.unlock(unlock_in(PASSPHRASE)).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
    assert_ne!(err.code, ErrorCode::NotFound);
}

/// A keystore that cannot be *read* must never be reported as `first_run`: that would
/// offer to create an account over a live vault and overwrite the only copy of the
/// wrapped master key. It fails safe towards `locked`, and the subsequent `unlock`
/// collapses into the same `unlock_failed` as a wrong passphrase (api.md §3).
#[test]
fn keystore_read_failure_fails_safe_to_locked_and_unlock_failed_api_3() {
    let (mut mgr, ks) = created();
    mgr.lock().unwrap();

    // `unlock` reads the keystore twice: once to compute the state, once for the item.
    ks.fail_next_loads(2);
    let err = mgr.unlock(unlock_in(PASSPHRASE)).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnlockFailed);
    assert!(!mgr.has_resident_key_material());

    // Once the backend recovers, the correct passphrase still opens the vault.
    assert_eq!(
        mgr.unlock(unlock_in(PASSPHRASE)).unwrap().state,
        SessionState::Unlocked
    );
}

/// The same fail-safe seen through `get_session_state`: a read failure is `locked`, never
/// `first_run` (which would invite `create_account`).
#[test]
fn get_session_state_never_reports_first_run_on_a_read_failure_arch_3_2() {
    let (mut mgr, ks) = created();
    mgr.lock().unwrap();
    ks.fail_next_loads(1);
    assert_eq!(mgr.get_session_state().state, SessionState::Locked);
}

/// `create_account` must also refuse while the keystore is unreadable — the state is
/// `locked`, so the answer is `account_exists`, not a first-run overwrite.
#[test]
fn create_account_refuses_while_the_keystore_is_unreadable_arch_3_4() {
    let (mut mgr, ks) = created();
    mgr.lock().unwrap();
    ks.fail_next_loads(1);
    let err = mgr
        .create_account(CreateAccountIn {
            display_name: "Second".into(),
            passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AccountExists);
}

/// architecture §3.3: "Passphrase failure zeroizes and refuses (no partial open)."
#[test]
fn failed_unlock_leaves_session_locked_with_no_key_material_arch_3_3() {
    let (mut mgr, _) = created();
    mgr.lock().unwrap();
    mgr.unlock(unlock_in("wrong passphrase here")).unwrap_err();

    assert_eq!(mgr.get_session_state().state, SessionState::Locked);
    assert!(!mgr.has_resident_key_material());
    assert_eq!(
        mgr.sqlcipher_key().unwrap_err().code,
        ErrorCode::NotInSession
    );
}

/// api.md §3: `account_exists` — "`create_account` when not `\"first_run\"`".
#[test]
fn create_account_when_unlocked_is_account_exists_api_3() {
    let (mut mgr, _) = created();
    let err = mgr
        .create_account(CreateAccountIn {
            display_name: "Second".into(),
            passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AccountExists);
}

#[test]
fn create_account_when_locked_is_account_exists_api_3() {
    let (mut mgr, _) = created();
    mgr.lock().unwrap();
    let err = mgr
        .create_account(CreateAccountIn {
            display_name: "Second".into(),
            passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AccountExists);
}

/// `account_exists` must not clobber the existing `KeystoreItem`.
#[test]
fn rejected_create_account_leaves_keystore_item_unchanged_api_3() {
    let (mut mgr, ks) = created();
    mgr.lock().unwrap();
    let before = ks.load().unwrap().unwrap();

    mgr.create_account(CreateAccountIn {
        display_name: "Second".into(),
        passphrase: OTHER_PASSPHRASE.into(),
    })
    .unwrap_err();

    let after = ks.load().unwrap().unwrap();
    assert_eq!(before, after);
    // …and the original passphrase still opens the vault.
    assert_eq!(
        mgr.unlock(unlock_in(PASSPHRASE)).unwrap().state,
        SessionState::Unlocked
    );
}

// ---------------------------------------------------------------------------
// api.md §5.1 — input validation
// ---------------------------------------------------------------------------

/// api.md §5.1: "`passphrase` min length 8 (API floor)".
#[test]
fn create_account_passphrase_shorter_than_8_is_invalid_input_api_5_1() {
    for short in ["", "a", "1234567"] {
        let (mut mgr, ks) = fresh();
        let err = mgr
            .create_account(CreateAccountIn {
                display_name: "Alex".into(),
                passphrase: short.into(),
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "passphrase {short:?}");
        // Validation failure must not have written anything.
        assert!(ks.load().unwrap().is_none());
        assert_eq!(mgr.get_session_state().state, SessionState::FirstRun);
    }
}

#[test]
fn create_account_passphrase_exactly_8_is_accepted_api_5_1() {
    let (mut mgr, _) = fresh();
    let out = mgr
        .create_account(CreateAccountIn {
            display_name: "Alex".into(),
            passphrase: "12345678".into(),
        })
        .unwrap();
    assert_eq!(out.state, SessionState::Unlocked);
}

/// api.md §5.1: "Empty display_name → `invalid_input`."
#[test]
fn create_account_empty_display_name_is_invalid_input_api_5_1() {
    for name in ["", "   ", "\t\n"] {
        let (mut mgr, _) = fresh();
        let err = mgr
            .create_account(CreateAccountIn {
                display_name: name.into(),
                passphrase: PASSPHRASE.into(),
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "display_name {name:?}");
    }
}

/// api.md §5.1 / data-model §5.6: "`display_name` trimmed, 1..=80 chars."
#[test]
fn create_account_trims_display_name_api_5_1() {
    let (mut mgr, _) = fresh();
    mgr.create_account(CreateAccountIn {
        display_name: "  Alex Doe \n".into(),
        passphrase: PASSPHRASE.into(),
    })
    .unwrap();
    assert_eq!(mgr.get_account().unwrap().display_name, "Alex Doe");
}

#[test]
fn create_account_display_name_boundary_80_and_81_api_5_1() {
    let (mut mgr, _) = fresh();
    assert!(mgr
        .create_account(CreateAccountIn {
            display_name: "x".repeat(80),
            passphrase: PASSPHRASE.into(),
        })
        .is_ok());

    let (mut mgr, _) = fresh();
    let err = mgr
        .create_account(CreateAccountIn {
            display_name: "x".repeat(81),
            passphrase: PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

/// The 1..=80 bound is on trimmed *characters*, not bytes — a spec-faithful reading of
/// data-model §5.6 ("1..=80 trimmed").
#[test]
fn create_account_display_name_length_counts_chars_not_bytes_data_model_5_6() {
    let (mut mgr, _) = fresh();
    // 80 multi-byte chars = 240 bytes; must be accepted.
    assert!(mgr
        .create_account(CreateAccountIn {
            display_name: "é".repeat(80),
            passphrase: PASSPHRASE.into(),
        })
        .is_ok());
}

// ---------------------------------------------------------------------------
// api.md §5.1 `get_account` — data-model §5.6 `LocalAccount`
// ---------------------------------------------------------------------------

/// api.md §5.1: `get_account` Out = `{ account_id, display_name, created_at }`.
#[test]
fn get_account_returns_local_account_fields_api_5_1() {
    let (mut mgr, _) = fresh();
    let created = mgr
        .create_account(CreateAccountIn {
            display_name: "Alex".into(),
            passphrase: PASSPHRASE.into(),
        })
        .unwrap();

    let acct = mgr.get_account().unwrap();
    assert_eq!(acct.account_id, created.account_id);
    assert_eq!(acct.display_name, "Alex");
    // data-model §5.6 `created_at: Timestamp` — api.md renders timestamps as RFC 3339 UTC.
    assert!(acct.created_at.ends_with('Z'), "got {}", acct.created_at);
}

/// data-model §5.6: "`id`: UUID, generated on device". architecture §7: no network identity.
#[test]
fn account_id_is_a_device_generated_uuid_data_model_5_6() {
    let (mut mgr, _) = fresh();
    let id = mgr
        .create_account(CreateAccountIn {
            display_name: "Alex".into(),
            passphrase: PASSPHRASE.into(),
        })
        .unwrap()
        .account_id;
    assert_eq!(id.len(), 36);
    assert_eq!(id.matches('-').count(), 4);
    let groups: Vec<&str> = id.split('-').collect();
    assert_eq!(
        groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
        vec![8, 4, 4, 4, 12]
    );
    assert!(id
        .chars()
        .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    // UUID v4, RFC 4122 variant.
    assert_eq!(groups[2].as_bytes()[0], b'4');
    assert!(matches!(groups[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
}

/// architecture §3.2 / data-model §5.9: `KeystoreItem.account_id` "matches `LocalAccount.id`".
#[test]
fn keystore_item_account_id_matches_local_account_data_model_5_9() {
    let (mut mgr, ks) = fresh();
    let out = mgr
        .create_account(CreateAccountIn {
            display_name: "Alex".into(),
            passphrase: PASSPHRASE.into(),
        })
        .unwrap();
    assert_eq!(ks.load().unwrap().unwrap().account_id, out.account_id);
}

// ---------------------------------------------------------------------------
// architecture §3.3 — change passphrase
// ---------------------------------------------------------------------------

/// dev-plan W2 + api.md §3: "change_passphrase wrong current `passphrase_mismatch`".
#[test]
fn change_passphrase_wrong_current_is_passphrase_mismatch_api_3() {
    let (mut mgr, _) = created();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: "not the current one".into(),
            new_passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PassphraseMismatch);
}

/// architecture §3.3: a failed change must not partially write. The stored `KeystoreItem`
/// (salt + wrapped master key) has to be byte-identical afterwards.
#[test]
fn change_passphrase_wrong_current_leaves_keystore_item_unchanged_arch_3_3() {
    let (mut mgr, ks) = created();
    let before = ks.load().unwrap().unwrap();

    mgr.change_passphrase(ChangePassphraseIn {
        current: "not the current one".into(),
        new_passphrase: OTHER_PASSPHRASE.into(),
    })
    .unwrap_err();

    assert_eq!(before, ks.load().unwrap().unwrap());
    // The old passphrase still works.
    mgr.lock().unwrap();
    assert_eq!(
        mgr.unlock(unlock_in(PASSPHRASE)).unwrap().state,
        SessionState::Unlocked
    );
}

/// architecture §3.3: "re-derive a new `wrap_key` from the new passphrase and a **new**
/// salt; re-wrap the **same** `vault_master_key`."
#[test]
fn change_passphrase_uses_new_salt_and_keeps_same_master_key_arch_3_3() {
    let (mut mgr, ks) = created();
    let before = ks.load().unwrap().unwrap();
    let master_before = mgr.sqlcipher_key().unwrap();

    let out = mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.into(),
            new_passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap();
    assert!(out.ok);

    let after = ks.load().unwrap().unwrap();
    assert_ne!(before.kdf.salt, after.kdf.salt, "salt must be fresh");
    assert_ne!(
        before.wrapped_master_key.ciphertext, after.wrapped_master_key.ciphertext,
        "re-wrap must produce new ciphertext"
    );
    assert_eq!(before.account_id, after.account_id);

    // Same master key ⇒ same derived sqlcipher_key: this is KEK rotation, not master
    // rotation, so W3's DB stays openable.
    assert_eq!(*master_before, *mgr.sqlcipher_key().unwrap());
    // Session stays unlocked (api.md §5.1 Out = `{ ok: true }`, no state change).
    assert_eq!(mgr.get_session_state().state, SessionState::Unlocked);
}

#[test]
fn change_passphrase_then_new_passphrase_unlocks_and_old_fails_arch_3_3() {
    let (mut mgr, _) = created();
    let master_before = mgr.sqlcipher_key().unwrap();
    mgr.change_passphrase(ChangePassphraseIn {
        current: PASSPHRASE.into(),
        new_passphrase: OTHER_PASSPHRASE.into(),
    })
    .unwrap();
    mgr.lock().unwrap();

    assert_eq!(
        mgr.unlock(unlock_in(PASSPHRASE)).unwrap_err().code,
        ErrorCode::UnlockFailed
    );
    assert_eq!(mgr.get_session_state().state, SessionState::Locked);

    mgr.unlock(unlock_in(OTHER_PASSPHRASE)).unwrap();
    assert_eq!(*master_before, *mgr.sqlcipher_key().unwrap());
}

/// api.md §5.1: the min-length-8 floor applies to the new passphrase too.
#[test]
fn change_passphrase_new_shorter_than_8_is_invalid_input_api_5_1() {
    let (mut mgr, ks) = created();
    let before = ks.load().unwrap().unwrap();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.into(),
            new_passphrase: "short".into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(before, ks.load().unwrap().unwrap());
}

/// `invalid_input` on the new passphrase must be reported *without* first testing the
/// current one, so a too-short new passphrase is never an oracle for the current one.
#[test]
fn change_passphrase_validates_new_passphrase_before_checking_current_api_5_1() {
    let (mut mgr, _) = created();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: "also wrong".into(),
            new_passphrase: "short".into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------------------
// architecture §3.3 lock — "zeroize master key, wrap key, DEKs …"
// dev-plan W2 done-when: "lock zeroizes session key material (assert via subsequent
// decrypt/command failure, not log lines)."
// ---------------------------------------------------------------------------

/// Structural property: after `lock`, the session holds no key material at all, so there
/// is nothing left to use — stronger than zeroize-and-hope.
#[test]
fn lock_leaves_no_resident_key_material_arch_3_3() {
    let (mut mgr, _) = created();
    assert!(mgr.has_resident_key_material());
    mgr.lock().unwrap();
    assert!(!mgr.has_resident_key_material());
}

/// Behavioural property: every key-material consumer fails after `lock`, and succeeds
/// again only after a correct `unlock`.
#[test]
fn lock_makes_derived_keys_unavailable_arch_3_3() {
    let (mut mgr, _) = created();
    assert!(mgr.sqlcipher_key().is_ok());
    assert!(mgr.audit_mac_key().is_ok());

    mgr.lock().unwrap();

    assert_eq!(
        mgr.sqlcipher_key().unwrap_err().code,
        ErrorCode::NotInSession
    );
    assert_eq!(
        mgr.audit_mac_key().unwrap_err().code,
        ErrorCode::NotInSession
    );

    mgr.unlock(unlock_in(PASSPHRASE)).unwrap();
    assert!(mgr.sqlcipher_key().is_ok());
}

/// Cryptographic-erasure property: a blob sealed under a DEK wrapped by the live master
/// key can no longer be opened through the locked session — the unwrapping key is gone.
#[test]
fn artifact_sealed_before_lock_cannot_be_opened_after_lock_arch_3_3() {
    let (mut mgr, _) = created();

    let aad = Aad::for_document(ArtifactKind::Approved, "doc-1", 1);
    let sealed = {
        let master = mgr.sqlcipher_key().unwrap();
        pg_core::crypto::wrap(&master, b"sensitive", &aad).unwrap()
    };

    mgr.lock().unwrap();
    // No key is reachable through the session at all.
    assert!(mgr.sqlcipher_key().is_err());

    mgr.unlock(unlock_in(PASSPHRASE)).unwrap();
    let master = mgr.sqlcipher_key().unwrap();
    assert_eq!(
        pg_core::crypto::unwrap(&master, &sealed, &aad).unwrap(),
        b"sensitive"
    );
}

// ---------------------------------------------------------------------------
// architecture §3.1 — key hierarchy wiring
// ---------------------------------------------------------------------------

/// architecture §3.1: `vault_master_key ─ HKDF-SHA-256 info="pg-db-v1" → sqlcipher_key`
/// and `info="pg-audit-mac-v1" → audit_mac_key`, and the two are distinct.
#[test]
fn derived_subkeys_use_the_spec_hkdf_labels_arch_3_1() {
    let (mgr, ks) = created();
    let item = ks.load().unwrap().unwrap();

    // Independently re-derive from the stored item, the way an offline check would.
    let wrap_key = pg_core::keys::derive_wrap_key(PASSPHRASE, &item.kdf).unwrap();
    let master = pg_core::crypto::unwrap(
        &wrap_key,
        &item.wrapped_master_key,
        &Aad::global(ArtifactKind::WrappedMaster, 1),
    )
    .unwrap();
    let master: [u8; 32] = master.try_into().unwrap();

    assert_eq!(*mgr.sqlcipher_key().unwrap(), derive(&master, "pg-db-v1"));
    assert_eq!(
        *mgr.audit_mac_key().unwrap(),
        derive(&master, "pg-audit-mac-v1")
    );
    assert_ne!(
        *mgr.sqlcipher_key().unwrap(),
        *mgr.audit_mac_key().unwrap()
    );
}

/// data-model §5.9: `wrapped_master_key` is "AEAD(wrap_key, vault_master_key); **AAD kind 6**".
/// A different AAD kind must not open it.
#[test]
fn wrapped_master_key_is_bound_to_aad_kind_6_data_model_5_9() {
    let (_mgr, ks) = created();
    let item = ks.load().unwrap().unwrap();
    let wrap_key = pg_core::keys::derive_wrap_key(PASSPHRASE, &item.kdf).unwrap();

    assert_eq!(ArtifactKind::WrappedMaster.code(), 6);
    assert!(pg_core::crypto::unwrap(
        &wrap_key,
        &item.wrapped_master_key,
        &Aad::global(ArtifactKind::WrappedMaster, 1)
    )
    .is_ok());
    assert!(pg_core::crypto::unwrap(
        &wrap_key,
        &item.wrapped_master_key,
        &Aad::global(ArtifactKind::Config, 1)
    )
    .is_err());
}

/// architecture §3.1: "Floor: OWASP minimum current at implementation." The parameters
/// actually written into the `KeystoreItem` must be at or above that floor.
#[test]
fn stored_argon2id_params_meet_the_owasp_floor_arch_3_1() {
    let (_mgr, ks) = created();
    let kdf = ks.load().unwrap().unwrap().kdf;
    let floor = Argon2idParams::OWASP_FLOOR;

    assert!(kdf.m_cost >= floor.m_cost, "m_cost {}", kdf.m_cost);
    assert!(kdf.t_cost >= floor.t_cost, "t_cost {}", kdf.t_cost);
    assert!(kdf.p_cost >= floor.p_cost, "p_cost {}", kdf.p_cost);
    // A per-account random salt, long enough that rainbow tables are pointless.
    assert!(kdf.salt.len() >= 16);
}

/// Two accounts created with the *same* passphrase must not share a salt (and therefore
/// not a `wrap_key`) — architecture §3.1 stores the salt per keystore item.
#[test]
fn salt_is_random_per_account_arch_3_1() {
    let (mut a, ks_a) = fresh();
    a.create_account(CreateAccountIn {
        display_name: "A".into(),
        passphrase: PASSPHRASE.into(),
    })
    .unwrap();
    let (mut b, ks_b) = fresh();
    b.create_account(CreateAccountIn {
        display_name: "B".into(),
        passphrase: PASSPHRASE.into(),
    })
    .unwrap();

    let (ia, ib) = (
        ks_a.load().unwrap().unwrap(),
        ks_b.load().unwrap().unwrap(),
    );
    assert_ne!(ia.kdf.salt, ib.kdf.salt);
    assert_ne!(ia.account_id, ib.account_id);
    // Different master keys too (CSPRNG at first run, architecture §3.4).
    assert_ne!(*a.sqlcipher_key().unwrap(), *b.sqlcipher_key().unwrap());
}

/// data-model §5.9 `AuditHead`. No audit chain exists until W5, so first-run writes the
/// documented placeholder head rather than leaving the field undefined.
#[test]
fn first_run_writes_placeholder_audit_head_until_w5_data_model_5_9() {
    let (_mgr, ks) = created();
    let head = ks.load().unwrap().unwrap().audit_head;
    assert_eq!(head, AuditHead::GENESIS);
    assert_eq!(head.sequence, 0);
    assert_eq!(head.head_hash, [0u8; 32]);
}

/// architecture §3.3: change-passphrase is KEK rotation only — it must carry the existing
/// `audit_head` across untouched, or W5's anti-truncation check would be reset by a
/// passphrase change.
#[test]
fn change_passphrase_preserves_audit_head_arch_3_3() {
    let (mut mgr, ks) = created();
    // Simulate a W5-era head already persisted.
    let mut item = ks.load().unwrap().unwrap();
    item.audit_head = AuditHead {
        sequence: 42,
        head_hash: [7u8; 32],
    };
    ks.store(&item).unwrap();

    mgr.change_passphrase(ChangePassphraseIn {
        current: PASSPHRASE.into(),
        new_passphrase: OTHER_PASSPHRASE.into(),
    })
    .unwrap();

    assert_eq!(ks.load().unwrap().unwrap().audit_head, item.audit_head);
}

// ---------------------------------------------------------------------------
// architecture §3.2 / C-API-1 — the passphrase never leaves the input
// ---------------------------------------------------------------------------

/// C-API-1 (api.md §9): "Passphrase … appear[s] only as command **inputs** … They never
/// appear in outputs, events, or audit DTOs."
///
/// Sweeps every success and failure path of the six W2 commands with a distinctive
/// passphrase and asserts the string never occurs in any rendered output or error.
#[test]
fn passphrase_never_appears_in_any_command_output_c_api_1() {
    const SECRET: &str = "zebra-quintessential-passphrase-42";
    const SECRET2: &str = "kumquat-antidisestablishment-99";
    let mut seen: Vec<String> = Vec::new();

    macro_rules! record {
        ($e:expr) => {{
            let r = $e;
            seen.push(format!("{:?}", r));
            r
        }};
    }

    let keystore = Arc::new(InMemoryKeystore::new());
    let mut mgr = SessionManager::new(keystore.clone(), Arc::new(InMemoryAccountStore::new()));

    record!(mgr.get_session_state());
    // Failure paths first (validation, wrong state).
    record!(mgr.create_account(CreateAccountIn {
        display_name: "".into(),
        passphrase: SECRET.into()
    }));
    record!(mgr.unlock(UnlockIn {
        passphrase: SECRET.into()
    }));
    record!(mgr.change_passphrase(ChangePassphraseIn {
        current: SECRET.into(),
        new_passphrase: SECRET2.into()
    }));
    record!(mgr.get_account());
    record!(mgr.lock());

    // Success paths.
    record!(mgr.create_account(CreateAccountIn {
        display_name: "Alex".into(),
        passphrase: SECRET.into()
    }));
    record!(mgr.get_account());
    record!(mgr.get_session_state());
    record!(mgr.change_passphrase(ChangePassphraseIn {
        current: SECRET.into(),
        new_passphrase: SECRET2.into()
    }));
    // Wrong-current failure.
    record!(mgr.change_passphrase(ChangePassphraseIn {
        current: SECRET.into(),
        new_passphrase: SECRET2.into()
    }));
    record!(mgr.lock());
    record!(mgr.unlock(UnlockIn {
        passphrase: SECRET.into()
    })); // now wrong
    record!(mgr.unlock(UnlockIn {
        passphrase: SECRET2.into()
    }));
    record!(mgr.get_account());

    for out in &seen {
        assert!(
            !out.contains(SECRET) && !out.contains(SECRET2),
            "C-API-1 violation: passphrase leaked into `{out}`"
        );
    }

    // …and it is not in the persisted keystore item either (architecture §3.2: "The
    // passphrase is never written to disk, keystore, logs, or audit payloads").
    let item = keystore.load().unwrap().unwrap();
    let blob = String::from_utf8(item.to_bytes()).unwrap();
    assert!(!blob.contains(SECRET) && !blob.contains(SECRET2));
    assert!(!blob.to_lowercase().contains("passphrase"));
}

/// C-API-1: a derived `Debug` on an input struct would leak the passphrase into any
/// `{:?}` log line or panic message. The redaction has to live on the type.
#[test]
fn debug_of_command_inputs_redacts_the_passphrase_c_api_1() {
    const SECRET: &str = "zebra-quintessential-passphrase-42";

    let create = CreateAccountIn {
        display_name: "Alex".into(),
        passphrase: SECRET.into(),
    };
    let unlock = UnlockIn {
        passphrase: SECRET.into(),
    };
    let change = ChangePassphraseIn {
        current: SECRET.into(),
        new_passphrase: SECRET.into(),
    };

    for rendered in [
        format!("{create:?}"),
        format!("{unlock:?}"),
        format!("{change:?}"),
    ] {
        assert!(
            !rendered.contains(SECRET),
            "C-API-1 violation in Debug: `{rendered}`"
        );
        assert!(rendered.contains("redacted"), "got `{rendered}`");
    }
    // display_name is not a secret (data-model §5.6) and stays visible for diagnostics.
    assert!(format!("{create:?}").contains("Alex"));
}

/// api.md §3: `ApiError.message` is "non-secret; never includes passphrase, key, field
/// text, document text". Messages are fixed classes, so there is no interpolation site.
#[test]
fn api_error_messages_are_fixed_non_secret_classes_api_3() {
    let (mut mgr, _) = created();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: "hunter22222".into(),
            new_passphrase: OTHER_PASSPHRASE.into(),
        })
        .unwrap_err();
    assert!(!err.message.contains("hunter22222"));
    assert_eq!(err.code.as_str(), "passphrase_mismatch");
    assert!(!err.message.is_empty());
}

// ---------------------------------------------------------------------------
// architecture §3.2 — keystore backends
// ---------------------------------------------------------------------------

/// architecture §3.2: the Linux fallback persists "the same `KeystoreItem` as a `0600`
/// file under the app-data directory, written via temp-file + `fsync` + atomic `rename`".
#[test]
fn file_keystore_round_trips_the_item_arch_3_2() {
    let dir = tempfile::tempdir().unwrap();
    let ks = FileKeystore::new(dir.path().join("keystore.json"));

    assert!(ks.load().unwrap().is_none());

    let mut mgr = SessionManager::new(Arc::new(ks), Arc::new(InMemoryAccountStore::new()));
    mgr.create_account(CreateAccountIn {
        display_name: "Alex".into(),
        passphrase: PASSPHRASE.into(),
    })
    .unwrap();
    let master = mgr.sqlcipher_key().unwrap();
    mgr.lock().unwrap();

    // A brand-new manager over the same file sees `locked`, not `first_run`.
    let reopened = FileKeystore::new(dir.path().join("keystore.json"));
    let mut mgr2 = SessionManager::new(Arc::new(reopened), Arc::new(InMemoryAccountStore::new()));
    assert_eq!(mgr2.get_session_state().state, SessionState::Locked);
    mgr2.unlock(unlock_in(PASSPHRASE)).unwrap();
    assert_eq!(*master, *mgr2.sqlcipher_key().unwrap());
}

/// architecture §3.2: "`0600` file".
#[cfg(unix)]
#[test]
fn file_keystore_writes_mode_0600_arch_3_2() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let ks = FileKeystore::new(path.clone());
    ks.store(&sample_item()).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "got {mode:o}");
}

/// "temp-file + fsync + atomic rename": no temp file may survive a successful write, and
/// an overwrite must replace the item rather than append or duplicate.
#[test]
fn file_keystore_write_is_atomic_and_leaves_no_temp_file_arch_3_2() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let ks = FileKeystore::new(path.clone());

    ks.store(&sample_item()).unwrap();
    let mut second = sample_item();
    second.audit_head = AuditHead {
        sequence: 9,
        head_hash: [3u8; 32],
    };
    ks.store(&second).unwrap();

    assert_eq!(ks.load().unwrap().unwrap(), second);
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["keystore.json".to_string()], "{entries:?}");
}

/// A truncated or garbled keystore file must fail closed, not be read as "no account"
/// (which would offer the user a first-run flow over a live vault).
#[test]
fn file_keystore_rejects_corrupt_content_arch_3_2() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keystore.json");
    let ks = FileKeystore::new(path.clone());
    ks.store(&sample_item()).unwrap();

    let good = std::fs::read(&path).unwrap();
    std::fs::write(&path, &good[..good.len() / 2]).unwrap();
    assert!(matches!(ks.load(), Err(KeystoreError::Corrupt)));
}

#[test]
fn file_keystore_delete_removes_the_item_arch_3_2() {
    let dir = tempfile::tempdir().unwrap();
    let ks = FileKeystore::new(dir.path().join("keystore.json"));
    ks.store(&sample_item()).unwrap();
    ks.delete().unwrap();
    assert!(ks.load().unwrap().is_none());
    // Deleting again is not an error (idempotent teardown).
    ks.delete().unwrap();
}

/// architecture §3.2: "The Key Manager records which backend is in use so the testing spec
/// can treat the fallback as a distinct configuration with a degraded threat model."
#[test]
fn keystore_backend_kind_is_recorded_arch_3_2() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        FileKeystore::new(dir.path().join("k.json")).kind(),
        KeystoreBackendKind::FileFallback
    );
    assert_eq!(InMemoryKeystore::new().kind(), KeystoreBackendKind::Memory);

    let mgr = SessionManager::new(
        Arc::new(InMemoryKeystore::new()),
        Arc::new(InMemoryAccountStore::new()),
    );
    assert_eq!(mgr.keystore_kind(), KeystoreBackendKind::Memory);
}

/// architecture §3.4 first-run must not leave a half-created account behind: if the
/// keystore write fails, `create_account` reports the failure and the session stays in
/// `first_run` with no account record.
#[test]
fn create_account_rolls_back_when_the_keystore_write_fails_arch_3_4() {
    let ks = Arc::new(InMemoryKeystore::new());
    ks.fail_next_store();
    let accounts = Arc::new(InMemoryAccountStore::new());
    let mut mgr = SessionManager::new(ks.clone(), accounts.clone());

    let err = mgr
        .create_account(CreateAccountIn {
            display_name: "Alex".into(),
            passphrase: PASSPHRASE.into(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal);

    assert_eq!(mgr.get_session_state().state, SessionState::FirstRun);
    assert!(ks.load().unwrap().is_none());
    assert!(accounts.load().unwrap().is_none());
    assert!(!mgr.has_resident_key_material());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_item() -> KeystoreItem {
    KeystoreItem {
        account_id: "1a1e07f7-1a1e-4a1e-8a1e-1a1e07f71a1e".to_string(),
        kdf: Argon2idParams {
            salt: vec![0xAB; 16],
            ..Argon2idParams::OWASP_FLOOR
        },
        wrapped_master_key: pg_core::crypto::wrap(
            &[0x42; 32],
            &[0x24; 32],
            &Aad::global(ArtifactKind::WrappedMaster, 1),
        )
        .unwrap(),
        audit_head: AuditHead::GENESIS,
    }
}

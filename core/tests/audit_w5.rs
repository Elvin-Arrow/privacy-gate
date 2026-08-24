//! W5 — Audit chain and integrity.
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §6 (audit-trail integrity: canonical encoding v1,
//!   anti-truncation head, verification and crash recovery, what this does/does not prove)
//! - `docs/specs/data-model.md` §5.8 (`AuditEntry`/`EventPayload`), §5.9 (`KeystoreItem`,
//!   `AuditHead`)
//! - `docs/specs/api.md` §5.1 (`get_integrity_report`, `unlock`'s `integrity` field), §2
//!   (session gating — `get_integrity_report`'s row)
//! - `docs/specs/testing.md` §8 ("NFR-R1 tamper", "NFR-R1 truncation", "Crash window",
//!   "Integrity vs crash", "C-API-6")
//! - `docs/dev-plan.md` W5 ("Tests first: happy append; flip payload byte → degraded, no
//!   document decrypt (none exist yet); truncation vs head; crash window fast-forward
//!   still `unlocked`; HMAC break is **not** fast-forwarded; degraded session cannot
//!   import/approve/share (C-API-6) — those commands fail even if not fully implemented.")
//!
//! Out of W5 scope and deliberately absent here: no UI integrity screen (W35), no user
//! vault restore, no concrete `EventPayload` shapes for commands that don't exist yet.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::{self, AuditStore, EventType, OriginalsFlag};
use pg_core::keystore::{InMemoryKeystore, KeystoreBackend};
use pg_core::session::{command_allowed, CreateAccountIn, SessionManager, SessionState, UnlockIn};
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

/// A `SessionManager` wired to a real `SqlCipherVault`, sharing the same object as the
/// `AccountStore`, `VaultBackend`, and `AuditStore` (architecture §4.2 / §6: one database).
/// Returns the vault handle too, so a test can append/corrupt/truncate audit rows directly
/// — the way an attacker (or a future command, for "happy append") would.
fn fresh_with_vault_and_audit() -> (
    SessionManager,
    Arc<SqlCipherVault>,
    Arc<InMemoryKeystore>,
    tempfile::TempDir,
) {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit_store: Arc<dyn AuditStore> = vault.clone();
    let mgr = SessionManager::new_with_vault_and_audit(keystore.clone(), accounts, backend, audit_store);
    (mgr, vault, keystore, dir)
}

fn mac_key(mgr: &SessionManager) -> [u8; 32] {
    *mgr.audit_mac_key().expect("session must be unlocked")
}

// ---------------------------------------------------------------------------
// dev-plan W5: "happy append"
// ---------------------------------------------------------------------------

/// Append once, persist the resulting head correctly, lock, unlock — clean, `ok`, no
/// fast-forward needed. The eventual audit-producing commands (W8+) will do the persist
/// step as part of their own command bodies; W5 exercises it directly since no production
/// command drives it yet (`crate::audit` module docs).
#[test]
fn append_with_correctly_persisted_head_unlocks_clean() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    let row = append_row_returning(&vault, &key, "{}");
    persist_head_for(&keystore, &row);

    mgr.lock().expect("lock");
    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    assert_eq!(out.state, SessionState::Unlocked);
    assert!(out.integrity.is_none(), "clean unlock: integrity is null");

    let report = mgr.get_integrity_report().expect("get_integrity_report");
    assert!(report.ok);
    assert_eq!(report.kind, "ok");
    assert_eq!(report.head_sequence, 1);
    assert_eq!(report.tail_sequence, 1);
    assert_eq!(report.first_bad_sequence, None);
}

fn append_row_returning(vault: &Arc<SqlCipherVault>, key: &[u8; 32], payload: &str) -> pg_core::audit::AuditRow {
    audit::append(vault.as_ref(), key, EventType::Import, None, 1_000, OriginalsFlag::Unset, payload)
        .expect("append")
}

/// Persist `row` as the keystore's `audit_head`. `head_hash` is computed the same way
/// `crate::audit`'s (private) `head_hash_of` does — `sha256(canonical_bytes(row))`
/// (architecture §6.1: an entry's own head hash is what the next entry's
/// `prev_entry_hash` must equal) — using only the module's public `canonical_bytes`, so
/// this test file never depends on a private implementation detail. Correctness of this
/// reconstruction is verified indirectly: `append_with_correctly_persisted_head_unlocks_clean`
/// only passes if this hash matches what `verify_against_head` independently computes
/// internally and calls `VerifyOutcome::Clean`.
fn persist_head_for(keystore: &Arc<InMemoryKeystore>, row: &pg_core::audit::AuditRow) {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(pg_core::audit::canonical_bytes(row));
    let head_hash: [u8; 32] = hasher.finalize().into();

    let mut item = keystore.load().expect("load").expect("item exists");
    item.audit_head = pg_core::keystore::AuditHead {
        sequence: row.sequence,
        head_hash,
    };
    keystore.store(&item).expect("store updated head");
}

// ---------------------------------------------------------------------------
// dev-plan W5: "flip payload byte → degraded, no document decrypt (none exist yet)"
// testing.md §8: "NFR-R1 tamper"
// ---------------------------------------------------------------------------

#[test]
fn flip_a_payload_byte_causes_degraded_integrity() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    let row = append_row_returning(&vault, &key, "{}");
    persist_head_for(&keystore, &row);

    // Corrupt while the vault is still open — the point under test is what the *next*
    // unlock's replay finds, not whether the vault happens to be open or closed at the
    // moment of corruption (an attacker with the file has it closed; this test's own
    // corruption helper only works on an open connection either way).
    vault
        .test_only_corrupt_payload(1)
        .expect("corrupt payload");
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let integrity = out.integrity.expect("degraded unlock carries a report");
    assert!(!integrity.ok);
    assert_eq!(integrity.kind, "modification");
    assert_eq!(integrity.first_bad_sequence, Some(1));

    // "no document decrypt": no document commands exist yet (dev-plan W5 parenthetical),
    // so the structural claim available at this chunk is that the degraded session still
    // reports itself as degraded on a second read, not "unlocked".
    assert_eq!(
        mgr.get_integrity_report().expect("get_integrity_report").kind,
        "modification"
    );
}

// ---------------------------------------------------------------------------
// dev-plan W5: "truncation vs head"; testing.md §8: "NFR-R1 truncation"
// ---------------------------------------------------------------------------

#[test]
fn truncated_tail_below_persisted_head_causes_degraded_integrity() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    let row1 = append_row_returning(&vault, &key, "{}");
    let row2 = append_row_returning(&vault, &key, "{}");
    persist_head_for(&keystore, &row2);

    // Truncate back to just sequence 1 — the persisted head (sequence 2) now points past
    // the end of what replays. Done while still open, same reasoning as the payload-flip
    // test above.
    vault.test_only_truncate_after(1).expect("truncate");
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let integrity = out.integrity.expect("degraded unlock carries a report");
    assert!(!integrity.ok);
    assert_eq!(integrity.kind, "truncation");
    assert_eq!(integrity.head_sequence, 2);
    assert_eq!(integrity.tail_sequence, 1);
    let _ = row1;
}

/// The extreme case of the same rule: every row truncated away, so the replay is entirely
/// empty, against a persisted head that is not genesis.
#[test]
fn fully_truncated_chain_against_a_non_genesis_head_causes_degraded_integrity() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    let row = append_row_returning(&vault, &key, "{}");
    persist_head_for(&keystore, &row);
    vault.test_only_truncate_after(0).expect("truncate everything");
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let integrity = out.integrity.expect("degraded unlock carries a report");
    assert_eq!(integrity.kind, "truncation");
    assert_eq!(integrity.head_sequence, 1);
    assert_eq!(integrity.tail_sequence, 0);
    assert_eq!(integrity.first_bad_sequence, Some(1));
}

// ---------------------------------------------------------------------------
// dev-plan W5: "crash window fast-forward still unlocked"; testing.md §8: "Crash window"
// ---------------------------------------------------------------------------

#[test]
fn crash_window_fast_forward_after_unpersisted_appends_still_unlocks() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    // Append rows to the DB but never touch the keystore's persisted head — exactly
    // "DB committed, keystore persist not yet done" (decision 0004's crash window).
    for _ in 0..5 {
        append_row_returning(&vault, &key, "{}");
    }
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    assert_eq!(
        out.state,
        SessionState::Unlocked,
        "crash-window fast-forward reports \"unlocked\", not a fourth state"
    );
    assert!(out.integrity.is_none(), "api.md: integrity is non-null iff degraded_integrity");

    let report = mgr.get_integrity_report().expect("get_integrity_report");
    assert!(report.ok);
    assert_eq!(report.kind, "crash_window_fast_forwarded");
    assert_eq!(report.tail_sequence, 5);

    // The fast-forward must have persisted: the keystore's head_sequence now matches T.
    let item = keystore.load().expect("load").expect("item exists");
    assert_eq!(item.audit_head.sequence, 5);
}

/// If persisting a fast-forwarded head fails, `unlock` must not leave the vault open
/// behind a `Locked`-reporting session — architecture §3.3's "Passphrase failure zeroizes
/// and refuses (no partial open)" is a property of the *session*, and a vault opened but
/// never installed into `self.open` because a later step failed is exactly a partial open
/// by another name. A retry (once the keystore is healthy again) must still be able to
/// unlock cleanly, which it could not if the stale-open connection were left blocking a
/// fresh open of the same file.
#[test]
fn failed_fast_forward_persist_does_not_leave_the_vault_open() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);
    for _ in 0..3 {
        append_row_returning(&vault, &key, "{}");
    }
    mgr.lock().expect("lock");
    assert!(!vault.is_open(), "lock must have closed the vault");

    keystore.fail_next_store();
    let err = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect_err("the injected keystore failure on the fast-forward persist must surface");
    assert_eq!(err.code, pg_core::api::ErrorCode::Internal);

    assert!(
        !vault.is_open(),
        "a failed fast-forward persist must close the vault it had just opened, not leave \
         a live connection behind an errored, still-locked-reporting session"
    );
    assert_eq!(mgr.get_session_state().state, SessionState::Locked);

    // The keystore is healthy again (the injected failure was one-shot) — a retry must
    // succeed, proving the vault was actually released, not merely reported as closed.
    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("retry after the transient keystore failure must unlock cleanly");
    assert_eq!(out.state, SessionState::Unlocked);
}

/// architecture §6.3: "`T.sequence == H.sequence + k` for k in 1..32" — 32 unpersisted
/// appends is still inside the window.
#[test]
fn crash_window_covers_up_to_32_unpersisted_appends() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    for _ in 0..32 {
        append_row_returning(&vault, &key, "{}");
    }
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    assert_eq!(out.state, SessionState::Unlocked);
    assert_eq!(
        keystore.load().expect("load").expect("item exists").audit_head.sequence,
        32
    );
}

/// One past the window (33) is not a crash window — architecture §6.3's literal range is
/// `1..32` for `k`, i.e. up to 32 inclusive per the `k in 1..32` prose read together with
/// `CRASH_WINDOW_MAX = 32`; 33 unpersisted appends must NOT fast-forward.
#[test]
fn thirty_three_unpersisted_appends_is_not_a_crash_window() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    for _ in 0..33 {
        append_row_returning(&vault, &key, "{}");
    }
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let _ = keystore;
}

/// architecture §6.3's fast-forward condition is three things, not two: chain valid,
/// every extra entry HMAC-valid, **and** "entry `H.sequence` matches `H.head_hash`". A
/// persisted head whose `sequence` sits inside the crash window but whose `head_hash`
/// does not match what the chain actually has at that sequence must still be an integrity
/// failure — otherwise a head that was corrupted (or forged) independently of the chain
/// itself could ride along inside an otherwise-legitimate crash-window gap.
#[test]
fn head_hash_mismatch_inside_the_crash_window_is_not_fast_forwarded() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);
    for _ in 0..5 {
        append_row_returning(&vault, &key, "{}");
    }

    // Persisted head claims sequence 2, `k = 3` (inside the window), but with a
    // `head_hash` that is not what row 2 actually hashes to.
    let mut item = keystore.load().expect("load").expect("item exists");
    item.audit_head = pg_core::keystore::AuditHead {
        sequence: 2,
        head_hash: [0x42u8; 32],
    };
    keystore.store(&item).expect("store");

    mgr.lock().expect("lock");
    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let integrity = out.integrity.expect("degraded unlock carries a report");
    assert_eq!(integrity.kind, "modification");
    assert_eq!(
        integrity.first_bad_sequence,
        Some(2),
        "the head-adjacent entry is the one that disagrees with the persisted head"
    );
}

// ---------------------------------------------------------------------------
// dev-plan W5: "HMAC break is not fast-forwarded"; testing.md §8: "Integrity vs crash"
// ---------------------------------------------------------------------------

#[test]
fn hmac_break_is_not_fast_forwarded_even_within_the_crash_window() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);

    for _ in 0..3 {
        append_row_returning(&vault, &key, "{}");
    }

    // Break the HMAC of the first row: a break mid-chain, well within the 1..32 window in
    // raw row *count*, must still fail — the crash window is for an honest gap, not a
    // license to skip verification.
    vault.test_only_corrupt_payload(1).expect("corrupt");
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let integrity = out.integrity.expect("degraded unlock carries a report");
    assert_eq!(integrity.kind, "modification");
    assert_eq!(integrity.first_bad_sequence, Some(1));
    let _ = keystore;
}

/// architecture §6.1: "An attacker who edits the DB without the vault key cannot produce
/// a valid HMAC." The sharpest version of that claim: a chain that is internally
/// *perfect* — contiguous sequences, every `prev_entry_hash` link correct (SHA-256 is
/// unkeyed, so an attacker without `audit_mac_key` can still compute those) — but signed
/// under a key that is not the real `audit_mac_key`, starting from a fresh vault (head at
/// genesis, well inside the crash window). The only thing that can catch this is the
/// `entry_signature` check itself; if that check were ever weakened, this test is what
/// would still notice.
#[test]
fn a_chain_forged_without_the_real_mac_key_is_not_fast_forwarded() {
    let (mut mgr, vault, _keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");

    let attacker_key = [0xAAu8; 32]; // deliberately not this session's audit_mac_key
    for _ in 0..3 {
        audit::append(
            vault.as_ref(),
            &attacker_key,
            EventType::Import,
            None,
            1_000,
            OriginalsFlag::Unset,
            "{}",
        )
        .expect("append (forged, but internally self-consistent)");
    }
    mgr.lock().expect("lock");

    let out = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(
        out.state,
        SessionState::DegradedIntegrity,
        "only the entry_signature check can reject a chain forged under the wrong key"
    );
    assert_eq!(
        out.integrity.expect("degraded unlock carries a report").first_bad_sequence,
        Some(1)
    );
}

// ---------------------------------------------------------------------------
// dev-plan W5: "degraded session cannot import/approve/share (C-API-6) — those commands
// fail even if not fully implemented."
// ---------------------------------------------------------------------------

/// Every command that would touch document content is unregistered in `SESSION_TABLE`
/// (none of import/approve/share exist as `SessionManager` methods yet), so
/// `command_allowed` refuses them in every state, `degraded_integrity` included — proving
/// C-API-6 holds structurally now, before those commands are implemented, rather than
/// leaving it to be checked only once they land.
#[test]
fn degraded_session_cannot_reach_unimplemented_document_commands() {
    for command in [
        "import_document",
        "open_approval",
        "set_field_decisions",
        "submit_approval",
        "abort_approval",
        "delete_document",
        "delete_retained_original",
        "list_variants",
        "get_variant",
        "save_variant",
        "delete_variant",
        "preview_share",
        "commit_share",
        "cloud_ai_set_config",
    ] {
        assert!(
            !command_allowed(command, SessionState::DegradedIntegrity),
            "{command} must be refused in degraded_integrity (C-API-6)"
        );
    }
}

/// `lock` and `get_account` remain available while degraded (api.md §2 table) — the
/// degraded session is not fully inert, only barred from document content.
#[test]
fn degraded_session_still_allows_lock_and_get_account() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);
    let row = append_row_returning(&vault, &key, "{}");
    persist_head_for(&keystore, &row);
    vault.test_only_corrupt_payload(1).expect("corrupt");
    mgr.lock().expect("lock");
    mgr.unlock(UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");
    assert_eq!(mgr.get_session_state().state, SessionState::DegradedIntegrity);

    assert!(mgr.get_account().is_ok(), "get_account allowed while degraded");
    assert!(mgr.lock().is_ok(), "lock allowed while degraded");
}

/// `change_passphrase` is explicitly "no" for `degraded_integrity` in api.md §2, unlike
/// `lock`/`get_account`.
#[test]
fn degraded_session_refuses_change_passphrase() {
    let (mut mgr, vault, keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let key = mac_key(&mgr);
    let row = append_row_returning(&vault, &key, "{}");
    persist_head_for(&keystore, &row);
    vault.test_only_corrupt_payload(1).expect("corrupt");
    mgr.lock().expect("lock");
    mgr.unlock(UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");

    let err = mgr
        .change_passphrase(pg_core::session::ChangePassphraseIn {
            current: PASSPHRASE.to_string(),
            new_passphrase: "another passphrase entirely".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::NotInSession);
}

// ---------------------------------------------------------------------------
// dev-plan W5: "Done when: get_integrity_report matches unlock outcome."
// ---------------------------------------------------------------------------

#[test]
fn get_integrity_report_matches_a_clean_create_account() {
    let (mut mgr, _vault, _keystore, _dir) = fresh_with_vault_and_audit();
    mgr.create_account(create_in()).expect("create_account");
    let report = mgr.get_integrity_report().expect("get_integrity_report");
    assert!(report.ok);
    assert_eq!(report.kind, "ok");
    assert_eq!(report.head_sequence, 0);
    assert_eq!(report.tail_sequence, 0);
}

#[test]
fn get_integrity_report_refused_before_unlock() {
    let (mgr, _vault, _keystore, _dir) = fresh_with_vault_and_audit();
    assert_eq!(
        mgr.get_integrity_report().unwrap_err().code,
        pg_core::api::ErrorCode::NotInSession
    );
}

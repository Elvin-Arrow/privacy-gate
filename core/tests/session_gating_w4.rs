//! W4 — Session gating table (commands that exist).
//!
//! Spec sources:
//! - `docs/specs/api.md` §2 (the session-state × command-group matrix, verbatim source of
//!   truth for every cell asserted here)
//! - `docs/specs/testing.md` "Session table" row (§6): "Every api.md §2 cell: allowed vs
//!   `not_in_session`"
//! - `docs/dev-plan.md` W4 ("Tests first: table-driven every cell for registered
//!   commands"; "Integrate: single gate in the command dispatcher"; "Done when: adding a
//!   command requires a new row in the table test (will fail until filled)")
//!
//! Out of W4 scope and deliberately absent here: `get_integrity_report`,
//! `list_audit_events`, and every document/approval/share/config/variant command — none of
//! them exist yet (dev-plan W4 "Do not: implement gated-but-unwritten commands").
//!
//! # Why this is table-driven against every state, including `degraded_integrity`
//!
//! No command in W2–W4 can put a live `SessionManager` into
//! `SessionState::DegradedIntegrity` — that only becomes reachable in W5
//! (`SessionManager::verify_integrity_on_unlock`). So this file tests the
//! `degraded_integrity` column two ways: end-to-end tests exercise the three states a real
//! `SessionManager` can actually reach (`first_run`, `locked`, `unlocked`); a single
//! data-level test (`every_api_md_2_cell_is_covered`) walks the full 5-command ×
//! 4-state matrix against `pg_core::session::command_allowed` (re-exported for this
//! purpose) so every cell in the spec table — including the ones no live session can
//! reach yet — is asserted against api.md §2's text, not left silently unverified.

use std::sync::Arc;

use pg_core::account::InMemoryAccountStore;
use pg_core::api::ErrorCode;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    command_allowed, ChangePassphraseIn, CreateAccountIn, SessionManager, SessionState, UnlockIn,
};

const PASSPHRASE: &str = "correct horse battery staple";

fn fresh() -> SessionManager {
    let keystore = Arc::new(InMemoryKeystore::new());
    let accounts = Arc::new(InMemoryAccountStore::new());
    SessionManager::new(keystore, accounts)
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

/// A `SessionManager` at each of the three reachable states, freshly built so tests don't
/// share mutable state through a shared fixture.
fn at_first_run() -> SessionManager {
    fresh()
}

fn at_locked() -> SessionManager {
    let mut mgr = fresh();
    mgr.create_account(create_in()).expect("create_account");
    mgr.lock().expect("lock");
    mgr
}

fn at_unlocked() -> SessionManager {
    let mut mgr = fresh();
    mgr.create_account(create_in()).expect("create_account");
    mgr
}

// ---------------------------------------------------------------------------
// Data-level: the full api.md §2 matrix, all 4 states × 5 registered commands, asserted
// directly against the table dev-plan W4 asks the dispatcher to gate on.
// ---------------------------------------------------------------------------

#[test]
fn every_api_md_2_cell_is_covered() {
    use SessionState::{DegradedIntegrity, FirstRun, Locked, Unlocked};

    // (command, first_run, locked, unlocked, degraded_integrity) — api.md §2, including
    // every command registered since W4 (testing.md §5.3 session gating table).
    let expected: &[(&str, bool, bool, bool, bool)] = &[
        ("create_account", true, false, false, false),
        ("unlock", false, true, false, false),
        ("lock", false, false, true, true),
        ("change_passphrase", false, false, true, false),
        ("get_account", false, false, true, true),
        ("get_integrity_report", false, false, true, true),
        ("list_audit_events", false, false, true, true),
        ("get_retention_default", false, false, true, false),
        ("set_retention_default", false, false, true, false),
        ("get_detector_preference", false, false, true, false),
        ("set_detector_preference", false, false, true, false),
        ("import_document", false, false, true, false),
        ("list_documents", false, false, true, false),
        ("get_document", false, false, true, false),
        ("open_approval", false, false, true, false),
        ("get_approval_view", false, false, true, false),
        ("set_field_decisions", false, false, true, false),
        ("submit_approval", false, false, true, false),
        ("abort_approval", false, false, true, false),
        ("delete_document", false, false, true, false),
        ("delete_retained_original", false, false, true, false),
        ("list_variants", false, false, true, false),
        ("get_variant", false, false, true, false),
        ("save_variant", false, false, true, false),
        ("delete_variant", false, false, true, false),
        ("preview_share", false, false, true, false),
        ("commit_share", false, false, true, false),
        ("cloud_ai_set_config", false, false, true, false),
        ("cloud_ai_get_config", false, false, true, false),
        ("cloud_ai_clear_config", false, false, true, false),
        ("cloud_ai_test", false, false, true, false),
    ];

    for &(command, first_run, locked, unlocked, degraded) in expected {
        assert_eq!(
            command_allowed(command, FirstRun),
            first_run,
            "{command} × first_run"
        );
        assert_eq!(command_allowed(command, Locked), locked, "{command} × locked");
        assert_eq!(
            command_allowed(command, Unlocked),
            unlocked,
            "{command} × unlocked"
        );
        assert_eq!(
            command_allowed(command, DegradedIntegrity),
            degraded,
            "{command} × degraded_integrity"
        );
    }
}

/// dev-plan W4: "adding a command requires a new row in the table test (will fail until
/// filled)." A command name with no row is refused in every state — this is the
/// fail-closed default the table gives an un-added command, proven directly.
#[test]
fn an_unregistered_command_name_is_refused_in_every_state() {
    for state in [
        SessionState::FirstRun,
        SessionState::Locked,
        SessionState::Unlocked,
        SessionState::DegradedIntegrity,
    ] {
        assert!(!command_allowed("some_future_command", state));
    }
}

/// `get_session_state` has no row at all (api.md §2: callable in every state) — confirm
/// the table's absence-means-refused default does not accidentally apply to it by testing
/// the one command that must never be gated, end to end, in the state where every other
/// gated command in this table is refused.
#[test]
fn get_session_state_has_no_gate_even_in_first_run() {
    let mgr = at_first_run();
    assert_eq!(mgr.get_session_state().state, SessionState::FirstRun);
}

// ---------------------------------------------------------------------------
// End-to-end: every command, at every state a live SessionManager can actually reach
// (first_run, locked, unlocked), through the real gate in session.rs.
// ---------------------------------------------------------------------------

#[test]
fn create_account_cell_first_run_allowed() {
    let mut mgr = at_first_run();
    assert!(mgr.create_account(create_in()).is_ok());
}

#[test]
fn create_account_cell_locked_refused() {
    let mut mgr = at_locked();
    let err = mgr.create_account(create_in()).unwrap_err();
    // api.md §3: this cell's specific code is `account_exists`, not the generic
    // `not_in_session` — the table only decides allowed/refused; the error on a refused
    // cell is still command-specific where api.md documents one.
    assert_eq!(err.code, ErrorCode::AccountExists);
}

#[test]
fn create_account_cell_unlocked_refused() {
    let mut mgr = at_unlocked();
    let err = mgr.create_account(create_in()).unwrap_err();
    assert_eq!(err.code, ErrorCode::AccountExists);
}

#[test]
fn unlock_cell_first_run_refused() {
    let mut mgr = at_first_run();
    let err = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

#[test]
fn unlock_cell_locked_allowed() {
    let mut mgr = at_locked();
    assert!(mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .is_ok());
}

#[test]
fn unlock_cell_unlocked_refused() {
    let mut mgr = at_unlocked();
    let err = mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

#[test]
fn lock_cell_first_run_refused() {
    let mut mgr = at_first_run();
    assert_eq!(mgr.lock().unwrap_err().code, ErrorCode::NotInSession);
}

#[test]
fn lock_cell_locked_refused() {
    let mut mgr = at_locked();
    assert_eq!(mgr.lock().unwrap_err().code, ErrorCode::NotInSession);
}

#[test]
fn lock_cell_unlocked_allowed() {
    let mut mgr = at_unlocked();
    assert!(mgr.lock().is_ok());
}

#[test]
fn change_passphrase_cell_first_run_refused() {
    let mut mgr = at_first_run();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.to_string(),
            new_passphrase: "another passphrase entirely".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

#[test]
fn change_passphrase_cell_locked_refused() {
    let mut mgr = at_locked();
    let err = mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.to_string(),
            new_passphrase: "another passphrase entirely".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

#[test]
fn change_passphrase_cell_unlocked_allowed() {
    let mut mgr = at_unlocked();
    assert!(mgr
        .change_passphrase(ChangePassphraseIn {
            current: PASSPHRASE.to_string(),
            new_passphrase: "another passphrase entirely".to_string(),
        })
        .is_ok());
}

#[test]
fn get_account_cell_first_run_refused() {
    let mgr = at_first_run();
    assert_eq!(mgr.get_account().unwrap_err().code, ErrorCode::NotInSession);
}

#[test]
fn get_account_cell_locked_refused() {
    let mgr = at_locked();
    assert_eq!(mgr.get_account().unwrap_err().code, ErrorCode::NotInSession);
}

#[test]
fn get_account_cell_unlocked_allowed() {
    let mgr = at_unlocked();
    assert!(mgr.get_account().is_ok());
}

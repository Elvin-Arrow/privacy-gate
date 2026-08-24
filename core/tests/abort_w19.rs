//! W19 — `abort_approval` and lock vs retention.
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.4 (`abort_approval`; retain keeps catalog; discard deletes
//!   the row; core must not serve approval view after abort)
//! - `docs/specs/data-model.md` §8 (`lock` or `abort` while discard and not approved →
//!   delete `document` + kind=8; retain original remains)
//! - `docs/specs/architecture.md` §5.2 (no review payload after Aborted)
//! - `docs/dev-plan.md` W19 ("Tests first: both retention paths; span text gone after
//!   abort (no `get_approval_view`).")
//!
//! Seam: [`SessionManager::abort_approval`] and [`SessionManager::lock`]. Catalog presence
//! is observed through `get_document` / `open_approval` / `list_documents`.
//!
//! Explicitly **not** in this chunk: UI copy (W33), `delete_document` DEK destroy (W20).

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::AuditStore;
use pg_core::catalog::{DocumentStore, EffectiveRetention};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    command_allowed, AbortApprovalIn, CreateAccountIn, GetApprovalViewIn, GetDocumentIn,
    ImportDocumentIn, OpenApprovalIn, SessionManager, SessionState, SetRetentionDefaultIn,
    SubmitApprovalIn, UnlockIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const BODY: &[u8] = b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.";

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    (dir, path)
}

fn fresh_confirmed() -> (SessionManager, tempfile::TempDir) {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault;
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents)
        .with_detector(Arc::new(StubDetector));
    mgr.create_account(create_in()).expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm retention");
    (mgr, dir)
}

fn import_letter(mgr: &mut SessionManager, retention: Option<EffectiveRetention>) -> String {
    mgr.import_document(ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: BODY.to_vec(),
        retention_override: retention,
    })
    .expect("import")
    .summary
    .doc_id
}

fn unlock(mgr: &mut SessionManager) {
    mgr.unlock(UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");
}

// ---------------------------------------------------------------------------
// api.md §2 gating
// ---------------------------------------------------------------------------

#[test]
fn abort_approval_unlocked_only() {
    assert!(!command_allowed("abort_approval", SessionState::FirstRun));
    assert!(!command_allowed("abort_approval", SessionState::Locked));
    assert!(command_allowed("abort_approval", SessionState::Unlocked));
    assert!(!command_allowed(
        "abort_approval",
        SessionState::DegradedIntegrity
    ));
}

#[test]
fn abort_unknown_session_is_not_found() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr
        .abort_approval(AbortApprovalIn {
            approval_session_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

// ---------------------------------------------------------------------------
// Discard path
// ---------------------------------------------------------------------------

#[test]
fn abort_discard_drops_catalog_and_approval_view() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr, None);
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .expect("open");
    let out = mgr
        .abort_approval(AbortApprovalIn {
            approval_session_id: view.approval_session_id.clone(),
        })
        .expect("abort");
    assert_eq!(out.lifecycle, pg_core::session::ApprovalLifecycle::Aborted);

    let view_err = mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: view.approval_session_id,
        })
        .unwrap_err();
    assert_eq!(view_err.code, ErrorCode::NotFound);

    let get_err = mgr
        .get_document(GetDocumentIn { doc_id: doc_id.clone() })
        .unwrap_err();
    assert_eq!(get_err.code, ErrorCode::NotFound);

    let open_err = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .unwrap_err();
    assert_eq!(open_err.code, ErrorCode::NotFound);
}

#[test]
fn lock_drops_unapproved_discard_from_catalog() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr, None);
    mgr.lock().expect("lock");
    unlock(&mut mgr);
    let err = mgr
        .get_document(GetDocumentIn { doc_id })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn lock_keeps_approved_discard_document() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr, None);
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .expect("open");
    let decisions: Vec<_> = view
        .fields
        .iter()
        .map(|f| pg_core::session::FieldDecisionDto {
            field_id: f.id.clone(),
            decision: pg_core::session::FieldDecisionKind::Redact,
        })
        .collect();
    mgr.set_field_decisions(pg_core::session::SetFieldDecisionsIn {
        approval_session_id: view.approval_session_id.clone(),
        decisions,
    })
    .expect("decide");
    mgr.submit_approval(SubmitApprovalIn {
        approval_session_id: view.approval_session_id,
    })
    .expect("submit");
    mgr.lock().expect("lock");
    unlock(&mut mgr);
    let got = mgr
        .get_document(GetDocumentIn { doc_id })
        .expect("approved discard survives lock");
    assert!(got.summary.has_approved_version);
}

// ---------------------------------------------------------------------------
// Retain path
// ---------------------------------------------------------------------------

#[test]
fn abort_retain_keeps_catalog_and_allows_reopen() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr, Some(EffectiveRetention::Retain));
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .expect("open");
    mgr.abort_approval(AbortApprovalIn {
        approval_session_id: view.approval_session_id.clone(),
    })
    .expect("abort");

    let view_err = mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: view.approval_session_id,
        })
        .unwrap_err();
    assert_eq!(view_err.code, ErrorCode::NotFound);

    let summary = mgr
        .get_document(GetDocumentIn {
            doc_id: doc_id.clone(),
        })
        .expect("retain catalog remains")
        .summary;
    assert!(!summary.has_approved_version);
    assert!(summary.has_retained_original);

    let reopened = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("reopen after abort retain");
    assert!(
        reopened
            .fields
            .iter()
            .any(|f| f.span.text.as_deref() == Some("PG-CANARY-X1")),
        "reopen must still show span text for a new consent step"
    );
}

#[test]
fn lock_keeps_unapproved_retain_and_allows_open_after_unlock() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr, Some(EffectiveRetention::Retain));
    mgr.lock().expect("lock");
    unlock(&mut mgr);
    let got = mgr
        .get_document(GetDocumentIn {
            doc_id: doc_id.clone(),
        })
        .expect("retain unapproved survives lock");
    assert!(!got.summary.has_approved_version);
    assert!(got.summary.has_retained_original);
    let view = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open_approval after lock reconstructs from retained original");
    assert!(
        view.fields
            .iter()
            .any(|f| f.span.text.as_deref() == Some("PG-CANARY-X1")),
        "reconstructed view must include canary span text"
    );
}

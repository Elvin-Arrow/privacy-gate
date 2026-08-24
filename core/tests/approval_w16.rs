//! W16 — Approval session (`open_approval` / `get_approval_view` / `set_field_decisions`).
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.4 (one session per process; `ApprovalView`; lifecycle)
//! - `docs/specs/api.md` §4 (`DetectedFieldDto.span.text` only on approval commands)
//! - `docs/specs/testing.md` C-API-2
//! - `docs/specs/design.md` §2.3 (Approval Engine; C-DES-1)
//! - `docs/dev-plan.md` W16 ("Tests first: `approval_busy`; span text on view, absent on
//!   `get_document`; partial decisions leave `awaiting_decisions`; all decided →
//!   `decided`; `approval_bad_state` on wrong lifecycle.")
//!
//! Seam: [`SessionManager`] approval commands. `submit_approval` is W18; `abort_approval`
//! / lock-vs-retention catalog deletion is W19. `already_approved` after submit is W18
//! (this chunk still refuses `open_approval` when `has_approved_version` is already true).
//!
//! AC tests install [`StubDetector`] so canary spans are locatable independently of
//! hybrid/Ollama selection.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::AuditStore;
use pg_core::catalog::DocumentStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    command_allowed, CreateAccountIn, FieldDecisionDto, FieldDecisionKind, GetApprovalViewIn,
    GetDocumentIn, ImportDocumentIn, OpenApprovalIn, SessionManager, SessionState,
    SetFieldDecisionsIn, SetRetentionDefaultIn,
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

fn import_letter(mgr: &mut SessionManager) -> String {
    mgr.import_document(ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: BODY.to_vec(),
        retention_override: None,
    })
    .expect("import")
    .summary
    .doc_id
}

// ---------------------------------------------------------------------------
// api.md §2 gating
// ---------------------------------------------------------------------------

#[test]
fn approval_commands_unlocked_only() {
    for command in [
        "open_approval",
        "get_approval_view",
        "set_field_decisions",
    ] {
        assert!(!command_allowed(command, SessionState::FirstRun));
        assert!(!command_allowed(command, SessionState::Locked));
        assert!(command_allowed(command, SessionState::Unlocked));
        assert!(!command_allowed(command, SessionState::DegradedIntegrity));
    }
}

#[test]
fn open_approval_refused_before_unlock() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let keystore = Arc::new(InMemoryKeystore::new());
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault;
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents);
    assert_eq!(
        mgr.open_approval(OpenApprovalIn {
            doc_id: "00000000-0000-4000-8000-000000000001".to_string(),
        })
        .unwrap_err()
        .code,
        ErrorCode::NotInSession
    );
    let _dir = dir;
}

// ---------------------------------------------------------------------------
// open_approval
// ---------------------------------------------------------------------------

#[test]
fn open_approval_unknown_doc_is_not_found() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr
        .open_approval(OpenApprovalIn {
            doc_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn open_approval_view_includes_span_text_and_pages() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    let view = mgr
        .open_approval(OpenApprovalIn { doc_id: doc_id.clone() })
        .expect("open_approval");
    assert_eq!(view.doc_id, doc_id);
    assert_eq!(view.lifecycle, pg_core::session::ApprovalLifecycle::AwaitingDecisions);
    assert!(!view.approval_session_id.is_empty());
    assert_eq!(view.fields.len(), 2);
    for field in &view.fields {
        let text = field.span.text.as_deref().expect("C-API-2: text on approval view");
        assert!(text.contains("PG-CANARY-"), "{text}");
    }
    assert!(!view.pages.is_empty());
    let page_text: String = view.pages[0]
        .spans
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(page_text.contains("PG-CANARY-X1"));
}

#[test]
fn get_document_does_not_include_span_or_field_text() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    let got = mgr
        .get_document(GetDocumentIn { doc_id })
        .expect("get_document");
    let serialized = serde_json::to_value(&got.summary).expect("serialize");
    let obj = serialized.as_object().expect("object");
    assert!(!obj.contains_key("pages"));
    assert!(!obj.contains_key("fields"));
    let dump = serialized.to_string();
    assert!(
        !dump.contains("PG-CANARY-"),
        "C-API-2: catalog summary must not carry span text, got {dump}"
    );
}

#[test]
fn second_open_approval_is_approval_busy() {
    let (mut mgr, _dir) = fresh_confirmed();
    let first = import_letter(&mut mgr);
    let second = mgr
        .import_document(ImportDocumentIn {
            filename: "other.txt".to_string(),
            bytes: b"PG-CANARY-Y1 only".to_vec(),
            retention_override: None,
        })
        .expect("second import")
        .summary
        .doc_id;
    mgr.open_approval(OpenApprovalIn { doc_id: first })
        .expect("first open");
    let err = mgr
        .open_approval(OpenApprovalIn { doc_id: second })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ApprovalBusy);
}

#[test]
fn reopen_same_doc_while_session_active_is_still_busy() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    mgr.open_approval(OpenApprovalIn {
        doc_id: doc_id.clone(),
    })
    .expect("open");
    let err = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ApprovalBusy);
}

// ---------------------------------------------------------------------------
// get_approval_view
// ---------------------------------------------------------------------------

#[test]
fn get_approval_view_returns_the_open_session() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    let opened = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open");
    let again = mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: opened.approval_session_id.clone(),
        })
        .expect("get_approval_view");
    assert_eq!(again.approval_session_id, opened.approval_session_id);
    assert_eq!(again.doc_id, opened.doc_id);
    assert_eq!(again.fields.len(), opened.fields.len());
}

#[test]
fn get_approval_view_unknown_session_is_not_found() {
    let (mgr, _dir) = fresh_confirmed();
    let err = mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

// ---------------------------------------------------------------------------
// set_field_decisions
// ---------------------------------------------------------------------------

#[test]
fn partial_decisions_leave_awaiting_decisions() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    let view = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open");
    let one = &view.fields[0];
    let out = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: view.approval_session_id.clone(),
            decisions: vec![FieldDecisionDto {
                field_id: one.id.clone(),
                decision: FieldDecisionKind::Redact,
            }],
        })
        .expect("set one");
    assert_eq!(out.lifecycle, pg_core::session::ApprovalLifecycle::AwaitingDecisions);
    assert_eq!(out.unresolved_field_ids.len(), 1);
    assert_eq!(out.unresolved_field_ids[0], view.fields[1].id);
    let refreshed = mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: view.approval_session_id,
        })
        .expect("refresh");
    assert_eq!(
        refreshed.lifecycle,
        pg_core::session::ApprovalLifecycle::AwaitingDecisions
    );
}

#[test]
fn all_fields_decided_moves_lifecycle_to_decided() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    let view = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open");
    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| FieldDecisionDto {
            field_id: f.id.clone(),
            decision: FieldDecisionKind::KeepVisible,
        })
        .collect();
    let out = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: view.approval_session_id.clone(),
            decisions,
        })
        .expect("set all");
    assert_eq!(out.lifecycle, pg_core::session::ApprovalLifecycle::Decided);
    assert!(out.unresolved_field_ids.is_empty());
    let refreshed = mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: view.approval_session_id,
        })
        .expect("refresh");
    assert_eq!(refreshed.lifecycle, pg_core::session::ApprovalLifecycle::Decided);
}

#[test]
fn unknown_field_id_is_invalid_input() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    let view = mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open");
    let err = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: view.approval_session_id,
            decisions: vec![FieldDecisionDto {
                field_id: "00000000-0000-4000-8000-000000000099".to_string(),
                decision: FieldDecisionKind::Redact,
            }],
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn set_field_decisions_unknown_session_is_not_found() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: "00000000-0000-4000-8000-000000000099".to_string(),
            decisions: vec![],
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

/// Wrong lifecycle for these commands is Committed/Aborted (W18/W19). W16 can still
/// prove `set_field_decisions` is refused when no session is active with a *stale* id
/// after lock dropped the RAM session — that's `not_found`, not `approval_bad_state`.
/// `approval_bad_state` is reserved for a live session whose lifecycle no longer accepts
/// the command; this test locks that contract for `set_field_decisions` by using a
/// session id that exists on the wire shape but is not the active one while another
/// session is open — still `not_found`. A dedicated Committed/Aborted case lands with
/// submit/abort.
#[test]
fn set_field_decisions_wrong_session_id_while_busy_is_not_found() {
    let (mut mgr, _dir) = fresh_confirmed();
    let doc_id = import_letter(&mut mgr);
    mgr.open_approval(OpenApprovalIn { doc_id })
        .expect("open");
    let err = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: "00000000-0000-4000-8000-000000000099".to_string(),
            decisions: vec![],
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

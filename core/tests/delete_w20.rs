//! W20 — `delete_document` (DEK destroy).
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.3 (`delete_document`; audit `delete`)
//! - `docs/specs/architecture.md` §4.3 (overwrite-and-drop wrapped DEK; do not rely on
//!   VACUUM)
//! - `docs/specs/testing.md` §8 DEK destroy row (Vault load fails; wrapped DEK absent;
//!   do **not** decrypt old ciphertext with a pre-copied DEK)
//! - `docs/specs/data-model.md` §7 delete order
//! - `docs/dev-plan.md` W20
//!
//! Seam: [`SessionManager::delete_document`] plus catalog load / artifact-count oracles.
//! Pre-copied DEK decrypt is deliberately not an assertion.
//!
//! Explicitly **not** in this chunk: OS secure-erase of whole disk; `delete_retained_original`
//! (W21).

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::{AuditStore, EventType};
use pg_core::catalog::{DocumentStore, EffectiveRetention};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keys::unwrap_master_key;
use pg_core::keystore::{InMemoryKeystore, KeystoreBackend};
use pg_core::session::{
    command_allowed, CreateAccountIn, DeleteDocumentIn, FieldDecisionDto, FieldDecisionKind,
    GetDocumentIn, ImportDocumentIn, OpenApprovalIn, SessionManager, SessionState,
    SetFieldDecisionsIn, SetRetentionDefaultIn, SubmitApprovalIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const BODY: &[u8] = b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.";

struct Wired {
    mgr: SessionManager,
    vault: Arc<SqlCipherVault>,
    keystore: Arc<InMemoryKeystore>,
    _dir: tempfile::TempDir,
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

fn fresh_confirmed() -> Wired {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let mut mgr = SessionManager::new_full(keystore.clone(), accounts, backend, audit, config)
        .with_documents(documents)
        .with_detector(Arc::new(StubDetector));
    mgr.create_account(create_in()).expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm");
    Wired {
        mgr,
        vault,
        keystore,
        _dir: dir,
    }
}

fn import(mgr: &mut SessionManager, retention: Option<EffectiveRetention>) -> String {
    mgr.import_document(ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: BODY.to_vec(),
        retention_override: retention,
    })
    .expect("import")
    .summary
    .doc_id
}

fn approve(mgr: &mut SessionManager, doc_id: &str) {
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.to_string(),
        })
        .expect("open");
    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| FieldDecisionDto {
            field_id: f.id.clone(),
            decision: FieldDecisionKind::Redact,
        })
        .collect();
    mgr.set_field_decisions(SetFieldDecisionsIn {
        approval_session_id: view.approval_session_id.clone(),
        decisions,
    })
    .expect("decide");
    mgr.submit_approval(SubmitApprovalIn {
        approval_session_id: view.approval_session_id,
    })
    .expect("submit");
}

fn master_of(wired: &Wired) -> pg_core::keys::VaultMasterKey {
    let item = wired.keystore.load().expect("load").expect("item");
    unwrap_master_key(PASSPHRASE, &item).expect("unwrap")
}

#[test]
fn delete_document_unlocked_only() {
    assert!(!command_allowed("delete_document", SessionState::FirstRun));
    assert!(!command_allowed("delete_document", SessionState::Locked));
    assert!(command_allowed("delete_document", SessionState::Unlocked));
    assert!(!command_allowed(
        "delete_document",
        SessionState::DegradedIntegrity
    ));
}

#[test]
fn delete_unknown_doc_is_not_found() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .delete_document(DeleteDocumentIn {
            doc_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn delete_approved_document_is_not_found_and_artifacts_gone() {
    let mut wired = fresh_confirmed();
    let doc_id = import(&mut wired.mgr, None);
    approve(&mut wired.mgr, &doc_id);
    assert!(
        wired.vault.test_only_artifact_count(&doc_id).expect("count") > 0,
        "approved document must have stored artifacts before delete"
    );
    let out = wired
        .mgr
        .delete_document(DeleteDocumentIn {
            doc_id: doc_id.clone(),
        })
        .expect("delete_document");
    assert!(out.ok);

    let get_err = wired
        .mgr
        .get_document(GetDocumentIn {
            doc_id: doc_id.clone(),
        })
        .unwrap_err();
    assert_eq!(get_err.code, ErrorCode::NotFound);
    let open_err = wired
        .mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .unwrap_err();
    assert_eq!(open_err.code, ErrorCode::NotFound);

    let master = master_of(&wired);
    assert!(wired
        .vault
        .load_meta(&master, &doc_id)
        .expect("load_meta")
        .is_none());
    assert!(wired
        .vault
        .load_approved(&master, &doc_id)
        .expect("load_approved")
        .is_none());
    assert_eq!(
        wired.vault.test_only_artifact_count(&doc_id).expect("count"),
        0,
        "wrapped DEK rows must be absent after overwrite-and-drop"
    );
}

#[test]
fn delete_retained_original_is_also_unreadable() {
    let mut wired = fresh_confirmed();
    let doc_id = import(&mut wired.mgr, Some(EffectiveRetention::Retain));
    approve(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .delete_document(DeleteDocumentIn {
            doc_id: doc_id.clone(),
        })
        .expect("delete");
    let original = wired
        .vault
        .load_original(&master_of(&wired), &doc_id)
        .expect("load_original");
    assert!(original.is_none());
}

#[test]
fn delete_appends_audit_without_span_text() {
    let mut wired = fresh_confirmed();
    let doc_id = import(&mut wired.mgr, None);
    approve(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .delete_document(DeleteDocumentIn {
            doc_id: doc_id.clone(),
        })
        .expect("delete");
    let rows = wired.vault.replay().expect("replay");
    let delete = rows
        .iter()
        .find(|r| r.event_type == EventType::Delete)
        .expect("delete event");
    assert_eq!(delete.doc_id.as_deref(), Some(doc_id.as_str()));
    assert!(
        !delete.payload_jcs.contains("PG-CANARY-"),
        "delete payload must not carry document text, got {}",
        delete.payload_jcs
    );
    let payload: serde_json::Value =
        serde_json::from_str(&delete.payload_jcs).expect("json");
    assert_eq!(payload["doc_id"].as_str(), Some(doc_id.as_str()));
}

//! W21 — `delete_retained_original`.
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.3 (idempotent if already discarded; audit `discard_original`
//!   if an original was present)
//! - `docs/specs/testing.md` §8 DEK destroy (original unreadable; do not use a pre-copied
//!   DEK as the oracle)
//! - `docs/dev-plan.md` W21 ("Tests first: retain → delete original → approved remains;
//!   second call ok.")
//!
//! Seam: [`SessionManager::delete_retained_original`]. Canonical approved bytes are
//! checked by decrypting the kind=1 artifact, not by re-deriving redaction.
//!
//! Explicitly **not** in this chunk: changing approved content; variants (W22).

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
    command_allowed, CreateAccountIn, DeleteRetainedOriginalIn, FieldDecisionDto,
    FieldDecisionKind, GetDocumentIn, ImportDocumentIn, OpenApprovalIn, SessionManager,
    SessionState, SetFieldDecisionsIn, SetRetentionDefaultIn, SubmitApprovalIn,
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

fn import_retain(mgr: &mut SessionManager) -> String {
    mgr.import_document(ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: BODY.to_vec(),
        retention_override: Some(EffectiveRetention::Retain),
    })
    .expect("import")
    .summary
    .doc_id
}

fn approve_x1_redact_x2_keep(mgr: &mut SessionManager, doc_id: &str) {
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.to_string(),
        })
        .expect("open");
    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| {
            let text = f.span.text.as_deref().unwrap_or("");
            FieldDecisionDto {
                field_id: f.id.clone(),
                decision: if text.contains("PG-CANARY-X1") {
                    FieldDecisionKind::Redact
                } else {
                    FieldDecisionKind::KeepVisible
                },
            }
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
fn delete_retained_original_unlocked_only() {
    assert!(!command_allowed("delete_retained_original", SessionState::FirstRun));
    assert!(!command_allowed("delete_retained_original", SessionState::Locked));
    assert!(command_allowed(
        "delete_retained_original",
        SessionState::Unlocked
    ));
    assert!(!command_allowed(
        "delete_retained_original",
        SessionState::DegradedIntegrity
    ));
}

#[test]
fn delete_retained_original_unknown_doc_is_not_found() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .delete_retained_original(DeleteRetainedOriginalIn {
            doc_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn retain_then_delete_original_leaves_approved_unchanged() {
    let mut wired = fresh_confirmed();
    let doc_id = import_retain(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let before = wired
        .vault
        .load_approved(&master_of(&wired), &doc_id)
        .expect("load")
        .expect("approved");
    let out = wired
        .mgr
        .delete_retained_original(DeleteRetainedOriginalIn {
            doc_id: doc_id.clone(),
        })
        .expect("delete original");
    assert!(!out.summary.has_retained_original);
    assert!(out.summary.has_approved_version);
    let got = wired
        .mgr
        .get_document(GetDocumentIn {
            doc_id: doc_id.clone(),
        })
        .expect("get")
        .summary;
    assert!(!got.has_retained_original);
    assert!(got.has_approved_version);
    assert!(wired
        .vault
        .load_original(&master_of(&wired), &doc_id)
        .expect("load_original")
        .is_none());
    let after = wired
        .vault
        .load_approved(&master_of(&wired), &doc_id)
        .expect("load")
        .expect("approved still present");
    assert_eq!(after, before, "canonical approved bytes must not change");
    let text: String = after
        .redacted_content
        .pages
        .iter()
        .flat_map(|p| p.spans.iter())
        .map(|s| s.text.as_str())
        .collect();
    assert!(!text.contains("PG-CANARY-X1"));
    assert!(text.contains("PG-CANARY-X2"));
}

#[test]
fn second_delete_retained_original_is_idempotent_without_a_second_audit() {
    let mut wired = fresh_confirmed();
    let doc_id = import_retain(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .delete_retained_original(DeleteRetainedOriginalIn {
            doc_id: doc_id.clone(),
        })
        .expect("first");
    wired
        .mgr
        .delete_retained_original(DeleteRetainedOriginalIn {
            doc_id: doc_id.clone(),
        })
        .expect("second");
    let discards = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .filter(|r| r.event_type == EventType::DiscardOriginal)
        .count();
    assert_eq!(discards, 1, "audit discard_original only when an original existed");
}

#[test]
fn already_discarded_original_does_not_audit() {
    let mut wired = fresh_confirmed();
    let doc_id = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: BODY.to_vec(),
            retention_override: None,
        })
        .expect("import discard")
        .summary
        .doc_id;
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .delete_retained_original(DeleteRetainedOriginalIn { doc_id })
        .expect("idempotent on discard");
    let discards = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .filter(|r| r.event_type == EventType::DiscardOriginal)
        .count();
    assert_eq!(discards, 0);
}

//! W22 — `list_variants` / `get_variant` / `save_variant` / `delete_variant`.
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.5 (names 1..=80; `variant_name_conflict`; `get_variant`
//!   overrides are field_id + decision only)
//! - `docs/specs/data-model.md` §6.4 / C-DM-4 (insert only if approved)
//! - `docs/specs/testing.md` variants row; C-API-2 (no span text on variants)
//! - `docs/dev-plan.md` W22 ("Tests first: create/apply-on-share later (W26);
//!   `variant_name_conflict`; delete.")
//!
//! Seam: [`SessionManager`] variant commands. Apply-on-share is W26.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::catalog::DocumentStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keys::unwrap_master_key;
use pg_core::keystore::{InMemoryKeystore, KeystoreBackend};
use pg_core::session::{
    command_allowed, CreateAccountIn, DeleteVariantIn, FieldDecisionDto, FieldDecisionKind,
    GetVariantIn, ImportDocumentIn, ListVariantsIn, OpenApprovalIn, SaveVariantIn, SessionManager,
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
    let audit: Arc<dyn pg_core::audit::AuditStore> = vault.clone();
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

fn import_discard(mgr: &mut SessionManager) -> String {
    mgr.import_document(ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: BODY.to_vec(),
        retention_override: None,
    })
    .expect("import")
    .summary
    .doc_id
}

fn approve_x1_redact_x2_keep(mgr: &mut SessionManager, doc_id: &str) -> Vec<FieldDecisionDto> {
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
        decisions: decisions.clone(),
    })
    .expect("decide");
    mgr.submit_approval(SubmitApprovalIn {
        approval_session_id: view.approval_session_id,
    })
    .expect("submit");
    decisions
}

fn master_of(wired: &Wired) -> pg_core::keys::VaultMasterKey {
    let item = wired.keystore.load().expect("load").expect("item");
    unwrap_master_key(PASSPHRASE, &item).expect("unwrap")
}

fn variant_commands() -> [&'static str; 4] {
    [
        "list_variants",
        "get_variant",
        "save_variant",
        "delete_variant",
    ]
}

#[test]
fn variant_commands_unlocked_only() {
    for command in variant_commands() {
        assert!(!command_allowed(command, SessionState::FirstRun), "{command}");
        assert!(!command_allowed(command, SessionState::Locked), "{command}");
        assert!(command_allowed(command, SessionState::Unlocked), "{command}");
        assert!(
            !command_allowed(command, SessionState::DegradedIntegrity),
            "{command} must be refused in degraded_integrity (C-API-6)"
        );
    }
}

#[test]
fn save_variant_unknown_doc_is_not_found() {
    let wired = fresh_confirmed();
    let err = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: "00000000-0000-4000-8000-000000000099".to_string(),
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn save_variant_unapproved_is_not_approved() {
    let mut wired = fresh_confirmed();
    let doc_id = import_discard(&mut wired.mgr);
    let err = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id,
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotApproved);
}

#[test]
fn save_variant_rejects_empty_and_too_long_names() {
    let mut wired = fresh_confirmed();
    let doc_id = import_discard(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let empty = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: doc_id.clone(),
            name: "   ".to_string(),
            overrides: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(empty.code, ErrorCode::InvalidInput);
    let too_long = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id,
            name: "x".repeat(81),
            overrides: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(too_long.code, ErrorCode::InvalidInput);
}

#[test]
fn save_list_get_variant_has_overrides_without_span_text() {
    let mut wired = fresh_confirmed();
    let doc_id = import_discard(&mut wired.mgr);
    let decisions = approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let x2 = decisions
        .iter()
        .find(|d| d.decision == FieldDecisionKind::KeepVisible)
        .expect("kept X2");
    let overrides = vec![FieldDecisionDto {
        field_id: x2.field_id.clone(),
        decision: FieldDecisionKind::Redact,
    }];
    let saved = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: doc_id.clone(),
            name: "  Client  ".to_string(),
            overrides: overrides.clone(),
        })
        .expect("save");
    assert_eq!(saved.name, "Client");
    assert!(!saved.variant_id.is_empty());
    assert!(saved.created_at.ends_with('Z'));

    let listed = wired
        .mgr
        .list_variants(ListVariantsIn {
            doc_id: doc_id.clone(),
        })
        .expect("list");
    assert_eq!(listed.variants.len(), 1);
    assert_eq!(listed.variants[0].variant_id, saved.variant_id);
    assert_eq!(listed.variants[0].name, "Client");

    let got = wired
        .mgr
        .get_variant(GetVariantIn {
            doc_id: doc_id.clone(),
            variant_id: saved.variant_id.clone(),
        })
        .expect("get");
    assert_eq!(got.name, "Client");
    assert_eq!(got.overrides, overrides);
    let json = serde_json::to_string(&got).expect("json");
    assert!(
        !json.contains("PG-CANARY"),
        "get_variant must not carry span text (C-API-2): {json}"
    );
    assert!(!json.contains("Dear Sir"), "get_variant must not carry body text");

    let approved = wired
        .vault
        .load_approved(&master_of(&wired), &doc_id)
        .expect("load")
        .expect("approved");
    assert_eq!(
        approved.decisions.len(),
        2,
        "saving a variant must not mutate the canonical approved version"
    );
}

#[test]
fn duplicate_variant_name_on_same_doc_is_conflict() {
    let mut wired = fresh_confirmed();
    let doc_id = import_discard(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: doc_id.clone(),
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .expect("first");
    let err = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id,
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::VariantNameConflict);
}

#[test]
fn same_variant_name_on_two_docs_is_allowed() {
    let mut wired = fresh_confirmed();
    let a = import_discard(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &a);
    let b = import_discard(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &b);
    wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: a,
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .expect("a");
    wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: b,
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .expect("b");
}

#[test]
fn delete_variant_makes_it_unreadable_and_leaves_approved() {
    let mut wired = fresh_confirmed();
    let doc_id = import_discard(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let before_artifacts = wired
        .vault
        .test_only_artifact_count(&doc_id)
        .expect("count");
    let saved = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: doc_id.clone(),
            name: "Client".to_string(),
            overrides: Vec::new(),
        })
        .expect("save");
    assert_eq!(
        wired
            .vault
            .test_only_artifact_count(&doc_id)
            .expect("count after save"),
        before_artifacts + 1
    );
    wired
        .mgr
        .delete_variant(DeleteVariantIn {
            doc_id: doc_id.clone(),
            variant_id: saved.variant_id.clone(),
        })
        .expect("delete");
    let err = wired
        .mgr
        .get_variant(GetVariantIn {
            doc_id: doc_id.clone(),
            variant_id: saved.variant_id,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(wired
        .mgr
        .list_variants(ListVariantsIn {
            doc_id: doc_id.clone(),
        })
        .expect("list")
        .variants
        .is_empty());
    assert_eq!(
        wired
            .vault
            .test_only_artifact_count(&doc_id)
            .expect("count after delete"),
        before_artifacts,
        "kind=3 row must be gone (testing.md §8 DEK destroy)"
    );
    assert!(wired
        .vault
        .load_approved(&master_of(&wired), &doc_id)
        .expect("load")
        .is_some());
}

#[test]
fn get_and_delete_unknown_variant_are_not_found() {
    let mut wired = fresh_confirmed();
    let doc_id = import_discard(&mut wired.mgr);
    approve_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let missing = "00000000-0000-4000-8000-000000000099".to_string();
    let get = wired
        .mgr
        .get_variant(GetVariantIn {
            doc_id: doc_id.clone(),
            variant_id: missing.clone(),
        })
        .unwrap_err();
    assert_eq!(get.code, ErrorCode::NotFound);
    let del = wired
        .mgr
        .delete_variant(DeleteVariantIn {
            doc_id,
            variant_id: missing,
        })
        .unwrap_err();
    assert_eq!(del.code, ErrorCode::NotFound);
}

//! W18 — `submit_approval` (AC-1 core).
//!
//! Spec sources:
//! - `docs/specs/testing.md` §6.1 AC-1 (import, detect, approve, store; `already_approved`;
//!   unlock after lock still returns approved metadata; original bytes never on output)
//! - `docs/specs/api.md` §5.4 (`submit_approval`; lifecycle `decided` required; core must
//!   not serve approval view after submit)
//! - `docs/specs/api.md` §6 `approve` payload (`field_id`/`label`/`decision`; no span text)
//! - `docs/specs/data-model.md` §6.3 / §8 (kind=1 `ApprovedVersion`; discard → no kind=2)
//! - `docs/specs/design.md` §2.1 (Vault ack → overwrite RAM original)
//! - `docs/dev-plan.md` W18 ("Tests first: AC-1 through store; discard original not
//!   decryptable; retain original still encrypted; second `open_approval` →
//!   `already_approved`.")
//!
//! Seam: [`SessionManager::submit_approval`] plus [`DocumentStore`] load of the sealed
//! artifacts (the independent check that redacted canaries are gone and retained originals
//! still decrypt). Stub detector so canary spans are locatable regardless of hybrid/Ollama.
//!
//! Explicitly **not** in this chunk: `abort_approval` / lock-vs-retention catalog deletion
//! (W19), PDF export (W23), variants, share.

use std::sync::Arc;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as LoDocument, Object, Stream};

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::{AuditStore, EventType};
use pg_core::catalog::{DocumentStore, EffectiveRetention};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::importer::SourceFormat;
use pg_core::keys::unwrap_master_key;
use pg_core::keystore::{InMemoryKeystore, KeystoreBackend};
use pg_core::session::{
    command_allowed, CreateAccountIn, FieldDecisionDto, FieldDecisionKind, GetApprovalViewIn,
    GetDocumentIn, ImportDocumentIn, OpenApprovalIn, SessionManager, SessionState,
    SetFieldDecisionsIn, SetRetentionDefaultIn, SubmitApprovalIn, UnlockIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const BODY: &[u8] = b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.";
/// Independent expected post-redaction text: X1 omitted, X2 kept (data-model §6.3).
const REDACTED_TEXT: &str = "Dear Sir,  and PG-CANARY-X2 both appear here.";

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

struct Wired {
    mgr: SessionManager,
    vault: Arc<SqlCipherVault>,
    keystore: Arc<InMemoryKeystore>,
    _dir: tempfile::TempDir,
}

fn fresh_confirmed() -> Wired {
    let (dir, path) = temp_db_path();
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
    .expect("confirm retention");
    Wired {
        mgr,
        vault,
        keystore,
        _dir: dir,
    }
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

fn decide_x1_redact_x2_keep(
    mgr: &mut SessionManager,
    doc_id: &str,
) -> String {
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.to_string(),
        })
        .expect("open_approval");
    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| {
            let text = f.span.text.as_deref().expect("C-API-2: approval view has span text");
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
    assert_eq!(decisions.len(), 2, "stub must locate both canaries");
    let out = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: view.approval_session_id.clone(),
            decisions,
        })
        .expect("set_field_decisions");
    assert_eq!(out.lifecycle, pg_core::session::ApprovalLifecycle::Decided);
    view.approval_session_id
}

fn master_of(wired: &Wired) -> pg_core::keys::VaultMasterKey {
    let item = wired
        .keystore
        .load()
        .expect("keystore load")
        .expect("keystore item");
    unwrap_master_key(PASSPHRASE, &item).expect("unwrap master")
}

fn concatenated_redacted_text(wired: &Wired, doc_id: &str) -> String {
    let approved = wired
        .vault
        .load_approved(&master_of(wired), doc_id)
        .expect("load_approved")
        .expect("canonical ApprovedVersion");
    approved
        .redacted_content
        .pages
        .iter()
        .flat_map(|p| p.spans.iter())
        .map(|s| s.text.as_str())
        .collect()
}

fn build_text_pdf(text: &str) -> Vec<u8> {
    let mut doc = LoDocument::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("lopdf fixture");
    bytes
}

// ---------------------------------------------------------------------------
// api.md §2 gating
// ---------------------------------------------------------------------------

#[test]
fn submit_approval_unlocked_only() {
    assert!(!command_allowed("submit_approval", SessionState::FirstRun));
    assert!(!command_allowed("submit_approval", SessionState::Locked));
    assert!(command_allowed("submit_approval", SessionState::Unlocked));
    assert!(!command_allowed(
        "submit_approval",
        SessionState::DegradedIntegrity
    ));
}

#[test]
fn submit_unknown_session_is_not_found() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn submit_while_awaiting_is_approval_bad_state() {
    let mut wired = fresh_confirmed();
    let doc_id = import_letter(&mut wired.mgr, None);
    let view = wired
        .mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open");
    let err = wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: view.approval_session_id,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ApprovalBadState);
}

// ---------------------------------------------------------------------------
// AC-1 / FR-3.2
// ---------------------------------------------------------------------------

#[test]
fn ac1_submit_stores_canonical_approved_and_already_approved() {
    let mut wired = fresh_confirmed();
    let doc_id = import_letter(&mut wired.mgr, None);
    let session_id = decide_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let out = wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: session_id.clone(),
        })
        .expect("submit_approval");

    assert_eq!(out.lifecycle, pg_core::session::ApprovalLifecycle::Committed);
    assert!(out.summary.has_approved_version);
    assert!(!out.summary.has_retained_original);
    let dump = serde_json::to_string(&out.summary).expect("summary json");
    assert!(
        !dump.contains("PG-CANARY-"),
        "AC-1: original/field bytes never appear on command output, got {dump}"
    );

    let redacted = concatenated_redacted_text(&wired, &doc_id);
    assert_eq!(redacted, REDACTED_TEXT);
    assert!(
        !redacted.contains("PG-CANARY-X1"),
        "redacted canary must be omitted, got {redacted:?}"
    );
    assert!(
        redacted.contains("PG-CANARY-X2"),
        "kept canary must remain, got {redacted:?}"
    );

    let err = wired
        .mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AlreadyApproved);

    let view_err = wired
        .mgr
        .get_approval_view(GetApprovalViewIn {
            approval_session_id: session_id,
        })
        .unwrap_err();
    assert_eq!(
        view_err.code,
        ErrorCode::NotFound,
        "C-DES-1: core must not serve approval view after submit"
    );
}

#[test]
fn ac1_unlock_after_lock_still_returns_approved_metadata() {
    let mut wired = fresh_confirmed();
    let doc_id = import_letter(&mut wired.mgr, None);
    let session_id = decide_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: session_id,
        })
        .expect("submit");
    wired.mgr.lock().expect("lock");
    wired
        .mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    let got = wired
        .mgr
        .get_document(GetDocumentIn { doc_id })
        .expect("get_document after lock/unlock");
    assert!(got.summary.has_approved_version);
    assert_eq!(got.summary.source_filename, "letter.txt");
    let dump = serde_json::to_string(&got.summary).expect("summary json");
    assert!(
        !dump.contains("PG-CANARY-"),
        "AC-1: original bytes never on output after re-unlock, got {dump}"
    );
}

#[test]
fn ac1_pdf_import_approve_store() {
    let mut wired = fresh_confirmed();
    let bytes = build_text_pdf("Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.");
    let doc_id = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "letter.pdf".to_string(),
            bytes,
            retention_override: None,
        })
        .expect("import pdf")
        .summary
        .doc_id;
    let session_id = decide_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let out = wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: session_id,
        })
        .expect("submit pdf");
    assert!(out.summary.has_approved_version);
    assert_eq!(out.summary.source_format, SourceFormat::Pdf);
    let redacted = concatenated_redacted_text(&wired, &doc_id);
    assert!(
        !redacted.contains("PG-CANARY-X1"),
        "PDF redacted canary must be omitted, got {redacted:?}"
    );
    assert!(
        redacted.contains("PG-CANARY-X2"),
        "PDF kept canary must remain, got {redacted:?}"
    );
    let err = wired
        .mgr
        .open_approval(OpenApprovalIn { doc_id })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AlreadyApproved);
}

// ---------------------------------------------------------------------------
// Retention hand-off (design §2.1 / data-model §8)
// ---------------------------------------------------------------------------

#[test]
fn discard_original_is_not_decryptable_after_submit() {
    let mut wired = fresh_confirmed();
    let doc_id = import_letter(&mut wired.mgr, None);
    let session_id = decide_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: session_id,
        })
        .expect("submit");
    assert!(
        !wired
            .vault
            .has_retained_original(&doc_id)
            .expect("has_retained_original")
    );
    let original = wired
        .vault
        .load_original(&master_of(&wired), &doc_id)
        .expect("load_original");
    assert!(
        original.is_none(),
        "discard path must not leave a decryptable original"
    );
}

#[test]
fn retain_original_still_decrypts_after_submit() {
    let mut wired = fresh_confirmed();
    let doc_id = import_letter(&mut wired.mgr, Some(EffectiveRetention::Retain));
    let session_id = decide_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    let out = wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: session_id,
        })
        .expect("submit");
    assert!(out.summary.has_retained_original);
    assert!(
        wired
            .vault
            .has_retained_original(&doc_id)
            .expect("has_retained_original")
    );
    let original = wired
        .vault
        .load_original(&master_of(&wired), &doc_id)
        .expect("load_original")
        .expect("retained original present");
    let bytes = original.raw_bytes().expect("decode");
    assert_eq!(bytes, BODY);
}

// ---------------------------------------------------------------------------
// Audit `approve` (api.md §6 / FR-3.4)
// ---------------------------------------------------------------------------

#[test]
fn audit_approve_records_decisions_without_span_text() {
    let mut wired = fresh_confirmed();
    let doc_id = import_letter(&mut wired.mgr, None);
    let session_id = decide_x1_redact_x2_keep(&mut wired.mgr, &doc_id);
    wired
        .mgr
        .submit_approval(SubmitApprovalIn {
            approval_session_id: session_id,
        })
        .expect("submit");
    let rows = wired.vault.replay().expect("replay");
    let approve = rows
        .iter()
        .find(|r| r.event_type == EventType::Approve)
        .expect("approve event");
    assert_eq!(approve.doc_id.as_deref(), Some(doc_id.as_str()));
    assert!(
        !approve.payload_jcs.contains("PG-CANARY-"),
        "approve payload must not carry span text, got {}",
        approve.payload_jcs
    );
    let payload: serde_json::Value =
        serde_json::from_str(&approve.payload_jcs).expect("approve json");
    let decisions = payload["decisions"].as_array().expect("decisions array");
    assert_eq!(decisions.len(), 2);
    let kinds: Vec<&str> = decisions
        .iter()
        .map(|d| d["decision"].as_str().expect("decision"))
        .collect();
    assert!(kinds.contains(&"redact"));
    assert!(kinds.contains(&"keep_visible"));
    for d in decisions {
        assert!(d["field_id"].as_str().is_some());
        assert_eq!(d["label"].as_str(), Some("stub_canary"));
        assert!(d.get("span").is_none());
        assert!(d.get("text").is_none());
    }
}

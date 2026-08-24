//! W24 — `preview_share` / `commit_share` (person-export).
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.6 (token, 10 min / lock / replace; byte-identical commit)
//! - `docs/specs/api.md` §7 (filename + PDF info dictionary)
//! - `docs/dev-plan.md` W24 ("Tests first: `not_approved`; `preview_expired`;
//!   byte-identical commit; filename algorithm; metadata omits original path and
//!   redacted text")
//!
//! Seam: [`SessionManager::preview_share`] / [`SessionManager::commit_share`].
//! Explicitly **not**: Cloud AI (W27); ephemeral overrides (W26); save dialog (W34).

use std::collections::HashMap;
use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::{AuditStore, EventType};
use pg_core::catalog::DocumentStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::session::{
    command_allowed, CommitShareIn, CreateAccountIn, FieldDecisionDto, FieldDecisionKind,
    ImportDocumentIn, OpenApprovalIn, PreviewShareIn, SessionManager, SessionState,
    SetFieldDecisionsIn, SetRetentionDefaultIn, ShareKind, ShareRequestDto, SubmitApprovalIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const BODY: &[u8] = b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.";

struct Wired {
    mgr: SessionManager,
    vault: Arc<SqlCipherVault>,
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
    let vault = Arc::new(SqlCipherVault::new(path));
    let keystore = Arc::new(pg_core::keystore::InMemoryKeystore::new());
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
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
        _dir: dir,
    }
}

fn import_and_approve(mgr: &mut SessionManager, filename: &str) -> String {
    let doc_id = mgr
        .import_document(ImportDocumentIn {
            filename: filename.to_string(),
            bytes: BODY.to_vec(),
            retention_override: None,
        })
        .expect("import")
        .summary
        .doc_id;
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
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
    doc_id
}

fn export_request(doc_ids: Vec<String>) -> PreviewShareIn {
    PreviewShareIn {
        request: ShareRequestDto {
            kind: ShareKind::ExportToPerson,
            doc_ids,
            per_doc_overrides: HashMap::new(),
            applied_variant_ids: HashMap::new(),
            recipient_note: Some("caseworker".to_string()),
            ai_instruction: None,
        },
    }
}

#[test]
fn share_commands_unlocked_only() {
    for command in ["preview_share", "commit_share"] {
        assert!(!command_allowed(command, SessionState::FirstRun));
        assert!(!command_allowed(command, SessionState::Locked));
        assert!(command_allowed(command, SessionState::Unlocked));
        assert!(!command_allowed(command, SessionState::DegradedIntegrity));
    }
}

#[test]
fn preview_unapproved_is_not_approved() {
    let mut wired = fresh_confirmed();
    let doc_id = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: BODY.to_vec(),
            retention_override: None,
        })
        .expect("import")
        .summary
        .doc_id;
    let err = wired
        .mgr
        .preview_share(export_request(vec![doc_id]))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotApproved);
}

#[test]
fn preview_unknown_doc_is_not_found() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .preview_share(export_request(vec![
            "00000000-0000-4000-8000-000000000099".to_string(),
        ]))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn preview_empty_doc_ids_is_invalid_input() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .preview_share(export_request(Vec::new()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn share_to_ai_is_cloud_ai_not_configured() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let err = wired
        .mgr
        .preview_share(PreviewShareIn {
            request: ShareRequestDto {
                kind: ShareKind::ShareToAi,
                doc_ids: vec![doc_id],
                per_doc_overrides: HashMap::new(),
                applied_variant_ids: HashMap::new(),
                recipient_note: None,
                ai_instruction: Some("summarise".to_string()),
            },
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNotConfigured);
}

#[test]
fn commit_bytes_match_preview_and_audit_share() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id.clone()]))
        .expect("preview");
    let pdf = preview.pdf_bytes.expect("export pdf");
    assert!(
        !pdf.windows(b"PG-CANARY-X1".len())
            .any(|w| w == b"PG-CANARY-X1"),
        "redacted canary must not appear in preview bytes"
    );
    let extracted = pdf_extract::extract_text_from_mem(&pdf).expect("extract");
    assert!(extracted.contains("PG-CANARY-X2"), "{extracted:?}");
    assert!(!extracted.contains("PG-CANARY-X1"), "{extracted:?}");
    let name = preview.suggested_filename.expect("filename");
    assert!(name.starts_with("letter-redacted-"));
    assert!(name.ends_with(".pdf"));
    assert!(!name.contains("Alex"), "no account display name in filename");
    let as_str = String::from_utf8_lossy(&pdf);
    assert!(as_str.contains("letter-redacted-"));
    assert!(!as_str.contains("/Author"));
    assert!(!as_str.contains("Alex"));

    let commit = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token.clone(),
        })
        .expect("commit");
    assert_eq!(commit.pdf_bytes.as_ref(), Some(&pdf));
    assert_eq!(commit.suggested_filename.as_deref(), Some(name.as_str()));
    assert_eq!(commit.kind, ShareKind::ExportToPerson);
    assert!(commit.audit_event_id >= 1);
    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains(&doc_id));
    assert!(share.payload_jcs.contains("export_to_person"));
    assert!(!share.payload_jcs.contains("PG-CANARY"));
}

#[test]
fn replaced_preview_expires_the_previous_token() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let first = wired
        .mgr
        .preview_share(export_request(vec![doc_id.clone()]))
        .expect("first");
    let _second = wired
        .mgr
        .preview_share(export_request(vec![doc_id]))
        .expect("second");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: first.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PreviewExpired);
}

#[test]
fn lock_expires_the_preview_token() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id]))
        .expect("preview");
    wired.mgr.lock().expect("lock");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInSession);
}

#[test]
fn ttl_expiry_is_preview_expired() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id]))
        .expect("preview");
    wired.mgr.test_only_expire_preview();
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PreviewExpired);
}

#[test]
fn unknown_preview_token_is_preview_expired() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PreviewExpired);
}

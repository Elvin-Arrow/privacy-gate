//! W26 — Ephemeral overrides + variants on share (AC-2).
//!
//! Spec sources:
//! - `docs/specs/srs.md` FR-5.4 (ephemeral, never mutates canonical), FR-5.5 (variants),
//!   FR-6.2 (`overrides_in_effect` warning flag).
//! - `docs/specs/design.md` §2.5 / §3.4 / §3.7 / C-DES-5 (overrides ephemeral; variants are
//!   the only persistent override form).
//! - `docs/specs/api.md` §4 `ShareRequestDto.per_doc_overrides` / `applied_variant_ids`;
//!   §5.6 `SharePreview.overrides_in_effect`.
//! - `docs/dev-plan.md` W26 ("Tests first: AC-2; vault approved unchanged after share with
//!   overrides." "Do not: persist overrides as a second canonical version.")
//!
//! Seam: [`pg_core::overlap::redact_with_overrides`] (pure re-render) and
//! [`SessionManager::preview_share`] (wiring `ShareRequestDto` overrides/variants into it).

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::AuditStore;
use pg_core::catalog::{DetectedField, DocumentStore, FieldDecision};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::importer::TextSpan;
use pg_core::overlap::redact_with_overrides;
use pg_core::session::{
    command_allowed, CreateAccountIn, FieldDecisionDto, FieldDecisionKind, GetVariantIn,
    ImportDocumentIn, OpenApprovalIn, PreviewShareIn, SaveVariantIn, SessionManager,
    SessionState, SetFieldDecisionsIn, SetRetentionDefaultIn, ShareKind, ShareRequestDto,
    SubmitApprovalIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

// ---------------------------------------------------------------------------
// Pure `redact_with_overrides` — no session, no vault.
// ---------------------------------------------------------------------------

fn field(id: &str, start: u64, text: &str, parent: Option<&str>) -> DetectedField {
    DetectedField {
        id: id.to_string(),
        label: id.to_string(),
        classification: "test".to_string(),
        span: TextSpan {
            byte_offset: start,
            byte_length: text.len() as u64,
            text: text.to_string(),
            page_index: 0,
        },
        parent_field_id: parent.map(str::to_string),
    }
}

fn decision(
    f: &DetectedField,
    d: FieldDecisionKind,
) -> FieldDecision {
    FieldDecision {
        field: f.clone(),
        decision: d,
    }
}

fn page_text(rd: &pg_core::catalog::RedactedDocument) -> String {
    rd.pages
        .iter()
        .flat_map(|p| &p.spans)
        .map(|s| s.text.as_str())
        .collect()
}

#[test]
fn override_reveals_a_canonically_redacted_field() {
    // "Dear PG-CANARY-A, cc PG-CANARY-B."
    let a = field("a", 5, "PG-CANARY-A", None);
    let b = field("b", 21, "PG-CANARY-B", None);
    let canonical = vec![
        decision(&a, FieldDecisionKind::Redact),
        decision(&b, FieldDecisionKind::KeepVisible),
    ];
    let redacted_content = pg_core::overlap::redact_pages(
        pg_core::importer::SourceFormat::Text,
        &[pg_core::importer::Page {
            spans: vec![TextSpan {
                byte_offset: 0,
                byte_length: 33,
                text: "Dear PG-CANARY-A, cc PG-CANARY-B.".to_string(),
                page_index: 0,
            }],
        }],
        &[a.clone(), b.clone()],
        &canonical
            .iter()
            .map(|d| (d.field.id.clone(), d.decision))
            .collect(),
    );
    assert!(!page_text(&redacted_content).contains("PG-CANARY-A"));

    // Ephemeral override: reveal `a` for this share only.
    let mut effective: HashMap<String, FieldDecisionKind> = canonical
        .iter()
        .map(|d| (d.field.id.clone(), d.decision))
        .collect();
    effective.insert("a".to_string(), FieldDecisionKind::KeepVisible);

    let overridden = redact_with_overrides(&canonical, &redacted_content, &effective);
    let text = page_text(&overridden);
    assert!(text.contains("PG-CANARY-A"), "{text:?}");
    assert!(text.contains("PG-CANARY-B"), "{text:?}");
}

#[test]
fn override_hides_a_canonically_kept_field() {
    let a = field("a", 5, "PG-CANARY-A", None);
    let b = field("b", 21, "PG-CANARY-B", None);
    let canonical = vec![
        decision(&a, FieldDecisionKind::KeepVisible),
        decision(&b, FieldDecisionKind::KeepVisible),
    ];
    let redacted_content = pg_core::overlap::redact_pages(
        pg_core::importer::SourceFormat::Text,
        &[pg_core::importer::Page {
            spans: vec![TextSpan {
                byte_offset: 0,
                byte_length: 33,
                text: "Dear PG-CANARY-A, cc PG-CANARY-B.".to_string(),
                page_index: 0,
            }],
        }],
        &[a.clone(), b.clone()],
        &canonical
            .iter()
            .map(|d| (d.field.id.clone(), d.decision))
            .collect(),
    );
    assert!(page_text(&redacted_content).contains("PG-CANARY-A"));

    let mut effective: HashMap<String, FieldDecisionKind> = canonical
        .iter()
        .map(|d| (d.field.id.clone(), d.decision))
        .collect();
    effective.insert("a".to_string(), FieldDecisionKind::Redact);

    let overridden = redact_with_overrides(&canonical, &redacted_content, &effective);
    let text = page_text(&overridden);
    assert!(!text.contains("PG-CANARY-A"), "{text:?}");
    assert!(!text.as_bytes().windows(11).any(|w| w == b"PG-CANARY-A"));
    assert!(text.contains("PG-CANARY-B"), "{text:?}");
}

#[test]
fn override_on_a_nested_field_does_not_disturb_the_outer_decision() {
    // outer [0,10) Redact, inner [3,6) canonically Keep -> inner visible, outer cut around it.
    let outer = field("outer", 0, "0123456789", None);
    let inner = field("inner", 3, "345", Some("outer"));
    let canonical = vec![
        decision(&outer, FieldDecisionKind::Redact),
        decision(&inner, FieldDecisionKind::KeepVisible),
    ];
    let redacted_content = pg_core::overlap::redact_pages(
        pg_core::importer::SourceFormat::Text,
        &[pg_core::importer::Page {
            spans: vec![TextSpan {
                byte_offset: 0,
                byte_length: 10,
                text: "0123456789".to_string(),
                page_index: 0,
            }],
        }],
        &[outer.clone(), inner.clone()],
        &canonical
            .iter()
            .map(|d| (d.field.id.clone(), d.decision))
            .collect(),
    );
    assert_eq!(page_text(&redacted_content), "345");

    // Override the inner field back to Redact ephemerally; outer stays Redact too, so
    // nothing should be visible at all.
    let mut effective: HashMap<String, FieldDecisionKind> = canonical
        .iter()
        .map(|d| (d.field.id.clone(), d.decision))
        .collect();
    effective.insert("inner".to_string(), FieldDecisionKind::Redact);
    let overridden = redact_with_overrides(&canonical, &redacted_content, &effective);
    assert_eq!(page_text(&overridden), "");
}

#[test]
fn identity_override_reproduces_the_canonical_content_exactly() {
    let a = field("a", 5, "PG-CANARY-A", None);
    let b = field("b", 21, "PG-CANARY-B", None);
    let canonical = vec![
        decision(&a, FieldDecisionKind::Redact),
        decision(&b, FieldDecisionKind::KeepVisible),
    ];
    let canonical_map: HashMap<String, FieldDecisionKind> = canonical
        .iter()
        .map(|d| (d.field.id.clone(), d.decision))
        .collect();
    let redacted_content = pg_core::overlap::redact_pages(
        pg_core::importer::SourceFormat::Text,
        &[pg_core::importer::Page {
            spans: vec![TextSpan {
                byte_offset: 0,
                byte_length: 33,
                text: "Dear PG-CANARY-A, cc PG-CANARY-B.".to_string(),
                page_index: 0,
            }],
        }],
        &[a, b],
        &canonical_map,
    );
    let same = redact_with_overrides(&canonical, &redacted_content, &canonical_map);
    assert_eq!(same, redacted_content);
}

// ---------------------------------------------------------------------------
// Session-level wiring: `preview_share` / `commit_share` with overrides + variants.
// ---------------------------------------------------------------------------

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
    let audit: Arc<dyn pg_core::audit::AuditStore> = vault.clone();
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

/// Imports + approves with X1 redacted, X2 kept (the canonical decision). Returns
/// `(doc_id, field_id_of_x1, field_id_of_x2)`.
fn import_and_approve(mgr: &mut SessionManager, filename: &str) -> (String, String, String) {
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
    let mut x1 = String::new();
    let mut x2 = String::new();
    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| {
            let text = f.span.text.as_deref().unwrap_or("");
            let decision = if text.contains("PG-CANARY-X1") {
                x1 = f.id.clone();
                FieldDecisionKind::Redact
            } else {
                x2 = f.id.clone();
                FieldDecisionKind::KeepVisible
            };
            FieldDecisionDto {
                field_id: f.id.clone(),
                decision,
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
    (doc_id, x1, x2)
}

fn export_request(
    doc_ids: Vec<String>,
    per_doc_overrides: HashMap<String, Vec<FieldDecisionDto>>,
    applied_variant_ids: HashMap<String, String>,
) -> PreviewShareIn {
    PreviewShareIn {
        request: ShareRequestDto {
            kind: ShareKind::ExportToPerson,
            doc_ids,
            per_doc_overrides,
            applied_variant_ids,
            recipient_note: Some("caseworker".to_string()),
            ai_instruction: None,
        },
    }
}

#[test]
fn commands_accept_overrides_and_variants_fields_unlocked_only() {
    for command in ["preview_share", "commit_share"] {
        assert!(!command_allowed(command, SessionState::FirstRun));
        assert!(!command_allowed(command, SessionState::Locked));
        assert!(command_allowed(command, SessionState::Unlocked));
        assert!(!command_allowed(command, SessionState::DegradedIntegrity));
    }
}

#[test]
fn ac2_ephemeral_override_reveals_x1_and_preview_flags_overrides_in_effect() {
    let mut wired = fresh_confirmed();
    let (doc_id, x1, _x2) = import_and_approve(&mut wired.mgr, "letter.txt");

    let mut overrides = HashMap::new();
    overrides.insert(
        doc_id.clone(),
        vec![FieldDecisionDto {
            field_id: x1.clone(),
            decision: FieldDecisionKind::KeepVisible,
        }],
    );
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id.clone()], overrides, HashMap::new()))
        .expect("preview");
    assert!(preview.overrides_in_effect);
    let pdf = preview.pdf_bytes.expect("export pdf");
    let extracted = pdf_extract::extract_text_from_mem(&pdf).expect("extract");
    assert!(extracted.contains("PG-CANARY-X1"), "{extracted:?}");
    assert!(extracted.contains("PG-CANARY-X2"), "{extracted:?}");

    let manifest = &preview.manifest[0];
    assert!(manifest.visible_field_ids.contains(&x1));

    let commit = wired
        .mgr
        .commit_share(pg_core::session::CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit");
    assert_eq!(commit.pdf_bytes.as_deref(), Some(pdf.as_slice()));

    // FR-5.6: the share is still recorded, and the audit payload never carries redacted
    // field text (canonical or overridden).
    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == pg_core::audit::EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains(&doc_id));
    assert!(!share.payload_jcs.contains("PG-CANARY"));
}

#[test]
fn preview_without_overrides_after_an_overridden_preview_is_back_to_canonical() {
    // "vault approved unchanged after share with overrides" (dev-plan W26).
    let mut wired = fresh_confirmed();
    let (doc_id, x1, _x2) = import_and_approve(&mut wired.mgr, "letter.txt");

    let mut overrides = HashMap::new();
    overrides.insert(
        doc_id.clone(),
        vec![FieldDecisionDto {
            field_id: x1,
            decision: FieldDecisionKind::KeepVisible,
        }],
    );
    let overridden = wired
        .mgr
        .preview_share(export_request(
            vec![doc_id.clone()],
            overrides,
            HashMap::new(),
        ))
        .expect("overridden preview");
    let overridden_text =
        pdf_extract::extract_text_from_mem(&overridden.pdf_bytes.unwrap()).expect("extract");
    assert!(overridden_text.contains("PG-CANARY-X1"));

    let canonical = wired
        .mgr
        .preview_share(export_request(vec![doc_id], HashMap::new(), HashMap::new()))
        .expect("canonical preview");
    assert!(!canonical.overrides_in_effect);
    let canonical_text =
        pdf_extract::extract_text_from_mem(&canonical.pdf_bytes.unwrap()).expect("extract");
    assert!(
        !canonical_text.contains("PG-CANARY-X1"),
        "canonical ApprovedVersion must be unaffected by the earlier ephemeral override: {canonical_text:?}"
    );
    assert!(canonical_text.contains("PG-CANARY-X2"));
}

#[test]
fn applied_variant_reveals_x1_at_preview_without_mutating_the_variant_or_approved() {
    let mut wired = fresh_confirmed();
    let (doc_id, x1, _x2) = import_and_approve(&mut wired.mgr, "letter.txt");

    let saved = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: doc_id.clone(),
            name: "reveal-x1".to_string(),
            overrides: vec![FieldDecisionDto {
                field_id: x1.clone(),
                decision: FieldDecisionKind::KeepVisible,
            }],
        })
        .expect("save_variant");

    let mut applied = HashMap::new();
    applied.insert(doc_id.clone(), saved.variant_id.clone());
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id.clone()], HashMap::new(), applied))
        .expect("preview with variant");
    assert!(preview.overrides_in_effect);
    let text = pdf_extract::extract_text_from_mem(&preview.pdf_bytes.unwrap()).expect("extract");
    assert!(text.contains("PG-CANARY-X1"));
    assert!(text.contains("PG-CANARY-X2"));

    // The variant itself and the canonical approved version are untouched.
    let reread = wired
        .mgr
        .get_variant(GetVariantIn {
            doc_id,
            variant_id: saved.variant_id,
        })
        .expect("get_variant");
    assert_eq!(reread.overrides.len(), 1);
    assert_eq!(reread.overrides[0].field_id, x1);
    assert_eq!(reread.overrides[0].decision, FieldDecisionKind::KeepVisible);
}

#[test]
fn per_doc_override_layers_on_top_of_an_applied_variant() {
    let mut wired = fresh_confirmed();
    let (doc_id, x1, x2) = import_and_approve(&mut wired.mgr, "letter.txt");

    // Variant reveals X1.
    let saved = wired
        .mgr
        .save_variant(SaveVariantIn {
            doc_id: doc_id.clone(),
            name: "reveal-x1".to_string(),
            overrides: vec![FieldDecisionDto {
                field_id: x1.clone(),
                decision: FieldDecisionKind::KeepVisible,
            }],
        })
        .expect("save_variant");

    // Ad-hoc override on top additionally hides X2.
    let mut applied = HashMap::new();
    applied.insert(doc_id.clone(), saved.variant_id);
    let mut overrides = HashMap::new();
    overrides.insert(
        doc_id.clone(),
        vec![FieldDecisionDto {
            field_id: x2,
            decision: FieldDecisionKind::Redact,
        }],
    );
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id], overrides, applied))
        .expect("preview");
    let text = pdf_extract::extract_text_from_mem(&preview.pdf_bytes.unwrap()).expect("extract");
    assert!(text.contains("PG-CANARY-X1"), "{text:?}");
    assert!(!text.contains("PG-CANARY-X2"), "{text:?}");
    assert!(!text.as_bytes().windows(11).any(|w| w == b"PG-CANARY-X2"));
}

#[test]
fn unknown_field_id_in_per_doc_overrides_is_invalid_input() {
    let mut wired = fresh_confirmed();
    let (doc_id, ..) = import_and_approve(&mut wired.mgr, "letter.txt");
    let mut overrides = HashMap::new();
    overrides.insert(
        doc_id.clone(),
        vec![FieldDecisionDto {
            field_id: "not-a-real-field".to_string(),
            decision: FieldDecisionKind::KeepVisible,
        }],
    );
    let err = wired
        .mgr
        .preview_share(export_request(vec![doc_id], overrides, HashMap::new()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn unknown_variant_id_in_applied_variant_ids_is_not_found() {
    let mut wired = fresh_confirmed();
    let (doc_id, ..) = import_and_approve(&mut wired.mgr, "letter.txt");
    let mut applied = HashMap::new();
    applied.insert(
        doc_id.clone(),
        "00000000-0000-4000-8000-000000000099".to_string(),
    );
    let err = wired
        .mgr
        .preview_share(export_request(vec![doc_id], HashMap::new(), applied))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn oq6_oracle_holds_with_an_override_in_effect() {
    let mut wired = fresh_confirmed();
    let (doc_id, x1, x2) = import_and_approve(&mut wired.mgr, "letter.txt");
    // Override reverses canonical: now X2 is redacted, X1 stays redacted too (no reveal),
    // so the oracle should find neither canary anywhere in egress.
    let mut overrides = HashMap::new();
    overrides.insert(
        doc_id.clone(),
        vec![FieldDecisionDto {
            field_id: x2.clone(),
            decision: FieldDecisionKind::Redact,
        }],
    );
    let preview = wired
        .mgr
        .preview_share(export_request(vec![doc_id], overrides, HashMap::new()))
        .expect("preview");
    let pdf = preview.pdf_bytes.expect("pdf");
    let _ = x1;
    common::oracle::check(&pdf, &["PG-CANARY-X1", "PG-CANARY-X2"], &[])
        .expect("W25 OQ-6 oracle holds when overrides redact further");
}

//! W10 — Catalog and `import_document` (no detector yet).
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.3 (`import_document`, `list_documents`, `get_document`), §4
//!   (`DocumentSummary`)
//! - `docs/specs/srs.md` FR-1.3–1.5
//! - `docs/specs/data-model.md` §6.1 (`DocumentMeta`), §6.2 (`OriginalRecord`)
//! - `docs/dev-plan.md` W10 ("Tests first: basename only; `over_budget` true still
//!   completes; two imports of same bytes → two `doc_id`s; `get_document` has no span
//!   text; newest first.")
//!
//! Detection in this file is whatever W15c selects (factory `"auto"` → hybrid when
//! Ollama is unreachable). Fixtures here have no UK PII, so `detected_field_count`
//! stays `0`. Real detection goldens live in later detector tests; AC stub path uses
//! `with_detector`.
//!
//! Out of W10 scope and deliberately absent here: `retention_policy_unset` /
//! `retention_loosen_forbidden` (W11 — this file's fixtures always confirm a policy
//! first, matching C-TEST-6), real detection (W12), `already_approved` paths, approval UI.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::{DocumentStore, EffectiveRetention};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::importer::SourceFormat;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    CreateAccountIn, GetDocumentIn, ImportDocumentIn, SessionManager, SetRetentionDefaultIn,
};
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

/// A `SessionManager` wired to a real `SqlCipherVault` (sharing one connection as every
/// backend), with the retention default already confirmed as `discard` (C-TEST-6: "Paranoid
/// tests call `set_retention_default` first" — same discipline applies to any import test).
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
        .with_documents(documents);
    mgr.create_account(create_in()).expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm retention default");
    (mgr, dir)
}

fn import_in(filename: &str, bytes: &[u8]) -> ImportDocumentIn {
    ImportDocumentIn {
        filename: filename.to_string(),
        bytes: bytes.to_vec(),
        retention_override: None,
    }
}

// ---------------------------------------------------------------------------
// dev-plan W10: "basename only"
// ---------------------------------------------------------------------------

#[test]
fn import_document_accepts_a_basename_filename() {
    let (mut mgr, _dir) = fresh_confirmed();
    let out = mgr
        .import_document(import_in("letter.txt", b"hello world"))
        .expect("import_document with a basename filename");
    assert_eq!(out.summary.source_filename, "letter.txt");
}

#[test]
fn import_document_rejects_unix_path_separators() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr
        .import_document(import_in("../etc/passwd", b"hello world"))
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::InvalidInput);
}

#[test]
fn import_document_rejects_windows_path_separators() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr
        .import_document(import_in(r"C:\Users\alex\letter.txt", b"hello world"))
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::InvalidInput);
}

#[test]
fn import_document_rejects_empty_filename() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr.import_document(import_in("", b"hello world")).unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------------------
// dev-plan W10: "over_budget true still completes"
// ---------------------------------------------------------------------------

#[test]
fn over_budget_document_still_completes() {
    let (mut mgr, _dir) = fresh_confirmed();
    // Just over the 25 MB design §7 interactive budget, still valid UTF-8 text.
    let bytes = vec![b'a'; 25 * 1024 * 1024 + 1];
    let out = mgr
        .import_document(import_in("big.txt", &bytes))
        .expect("import_document must complete even over budget");
    assert!(out.over_budget);
    assert_eq!(out.summary.source_filename, "big.txt");
}

#[test]
fn under_budget_document_is_not_flagged() {
    let (mut mgr, _dir) = fresh_confirmed();
    let out = mgr
        .import_document(import_in("small.txt", b"hello world"))
        .expect("import_document");
    assert!(!out.over_budget);
}

// ---------------------------------------------------------------------------
// dev-plan W10: "two imports of same bytes → two doc_ids"
// ---------------------------------------------------------------------------

#[test]
fn two_imports_of_identical_bytes_get_two_doc_ids() {
    let (mut mgr, _dir) = fresh_confirmed();
    let bytes = b"identical content, imported twice";
    let out_a = mgr.import_document(import_in("a.txt", bytes)).expect("first import");
    let out_b = mgr.import_document(import_in("b.txt", bytes)).expect("second import");
    assert_ne!(out_a.summary.doc_id, out_b.summary.doc_id);

    let listed = mgr.list_documents().expect("list_documents");
    assert_eq!(listed.documents.len(), 2);
}

// ---------------------------------------------------------------------------
// dev-plan W10: "get_document has no span text"
// ---------------------------------------------------------------------------

#[test]
fn get_document_returns_summary_with_no_span_or_field_text() {
    let (mut mgr, _dir) = fresh_confirmed();
    let imported = mgr
        .import_document(import_in("letter.txt", b"the quick brown fox"))
        .expect("import_document");

    let got = mgr
        .get_document(GetDocumentIn {
            doc_id: imported.summary.doc_id.clone(),
        })
        .expect("get_document");

    assert_eq!(got.summary, imported.summary);
    // Structural proof, not just an assertion on values: DocumentSummary (api.md §4) has
    // no field that could hold span or document text at all — `source_filename` is the
    // only string field carrying user content, and it is the filename, not the body.
    let serialized = serde_json::to_value(&got.summary).expect("serialize DocumentSummary");
    let obj = serialized.as_object().expect("object");
    let expected_keys = [
        "doc_id",
        "source_filename",
        "source_format",
        "imported_at",
        "retention",
        "has_approved_version",
        "has_retained_original",
        "detected_field_count",
    ];
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = expected_keys.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected, "DocumentSummary must carry exactly api.md §4's fields");
}

#[test]
fn get_document_not_found_for_unknown_doc_id() {
    let (mgr, _dir) = fresh_confirmed();
    let err = mgr
        .get_document(GetDocumentIn {
            doc_id: "00000000-0000-4000-8000-000000000099".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::NotFound);
}

// ---------------------------------------------------------------------------
// dev-plan W10: "newest first"
// ---------------------------------------------------------------------------

#[test]
fn list_documents_is_newest_import_first() {
    let (mut mgr, _dir) = fresh_confirmed();
    let first = mgr.import_document(import_in("first.txt", b"one")).expect("import 1");
    let second = mgr.import_document(import_in("second.txt", b"two")).expect("import 2");
    let third = mgr.import_document(import_in("third.txt", b"three")).expect("import 3");

    let listed = mgr.list_documents().expect("list_documents");
    let ids: Vec<&str> = listed.documents.iter().map(|d| d.doc_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            third.summary.doc_id.as_str(),
            second.summary.doc_id.as_str(),
            first.summary.doc_id.as_str(),
        ]
    );
}

/// The ordering must survive a lock/unlock cycle — it isn't an artifact of in-memory
/// insertion order that happens to look right before anything round-trips through SQL.
/// Retain is required: lock drops unapproved discard rows (data-model §8 / W19).
#[test]
fn newest_first_ordering_survives_lock_and_unlock() {
    let (mut mgr, _dir) = fresh_confirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Retain,
    })
    .expect("set retain so catalog rows survive lock");
    mgr.import_document(import_in("first.txt", b"one")).expect("import 1");
    let second = mgr.import_document(import_in("second.txt", b"two")).expect("import 2");
    mgr.lock().expect("lock");
    mgr.unlock(pg_core::session::UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");

    let listed = mgr.list_documents().expect("list_documents");
    assert_eq!(listed.documents[0].doc_id, second.summary.doc_id);
}

// ---------------------------------------------------------------------------
// data-model §6.1: never_retain → document retention: discard (not stored as never_retain)
// ---------------------------------------------------------------------------

#[test]
fn never_retain_default_writes_discard_on_the_document() {
    let (mut mgr, _dir) = fresh_confirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::NeverRetain,
    })
    .expect("set global default to never_retain");

    let out = mgr
        .import_document(import_in("paranoid.txt", b"sensitive-ish content"))
        .expect("import_document under never_retain");
    assert_eq!(
        out.summary.retention,
        EffectiveRetention::Discard,
        "never_retain must never be stored on the document itself (data-model §6.1)"
    );
    assert!(!out.summary.has_retained_original);
}

#[test]
fn retain_default_produces_a_retained_original() {
    let (mut mgr, _dir) = fresh_confirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Retain,
    })
    .expect("set global default to retain");

    let out = mgr
        .import_document(import_in("keepme.txt", b"content worth keeping"))
        .expect("import_document under retain");
    assert_eq!(out.summary.retention, EffectiveRetention::Retain);
    assert!(out.summary.has_retained_original);
}

#[test]
fn per_import_override_takes_precedence_over_the_default() {
    let (mut mgr, _dir) = fresh_confirmed();
    // Default is discard (fresh_confirmed's setup); override this one import to retain.
    let out = mgr
        .import_document(ImportDocumentIn {
            filename: "override.txt".to_string(),
            bytes: b"overridden to retain".to_vec(),
            retention_override: Some(EffectiveRetention::Retain),
        })
        .expect("import_document with override");
    assert_eq!(out.summary.retention, EffectiveRetention::Retain);
    assert!(out.summary.has_retained_original);
}

// ---------------------------------------------------------------------------
// PDF import through the catalog command (format switch, dev-plan W10 "Integrate")
// ---------------------------------------------------------------------------

#[test]
fn import_document_detects_pdf_by_content_not_filename() {
    let (mut mgr, _dir) = fresh_confirmed();
    // A minimal but real PDF magic-byte prefix is enough to route to the PDF path; a full
    // parse failure past that point still correctly surfaces as unsupported_document
    // (proving the switch dispatched to import_pdf, not import_text, which would have
    // accepted these bytes as arbitrary "text").
    let err = mgr
        .import_document(import_in("document.txt", b"%PDF-1.5\ngarbage, not a real pdf"))
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::UnsupportedDocument);
}

#[test]
fn empty_bytes_are_unsupported_document() {
    let (mut mgr, _dir) = fresh_confirmed();
    let err = mgr.import_document(import_in("empty.txt", b"")).unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::UnsupportedDocument);
}

// ---------------------------------------------------------------------------
// C-API-6: import/list/get are unavailable while degraded or before unlock.
// ---------------------------------------------------------------------------

#[test]
fn import_document_refused_before_unlock() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault;
    let mut mgr =
        SessionManager::new_full(keystore, accounts, backend, audit, config).with_documents(documents);

    let err = mgr.import_document(import_in("x.txt", b"hi")).unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::NotInSession);
}

#[test]
fn config_and_document_commands_are_refused_while_degraded() {
    for command in ["import_document", "list_documents", "get_document"] {
        assert!(!pg_core::session::command_allowed(
            command,
            pg_core::session::SessionState::DegradedIntegrity
        ));
    }
}

// ---------------------------------------------------------------------------
// The audit head advances on import (architecture §6.2) — proven end to end via lock's
// on-persist behaviour, exercising the record_audit_append wiring this chunk adds.
// ---------------------------------------------------------------------------

#[test]
fn import_appends_an_audit_row_and_the_head_persists_on_lock() {
    let (mut mgr, _dir) = fresh_confirmed();
    mgr.import_document(import_in("one.txt", b"first document"))
        .expect("import_document");
    let report_before_lock = mgr.get_integrity_report().expect("get_integrity_report");
    // set_retention_default in fresh_confirmed's setup does not append audit rows (config
    // changes are not audited events in this codebase); one import now appends two rows
    // (W12: `import`, then `detect` — design §2.2's Detector also emits to the Audit
    // Trail), so the live head is at sequence 2 even though the last *persisted* report
    // still reflects the state as of the last unlock.
    assert_eq!(report_before_lock.tail_sequence, 0, "report reflects the last unlock, not live appends");

    mgr.lock().expect("lock");
    let out = mgr
        .unlock(pg_core::session::UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    assert_eq!(
        out.state,
        pg_core::session::SessionState::Unlocked,
        "the persisted head from lock must make this a clean unlock, not a crash-window \
         fast-forward or (worse) a failure"
    );
    let report_after = mgr.get_integrity_report().expect("get_integrity_report");
    assert_eq!(report_after.kind, "ok");
    assert_eq!(report_after.tail_sequence, 2, "import + detect = two audit rows for one import_document call");
}

/// The imported document's summary survives a lock/unlock cycle intact, and so does the
/// original bytes' round trip through the catalog when retained.
#[test]
fn imported_document_and_retained_original_survive_lock_and_unlock() {
    let (mut mgr, _dir) = fresh_confirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Retain,
    })
    .expect("set retain");
    let imported = mgr
        .import_document(import_in("keep.txt", b"content that must survive"))
        .expect("import_document");

    mgr.lock().expect("lock");
    mgr.unlock(pg_core::session::UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");

    let got = mgr
        .get_document(GetDocumentIn {
            doc_id: imported.summary.doc_id,
        })
        .expect("get_document after reopen");
    assert!(got.summary.has_retained_original);
    assert_eq!(got.summary.source_format, SourceFormat::Text);
}

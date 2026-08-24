//! W12 — Detector host + stub (unblocks AC-1).
//!
//! Spec sources:
//! - `docs/specs/design.md` §2.2 (Detector responsibilities: run over the Importer's IR,
//!   produce classified fields with byte-offset spans, emit a `detect` audit event, expose
//!   an empty first-party plugin hook)
//! - `docs/specs/architecture.md` §10 (detector identity — out of scope for the stub)
//! - `docs/specs/testing.md` §10 ("Detector stub: implements the same host-facing trait as
//!   `pg-hybrid-v1`; returns the sidecar fields. Used by AC-1..AC-4 so model drift cannot
//!   hide a vault bug.")
//! - `docs/specs/srs.md` FR-2.1–2.4
//! - `docs/dev-plan.md` W12 ("Tests first: fixture sidecar fields appear as locatable
//!   spans; no network; empty hook does not crash.")
//!
//! # Two layers, deliberately
//!
//! `StubDetector::detect` is tested directly (no `SessionManager`, no vault) for span
//! locatability — the same layer W8/W9 tested `import_text`/`import_pdf` at, and the right
//! one for "does this component do its one job correctly" without a real `field_id` being
//! predictable (`crate::detector` mints a fresh UUID per field, by design — "known
//! field_ids" in dev-plan's sense means a *known, deterministic set of matches*, not a
//! fixed ID string). `import_document`'s integration with the Detector — did it actually
//! get called, does `detected_field_count` show up in the catalog — is tested separately
//! through `SessionManager`, matching how every other importer-adjacent chunk in this
//! project has split "component" from "command" coverage.
//!
//! "No network": structural, not a runtime probe — `StubDetector` holds no fields, opens
//! no sockets, and calls nothing but string/byte operations (`crate::detector` module
//! docs). There is no HTTP/network dependency anywhere in this module to point a test at.
//!
//! Out of W12 scope and deliberately absent here: the pattern pack `pg-patterns-uk-v1`
//! (W13), ONNX (W15a), the Ollama backend (W15b/W15c) — StubDetector never matches real
//! PII shapes, only the synthetic `PG-CANARY-` marker.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::DocumentStore;
use pg_core::config::ConfigStore;
use pg_core::detector::{Detector, StubDetector, STUB_CANARY_MARKER};
use pg_core::importer::{self, Document, Page, SourceFormat, TextSpan};
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{CreateAccountIn, ImportDocumentIn, SessionManager, SetRetentionDefaultIn};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const DOC_ID: &str = "00000000-0000-4000-8000-000000000010";
const PASSPHRASE: &str = "correct horse battery staple";

// ---------------------------------------------------------------------------
// dev-plan W12: "fixture sidecar fields appear as locatable spans"
// (component layer — StubDetector directly)
// ---------------------------------------------------------------------------

#[test]
fn stub_detector_locates_a_planted_canary_at_its_real_byte_offset() {
    let text = "Dear Sir, reference PG-CANARY-A1B2 is enclosed.";
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");

    let fields = StubDetector.detect(&doc);
    assert_eq!(fields.len(), 1);
    let field = &fields[0];
    assert_eq!(field.span.text, "PG-CANARY-A1B2");
    assert_eq!(field.span.page_index, 0);
    // Locatable: the reported offset must actually point at the marker within the
    // original text, not just report the right substring.
    let expected_offset = text.find("PG-CANARY-A1B2").expect("marker present in fixture") as u64;
    assert_eq!(field.span.byte_offset, expected_offset);
    assert_eq!(field.span.byte_length, "PG-CANARY-A1B2".len() as u64);
    assert_eq!(field.label, "stub_canary");
    assert_eq!(field.classification, "synthetic_canary");
    assert!(field.parent_field_id.is_none());
}

#[test]
fn stub_detector_finds_every_marker_in_a_multi_marker_document() {
    let text = "PG-CANARY-ONE middle text PG-CANARY-TWO trailing PG-CANARY-THREE";
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");
    let fields = StubDetector.detect(&doc);
    let mut found: Vec<&str> = fields.iter().map(|f| f.span.text.as_str()).collect();
    found.sort_unstable();
    assert_eq!(found, vec!["PG-CANARY-ONE", "PG-CANARY-THREE", "PG-CANARY-TWO"]);

    // Every reported offset must be locatable, not just the first.
    for field in &fields {
        let start = field.span.byte_offset as usize;
        let end = start + field.span.byte_length as usize;
        assert_eq!(&text.as_bytes()[start..end], field.span.text.as_bytes());
    }
}

#[test]
fn stub_detector_locates_markers_across_pdf_pages_with_correct_page_index() {
    let doc = Document {
        id: DOC_ID.to_string(),
        source_format: SourceFormat::Pdf,
        pages: vec![
            Page {
                spans: vec![TextSpan {
                    byte_offset: 0,
                    byte_length: "no marker here".len() as u64,
                    text: "no marker here".to_string(),
                    page_index: 0,
                }],
            },
            Page {
                spans: vec![TextSpan {
                    byte_offset: 0,
                    byte_length: "second page has PG-CANARY-PAGE2".len() as u64,
                    text: "second page has PG-CANARY-PAGE2".to_string(),
                    page_index: 1,
                }],
            },
        ],
        raw_bytes: Vec::new(),
    };

    let fields = StubDetector.detect(&doc);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].span.page_index, 1);
    assert_eq!(fields[0].span.text, "PG-CANARY-PAGE2");
}

// ---------------------------------------------------------------------------
// dev-plan W12: "empty hook does not crash"
// ---------------------------------------------------------------------------

#[test]
fn stub_detector_on_a_document_with_no_markers_returns_empty_without_panicking() {
    let text = "Ordinary prose with no synthetic markers at all.";
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");
    let fields = StubDetector.detect(&doc);
    assert!(fields.is_empty());
}

#[test]
fn stub_detector_on_an_empty_document_does_not_panic() {
    let doc = Document {
        id: DOC_ID.to_string(),
        source_format: SourceFormat::Text,
        pages: Vec::new(),
        raw_bytes: Vec::new(),
    };
    let fields = StubDetector.detect(&doc);
    assert!(fields.is_empty());
}

/// A marker with nothing else around it — no whitespace to delimit a token boundary on
/// one side — must not panic on an out-of-range slice.
#[test]
fn stub_detector_handles_a_marker_at_the_very_start_or_end_of_text() {
    let doc = importer::import_text(b"PG-CANARY-START", DOC_ID).expect("import_text");
    let fields = StubDetector.detect(&doc);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].span.text, "PG-CANARY-START");
    assert_eq!(fields[0].span.byte_offset, 0);
}

#[test]
fn stub_canary_marker_constant_matches_what_the_detector_actually_looks_for() {
    // Documentation-level sanity: the public constant genuinely drives detection, so a
    // caller building fixtures against it (as this file does) isn't relying on a stale doc
    // comment.
    let text = format!("prefix {STUB_CANARY_MARKER}X suffix");
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");
    assert_eq!(StubDetector.detect(&doc).len(), 1);
}

// ---------------------------------------------------------------------------
// Command-layer integration: import_document actually calls the detector and records
// detected_field_count + the audit `detect` event.
// ---------------------------------------------------------------------------

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
    mgr.create_account(CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: pg_core::config::RetentionPolicy::Discard,
    })
    .expect("confirm retention default");
    (mgr, dir)
}

/// dev-plan W12: "import of fixture yields known field_ids" — read here as "a known,
/// deterministic *count* of correctly-labeled fields," since `field_id` values are
/// randomly generated per detection (see module docs); `detected_field_count` is the
/// stable, API-visible signal `SessionManager` exposes.
#[test]
fn import_document_runs_the_stub_detector_when_installed() {
    let (mut mgr, _dir) = fresh_confirmed();
    let out = mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.".to_vec(),
            retention_override: None,
        })
        .expect("import_document");
    assert_eq!(out.summary.detected_field_count, 2);
}

#[test]
fn import_document_with_no_markers_has_zero_detected_fields() {
    let (mut mgr, _dir) = fresh_confirmed();
    let out = mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: b"Nothing synthetic in this letter at all.".to_vec(),
            retention_override: None,
        })
        .expect("import_document");
    assert_eq!(out.summary.detected_field_count, 0);
}

/// design §2.2: "Emit a detect event to the Audit Trail" — proven the same way
/// `catalog_w10.rs` proved the `import` event landed: one `import_document` call now
/// advances the audit chain by two rows (`import` then `detect`), and a subsequent
/// lock/unlock reports a clean (not degraded) integrity outcome at that tail sequence.
#[test]
fn import_document_appends_both_an_import_and_a_detect_audit_row() {
    let (mut mgr, _dir) = fresh_confirmed();
    mgr.import_document(ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: b"PG-CANARY-ONLY".to_vec(),
        retention_override: None,
    })
    .expect("import_document");

    mgr.lock().expect("lock");
    let out = mgr
        .unlock(pg_core::session::UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    assert_eq!(out.state, pg_core::session::SessionState::Unlocked);

    let report = mgr.get_integrity_report().expect("get_integrity_report");
    assert_eq!(report.kind, "ok");
    assert_eq!(report.tail_sequence, 2, "import + detect = two audit rows");
}

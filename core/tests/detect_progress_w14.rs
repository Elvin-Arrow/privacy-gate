//! W14 — `pg://detect-progress`.
//!
//! Spec sources:
//! - `docs/specs/api.md` §6 (`pg://detect-progress`: `{ doc_id, fraction, phase }`;
//!   `fraction` 0..1; `phase` is `"detecting"` until W15b adds Ollama warming)
//! - `docs/specs/ui.md` §7.2 (determinate bar from `{ fraction }` while import runs)
//! - `docs/dev-plan.md` W14 ("Tests first: in-process subscriber sees monotonic
//!   `fraction`; command tests don't require UI."; "Do not: fake 100% before detect
//!   finishes.")
//!
//! Seam: [`SessionManager::import_document`] emitting [`DetectProgress`] into a
//! [`ProgressSink`]. Tauri `emit` is W29; this chunk is the in-process contract that
//! shim will wrap. No UI.

use std::sync::{Arc, Mutex};

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::{DetectedField, DocumentStore};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::{Detector, StubDetector};
use pg_core::importer::Document;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    CreateAccountIn, DetectPhase, DetectProgress, ImportDocumentIn, ProgressSink, SessionManager,
    SetRetentionDefaultIn, DETECT_PROGRESS_EVENT,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";

struct RecordingSink {
    events: Mutex<Vec<DetectProgress>>,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
        })
    }

    fn snapshot(&self) -> Vec<DetectProgress> {
        self.events.lock().expect("progress sink").clone()
    }
}

impl ProgressSink for RecordingSink {
    fn emit_detect_progress(&self, event: DetectProgress) {
        self.events.lock().expect("progress sink").push(event);
    }
}

/// Records whether a `fraction >= 1.0` event had already been emitted when `detect` ran.
struct ProbeDetector {
    sink: Arc<RecordingSink>,
    complete_before_detect: Mutex<bool>,
}

impl Detector for ProbeDetector {
    fn id(&self) -> &'static str {
        "pg-probe-v1"
    }

    fn detect(&self, doc: &Document) -> Vec<DetectedField> {
        let faked = self
            .sink
            .snapshot()
            .iter()
            .any(|e| e.fraction >= 1.0);
        *self.complete_before_detect.lock().expect("probe") = faked;
        StubDetector.detect(doc)
    }
}

fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    (dir, path)
}

fn fresh_confirmed_with_sink(
    sink: Arc<dyn ProgressSink>,
    detector: Option<Arc<dyn Detector>>,
) -> (SessionManager, tempfile::TempDir) {
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
        .with_progress_sink(sink);
    if let Some(detector) = detector {
        mgr = mgr.with_detector(detector);
    }
    mgr.create_account(CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm retention default");
    (mgr, dir)
}

fn import_in(bytes: &[u8]) -> ImportDocumentIn {
    ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: bytes.to_vec(),
        retention_override: None,
    }
}

// ---------------------------------------------------------------------------
// api.md §6 event name
// ---------------------------------------------------------------------------

#[test]
fn detect_progress_event_name_matches_api_md() {
    assert_eq!(DETECT_PROGRESS_EVENT, "pg://detect-progress");
}

// ---------------------------------------------------------------------------
// dev-plan W14: "in-process subscriber sees monotonic fraction"
// ---------------------------------------------------------------------------

#[test]
fn import_document_emits_monotonic_detecting_fractions_ending_at_one() {
    let sink = RecordingSink::new();
    let (mut mgr, _dir) = fresh_confirmed_with_sink(sink.clone(), None);
    let out = mgr
        .import_document(import_in(b"PG-CANARY-X1 in the letter."))
        .expect("import_document");

    let events = sink.snapshot();
    assert!(
        events.len() >= 2,
        "need a start event and a completion event, got {events:?}"
    );
    for w in events.windows(2) {
        assert!(
            w[1].fraction >= w[0].fraction,
            "fraction must be monotonic, got {events:?}"
        );
    }
    assert!(events.iter().all(|e| (0.0..=1.0).contains(&e.fraction)));
    assert_eq!(events.last().unwrap().fraction, 1.0);
    assert!(events.iter().all(|e| e.phase == DetectPhase::Detecting));
    assert!(events.iter().all(|e| e.doc_id == out.summary.doc_id));
}

// ---------------------------------------------------------------------------
// dev-plan W14: "Do not: fake 100% before detect finishes."
// ---------------------------------------------------------------------------

#[test]
fn fraction_one_is_not_emitted_before_detect_runs() {
    let sink = RecordingSink::new();
    let probe = Arc::new(ProbeDetector {
        sink: sink.clone(),
        complete_before_detect: Mutex::new(true), // fail closed if detect is never called
    });
    let (mut mgr, _dir) =
        fresh_confirmed_with_sink(sink.clone(), Some(probe.clone()));
    mgr.import_document(import_in(b"PG-CANARY-X1"))
        .expect("import_document");

    assert!(
        !*probe.complete_before_detect.lock().expect("probe"),
        "fraction 1.0 must not be emitted until detect returns"
    );
    assert_eq!(sink.snapshot().last().unwrap().fraction, 1.0);
}

#[test]
fn retention_gate_emits_no_progress() {
    let sink = RecordingSink::new();
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
        .with_progress_sink(sink.clone());
    mgr.create_account(CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("create_account");
    // Unconfirmed factory default — import must refuse before detect (W11 / AC-7).
    let err = mgr.import_document(import_in(b"hello")).unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::RetentionPolicyUnset);
    assert!(
        sink.snapshot().is_empty(),
        "no detect-progress when detect never runs, got {:?}",
        sink.snapshot()
    );
    let _dir = dir;
}

#[test]
fn unsupported_document_emits_no_progress() {
    let sink = RecordingSink::new();
    let (mut mgr, _dir) = fresh_confirmed_with_sink(sink.clone(), None);
    let err = mgr.import_document(import_in(b"")).unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::UnsupportedDocument);
    assert!(
        sink.snapshot().is_empty(),
        "empty bytes never reach detect, got {:?}",
        sink.snapshot()
    );
}

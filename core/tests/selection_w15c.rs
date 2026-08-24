//! W15c — backend selection (`auto` / `bundled_only`) and audit detect honesty.
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.2 (`get_detector_preference` / `set_detector_preference`)
//! - `docs/specs/architecture.md` §10.1.3 (per-detect selection, not cached at unlock)
//! - `docs/specs/data-model.md` §5.8.1 detect payload (`backend` / `model_tag` /
//!   `fallback_reason`)
//! - `docs/dev-plan.md` W15c (matrix against the Ollama mock; AC stub path unaffected)
//!
//! Seam: [`SessionManager::import_document`] plus the two preference commands.
//! `with_detector` still overrides selection so AC-1..AC-4 can keep the stub.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::{AuditStore, EventType};
use pg_core::catalog::DocumentStore;
use pg_core::config::{ConfigStore, DetectorPreference, RetentionPolicy};
use pg_core::detector::{
    AllowlistEntry, Detector, StubDetector, HYBRID_OLLAMA_V1_ID, HYBRID_V1_ID,
    OLLAMA_ALLOWLISTED_TAG, STUB_DETECTOR_ID,
};
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    command_allowed, CreateAccountIn, DetectPhase, DetectProgress, ImportDocumentIn, ProgressSink,
    SessionManager, SessionState, SetDetectorPreferenceIn, SetRetentionDefaultIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const GOLDEN_NI: &str = "QQ123456C";
const GOLDEN_PERSON: &str = "Alice Example";
const FIXTURE_DIGEST: &str = "sha256:fixture-digest-not-a-real-model";

fn allowlist() -> Vec<AllowlistEntry> {
    vec![AllowlistEntry {
        tag: OLLAMA_ALLOWLISTED_TAG,
        digest: FIXTURE_DIGEST,
    }]
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

fn import_in(bytes: &[u8]) -> ImportDocumentIn {
    ImportDocumentIn {
        filename: "letter.txt".to_string(),
        bytes: bytes.to_vec(),
        retention_override: None,
    }
}

fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    (dir, path)
}

fn wired(
    vault: Arc<SqlCipherVault>,
    ollama: Option<(SocketAddr, Vec<AllowlistEntry>)>,
    detector: Option<Arc<dyn Detector>>,
) -> SessionManager {
    let keystore = Arc::new(InMemoryKeystore::new());
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault;
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents);
    if let Some((addr, al)) = ollama {
        mgr = mgr.with_ollama_endpoint(addr, al);
    }
    if let Some(d) = detector {
        mgr = mgr.with_detector(d);
    }
    mgr
}

fn confirm(mgr: &mut SessionManager) {
    mgr.create_account(create_in()).expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm retention");
}

fn last_detect_payload(vault: &SqlCipherVault) -> serde_json::Value {
    let rows = vault.replay().expect("replay");
    let detect = rows
        .iter()
        .rev()
        .find(|r| r.event_type == EventType::Detect)
        .expect("detect row");
    serde_json::from_str(&detect.payload_jcs).expect("detect payload json")
}

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

    fn clear(&self) {
        self.events.lock().expect("progress sink").clear();
    }
}

impl ProgressSink for RecordingSink {
    fn emit_detect_progress(&self, event: DetectProgress) {
        self.events.lock().expect("progress sink").push(event);
    }
}

// ---------------------------------------------------------------------------
// in-process Ollama mock (same contract as ollama_w15b)
// ---------------------------------------------------------------------------

struct MockState {
    tags_body: String,
    show_body: String,
    generate_body: String,
    tags_calls: u32,
    generate_calls: u32,
}

struct MockOllama {
    addr: SocketAddr,
    state: Arc<Mutex<MockState>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockOllama {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let state = Arc::new(Mutex::new(MockState {
            tags_body: serde_json::json!({
                "models": [{ "name": OLLAMA_ALLOWLISTED_TAG, "digest": FIXTURE_DIGEST, "size": 1 }]
            })
            .to_string(),
            show_body: serde_json::json!({
                "details": { "format": "gguf", "family": "gemma4" },
                "digest": FIXTURE_DIGEST
            })
            .to_string(),
            generate_body: {
                let inner = serde_json::json!({ "entities": [] }).to_string();
                serde_json::json!({ "model": OLLAMA_ALLOWLISTED_TAG, "response": inner, "done": true })
                    .to_string()
            },
            tags_calls: 0,
            generate_calls: 0,
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let state_t = Arc::clone(&state);
        let shutdown_t = Arc::clone(&shutdown);
        let thread = thread::spawn(move || loop {
            if shutdown_t.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => handle_conn(stream, &state_t),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });
        Self {
            addr,
            state,
            shutdown,
            thread: Some(thread),
        }
    }

    fn set_generate_entities(&self, entities: serde_json::Value) {
        let inner = serde_json::json!({ "entities": entities }).to_string();
        self.state.lock().expect("state").generate_body = serde_json::json!({
            "model": OLLAMA_ALLOWLISTED_TAG,
            "response": inner,
            "done": true
        })
        .to_string();
    }

    fn set_tags(&self, body: String) {
        self.state.lock().expect("state").tags_body = body;
    }

    fn set_show(&self, body: String) {
        self.state.lock().expect("state").show_body = body;
    }

    fn tags_calls(&self) -> u32 {
        self.state.lock().expect("state").tags_calls
    }

    fn generate_calls(&self) -> u32 {
        self.state.lock().expect("state").generate_calls
    }
}

impl Drop for MockOllama {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(50));
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, state: &Mutex<MockState>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut data = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = std::str::from_utf8(&data[..end]).unwrap_or("");
            let want = header.lines().find_map(|line| {
                let (n, v) = line.split_once(':')?;
                n.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse().ok())
                    .flatten()
            });
            if data.len() >= end + 4 + want.unwrap_or(0) {
                break;
            }
        }
    }
    let req = String::from_utf8_lossy(&data);
    let first = req.lines().next().unwrap_or("");
    let mut st = state.lock().expect("state");
    let (status, body) = if first.starts_with("GET /api/tags") {
        st.tags_calls += 1;
        (200, st.tags_body.clone())
    } else if first.starts_with("POST /api/show") {
        (200, st.show_body.clone())
    } else if first.starts_with("POST /api/generate") {
        st.generate_calls += 1;
        (200, st.generate_body.clone())
    } else {
        (404, "{}".into())
    };
    drop(st);
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

// ---------------------------------------------------------------------------
// api.md §5.2 preference commands
// ---------------------------------------------------------------------------

#[test]
fn factory_detector_preference_is_auto() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault, None, None);
    confirm(&mut mgr);
    let out = mgr.get_detector_preference().expect("get");
    assert_eq!(out.preference, DetectorPreference::Auto);
    let _dir = dir;
}

#[test]
fn set_detector_preference_persists_across_lock() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault, None, None);
    confirm(&mut mgr);
    let out = mgr
        .set_detector_preference(SetDetectorPreferenceIn {
            preference: DetectorPreference::BundledOnly,
        })
        .expect("set");
    assert_eq!(out.preference, DetectorPreference::BundledOnly);
    mgr.lock().expect("lock");
    mgr.unlock(pg_core::session::UnlockIn {
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("unlock");
    let read = mgr.get_detector_preference().expect("get");
    assert_eq!(read.preference, DetectorPreference::BundledOnly);
    let _dir = dir;
}

#[test]
fn detector_preference_commands_refused_before_unlock() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mgr = wired(vault, None, None);
    assert_eq!(
        mgr.get_detector_preference().unwrap_err().code,
        pg_core::api::ErrorCode::NotInSession
    );
    let _dir = dir;
}

#[test]
fn detector_preference_unregistered_while_degraded() {
    assert!(!command_allowed(
        "get_detector_preference",
        SessionState::DegradedIntegrity
    ));
    assert!(!command_allowed(
        "set_detector_preference",
        SessionState::DegradedIntegrity
    ));
}

#[test]
fn bundled_only_never_probes_ollama() {
    let mock = MockOllama::start();
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), Some((mock.addr, allowlist())), None);
    confirm(&mut mgr);
    mgr.set_detector_preference(SetDetectorPreferenceIn {
        preference: DetectorPreference::BundledOnly,
    })
    .expect("bundled_only");
    mgr.import_document(import_in(format!("NI {GOLDEN_NI}").as_bytes()))
        .expect("import");
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["detector_id"], HYBRID_V1_ID);
    assert_eq!(payload["backend"], "onnx");
    assert!(payload["fallback_reason"].is_null());
    assert_eq!(mock.tags_calls(), 0);
    assert_eq!(mock.generate_calls(), 0);
    let _dir = dir;
}

#[test]
fn auto_unreachable_falls_back_to_hybrid_with_reason() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(
        vault.clone(),
        Some((SocketAddr::from(([127, 0, 0, 1], 1)), allowlist())),
        None,
    );
    confirm(&mut mgr);
    mgr.import_document(import_in(format!("NI {GOLDEN_NI}").as_bytes()))
        .expect("import");
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["detector_id"], HYBRID_V1_ID);
    assert_eq!(payload["backend"], "onnx");
    assert_eq!(payload["fallback_reason"], "ollama_unreachable");
    let _dir = dir;
}

#[test]
fn auto_unallowlisted_falls_back_without_generate() {
    let mock = MockOllama::start();
    mock.set_tags(
        serde_json::json!({
            "models": [{ "name": "some-other:tag", "digest": FIXTURE_DIGEST, "size": 1 }]
        })
        .to_string(),
    );
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), Some((mock.addr, allowlist())), None);
    confirm(&mut mgr);
    mgr.import_document(import_in(format!("NI {GOLDEN_NI}").as_bytes()))
        .expect("import");
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["detector_id"], HYBRID_V1_ID);
    assert_eq!(payload["fallback_reason"], "model_not_allowlisted");
    assert_eq!(mock.generate_calls(), 0);
    let _dir = dir;
}

#[test]
fn auto_digest_mismatch_falls_back_without_generate() {
    let mock = MockOllama::start();
    mock.set_tags(
        serde_json::json!({
            "models": [{ "name": OLLAMA_ALLOWLISTED_TAG, "digest": "sha256:other", "size": 1 }]
        })
        .to_string(),
    );
    mock.set_show(
        serde_json::json!({
            "details": { "format": "gguf" },
            "digest": "sha256:other"
        })
        .to_string(),
    );
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), Some((mock.addr, allowlist())), None);
    confirm(&mut mgr);
    mgr.import_document(import_in(b"hello")).expect("import");
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["fallback_reason"], "digest_mismatch");
    assert_eq!(payload["detector_id"], HYBRID_V1_ID);
    assert_eq!(mock.generate_calls(), 0);
    let _dir = dir;
}

#[test]
fn auto_healthy_ollama_selects_ollama_backend() {
    let mock = MockOllama::start();
    mock.set_generate_entities(serde_json::json!([{
        "start": 0,
        "length": GOLDEN_PERSON.len(),
        "label": "person",
        "text": GOLDEN_PERSON
    }]));
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), Some((mock.addr, allowlist())), None);
    confirm(&mut mgr);
    let text = format!("{GOLDEN_PERSON}. NI {GOLDEN_NI}");
    let out = mgr
        .import_document(import_in(text.as_bytes()))
        .expect("import");
    assert!(out.summary.detected_field_count >= 2);
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["detector_id"], HYBRID_OLLAMA_V1_ID);
    assert_eq!(payload["backend"], "ollama");
    assert_eq!(payload["model_tag"], OLLAMA_ALLOWLISTED_TAG);
    assert!(payload["fallback_reason"].is_null());
    assert!(mock.generate_calls() >= 1);
    let _dir = dir;
}

#[test]
fn mid_document_offset_failure_falls_back_to_hybrid_not_partial_ner() {
    let mock = MockOllama::start();
    mock.set_generate_entities(serde_json::json!([{
        "start": 6,
        "length": GOLDEN_PERSON.len(),
        "label": "person",
        "text": GOLDEN_PERSON
    }]));
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), Some((mock.addr, allowlist())), None);
    confirm(&mut mgr);
    let out = mgr
        .import_document(import_in(format!("{GOLDEN_PERSON}. NI {GOLDEN_NI}").as_bytes()))
        .expect("import");
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["detector_id"], HYBRID_V1_ID);
    assert_eq!(payload["backend"], "onnx");
    assert_eq!(payload["fallback_reason"], "offset_verification_failed");
    let labels: Vec<String> = payload["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(!labels.iter().any(|l| l == "person"));
    assert!(labels.iter().any(|l| l == "uk_nino"));
    assert!(out.summary.detected_field_count >= 1);
    let _dir = dir;
}

#[test]
fn with_detector_stub_still_finds_canaries() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), None, Some(Arc::new(StubDetector)));
    confirm(&mut mgr);
    let out = mgr
        .import_document(import_in(b"PG-CANARY-X1 and PG-CANARY-X2"))
        .expect("import");
    assert_eq!(out.summary.detected_field_count, 2);
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["detector_id"], STUB_DETECTOR_ID);
    assert!(payload["backend"].is_null());
    assert!(payload["fallback_reason"].is_null());
    let _dir = dir;
}

#[test]
fn selection_is_per_detect_not_cached_at_unlock() {
    let mock = MockOllama::start();
    mock.set_generate_entities(serde_json::json!([{
        "start": 0,
        "length": GOLDEN_PERSON.len(),
        "label": "person",
        "text": GOLDEN_PERSON
    }]));
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(
        vault.clone(),
        Some((SocketAddr::from(([127, 0, 0, 1], 1)), allowlist())),
        None,
    );
    confirm(&mut mgr);
    mgr.import_document(import_in(b"hello")).expect("first");
    let first = last_detect_payload(&vault);
    assert_eq!(first["fallback_reason"], "ollama_unreachable");

    mgr.set_ollama_endpoint(mock.addr, allowlist());
    mgr.import_document(import_in(GOLDEN_PERSON.as_bytes()))
        .expect("second");
    let second = last_detect_payload(&vault);
    assert_eq!(second["detector_id"], HYBRID_OLLAMA_V1_ID);
    assert!(second["fallback_reason"].is_null());
    let _dir = dir;
}

#[test]
fn detector_preference_commands_refused_while_locked() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault, None, None);
    confirm(&mut mgr);
    mgr.lock().expect("lock");
    assert_eq!(
        mgr.get_detector_preference().unwrap_err().code,
        ErrorCode::NotInSession
    );
    assert_eq!(
        mgr.set_detector_preference(SetDetectorPreferenceIn {
            preference: DetectorPreference::BundledOnly,
        })
        .unwrap_err()
        .code,
        ErrorCode::NotInSession
    );
    let _dir = dir;
}

#[test]
fn set_detector_preference_does_not_confirm_retention() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault, None, None);
    mgr.create_account(create_in()).expect("create_account");
    mgr.set_detector_preference(SetDetectorPreferenceIn {
        preference: DetectorPreference::BundledOnly,
    })
    .expect("set preference");
    let retention = mgr.get_retention_default().expect("get retention");
    assert!(!retention.confirmed);
    assert_eq!(
        mgr.import_document(import_in(b"hello")).unwrap_err().code,
        ErrorCode::RetentionPolicyUnset
    );
    let _dir = dir;
}

#[test]
fn set_retention_default_preserves_detector_preference() {
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault, None, None);
    confirm(&mut mgr);
    mgr.set_detector_preference(SetDetectorPreferenceIn {
        preference: DetectorPreference::BundledOnly,
    })
    .expect("bundled_only");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Retain,
    })
    .expect("retention still writable");
    let pref = mgr.get_detector_preference().expect("get");
    assert_eq!(pref.preference, DetectorPreference::BundledOnly);
    let _dir = dir;
}

#[test]
fn auto_schema_failure_falls_back_without_generate() {
    let mock = MockOllama::start();
    mock.set_tags("not-json".into());
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault.clone(), Some((mock.addr, allowlist())), None);
    confirm(&mut mgr);
    mgr.import_document(import_in(b"hello")).expect("import");
    let payload = last_detect_payload(&vault);
    assert_eq!(payload["fallback_reason"], "schema_verification_failed");
    assert_eq!(payload["detector_id"], HYBRID_V1_ID);
    assert_eq!(mock.generate_calls(), 0);
    let _dir = dir;
}

#[test]
fn healthy_ollama_emits_warming_model_then_detecting() {
    let mock = MockOllama::start();
    let sink = RecordingSink::new();
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(vault, Some((mock.addr, allowlist())), None)
        .with_progress_sink(sink.clone());
    confirm(&mut mgr);
    mgr.import_document(import_in(GOLDEN_PERSON.as_bytes()))
        .expect("import");
    let events = sink.snapshot();
    assert!(
        events.iter().any(|e| e.phase == DetectPhase::WarmingModel),
        "successful handshake must emit warming_model, got {events:?}"
    );
    assert_eq!(events.last().unwrap().phase, DetectPhase::Detecting);
    assert_eq!(events.last().unwrap().fraction, 1.0);
    let _dir = dir;
}

#[test]
fn bundled_only_and_unreachable_never_emit_warming_model() {
    let sink = RecordingSink::new();
    let (dir, path) = temp_db_path();
    let vault = Arc::new(SqlCipherVault::new(path));
    let mut mgr = wired(
        vault,
        Some((SocketAddr::from(([127, 0, 0, 1], 1)), allowlist())),
        None,
    )
    .with_progress_sink(sink.clone());
    confirm(&mut mgr);
    mgr.set_detector_preference(SetDetectorPreferenceIn {
        preference: DetectorPreference::BundledOnly,
    })
    .expect("bundled_only");
    mgr.import_document(import_in(b"hello")).expect("bundled");
    assert!(
        sink.snapshot()
            .iter()
            .all(|e| e.phase == DetectPhase::Detecting),
        "bundled_only must not emit warming_model, got {:?}",
        sink.snapshot()
    );

    mgr.set_detector_preference(SetDetectorPreferenceIn {
        preference: DetectorPreference::Auto,
    })
    .expect("auto");
    sink.clear();
    mgr.import_document(import_in(b"hello")).expect("auto");
    assert!(
        sink.snapshot()
            .iter()
            .all(|e| e.phase == DetectPhase::Detecting),
        "unreachable auto must not emit warming_model, got {:?}",
        sink.snapshot()
    );
    let _dir = dir;
}

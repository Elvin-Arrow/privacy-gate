//! W28 — `list_audit_events` (AC-4).
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.8 (`list_audit_events` shape, filters, `AuditEventDto`,
//!   payload shapes per `EventType`, degraded-session behavior).
//! - `docs/specs/api.md` §2 (session gating — `list_audit_events` row: `no | no | yes |
//!   yes (verified prefix only)`).
//! - `docs/specs/srs.md` FR-7, AC-4.
//! - `docs/specs/testing.md` §6.4 AC-4 ("what did I share?"); C-API-1/2/5 (DTOs never
//!   carry span text, keys, or raw HMAC/chain material).
//! - `docs/dev-plan.md` W28 ("Tests first: AC-4 'what did I share?'; C-API-1/2 on DTOs."
//!   "Do not: webview HMAC verify.")
//!
//! Seam: [`SessionManager::list_audit_events`] — a read path over the audit chain W5
//! already writes and verifies; this chunk adds no new write.

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::{AuditStore, EventType};
use pg_core::catalog::DocumentStore;
use pg_core::cloud_ai::CloudAiSecret;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::session::{
    command_allowed, CommitShareIn, CreateAccountIn, FieldDecisionDto, FieldDecisionKind,
    ImportDocumentIn, ListAuditEventsIn, OpenApprovalIn, PreviewShareIn, SessionManager,
    SessionState, SetFieldDecisionsIn, SetRetentionDefaultIn, ShareKind, ShareRequestDto,
    SubmitApprovalIn, UnlockIn,
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
    let plugin_secrets: Arc<dyn pg_core::cloud_ai::PluginSecretStore> = vault.clone();
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents)
        .with_plugin_secrets(plugin_secrets)
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

fn ai_request(doc_ids: Vec<String>, instruction: Option<&str>) -> PreviewShareIn {
    PreviewShareIn {
        request: ShareRequestDto {
            kind: ShareKind::ShareToAi,
            doc_ids,
            per_doc_overrides: HashMap::new(),
            applied_variant_ids: HashMap::new(),
            recipient_note: Some("caseworker".to_string()),
            ai_instruction: instruction.map(str::to_string),
        },
    }
}

fn export_document(mgr: &mut SessionManager, doc_id: &str) {
    let preview = mgr
        .preview_share(export_request(vec![doc_id.to_string()]))
        .expect("preview");
    mgr.commit_share(CommitShareIn {
        preview_token: preview.preview_token,
    })
    .expect("commit");
}

// ---------------------------------------------------------------------------
// Mock Cloud AI HTTP server (same pattern as core/tests/cloud_ai_w27.rs).
// ---------------------------------------------------------------------------

struct MockState {
    status: u16,
    response_body: String,
    calls: u32,
}

struct MockCloudAi {
    addr: SocketAddr,
    // Kept alive for the lifetime of the mock (the accept-loop thread holds a clone);
    // this test file only ever reads through `secret()`/`url()`, not `state` directly.
    #[allow(dead_code)]
    state: Arc<Mutex<MockState>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn happy_response(text: &str) -> String {
    serde_json::json!({
        "choices": [
            { "message": { "role": "assistant", "content": text } }
        ]
    })
    .to_string()
}

impl MockCloudAi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(Mutex::new(MockState {
            status: 200,
            response_body: happy_response("ok"),
            calls: 0,
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let state_t = Arc::clone(&state);
        let shutdown_t = Arc::clone(&shutdown);
        let counter = Arc::new(AtomicU32::new(0));
        let thread = thread::spawn(move || loop {
            if shutdown_t.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    counter.fetch_add(1, Ordering::SeqCst);
                    handle_conn(stream, &state_t);
                }
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

    fn url(&self) -> String {
        format!("http://{}/v1/chat/completions", self.addr)
    }

    fn secret(&self, model: &str) -> CloudAiSecret {
        CloudAiSecret {
            endpoint_url: self.url(),
            model: model.to_string(),
            api_key: "sk-test-key-0123".to_string(),
            key_last4: "0123".to_string(),
        }
    }
}

impl Drop for MockCloudAi {
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
        if let Some(header_end) = find_double_crlf(&data) {
            let header = std::str::from_utf8(&data[..header_end]).unwrap_or("");
            let want = content_length(header).unwrap_or(0);
            if data.len() >= header_end + 4 + want {
                break;
            }
        }
    }
    let mut st = state.lock().expect("state");
    st.calls += 1;
    let status = st.status;
    let resp_body = st.response_body.clone();
    drop(st);
    reply(&mut stream, status, &resp_body);
}

fn find_double_crlf(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(header: &str) -> Option<usize> {
    for line in header.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn reply(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 { "OK" } else { "Error" };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

// ---------------------------------------------------------------------------
// Session gating (api.md §2 `list_audit_events` row).
// ---------------------------------------------------------------------------

#[test]
fn list_audit_events_gating() {
    assert!(!command_allowed("list_audit_events", SessionState::FirstRun));
    assert!(!command_allowed("list_audit_events", SessionState::Locked));
    assert!(command_allowed("list_audit_events", SessionState::Unlocked));
    assert!(command_allowed(
        "list_audit_events",
        SessionState::DegradedIntegrity
    ));
}

// ---------------------------------------------------------------------------
// AC-4: "what did I share?" — import -> detect -> approve -> export shows all four types,
// in order, for that doc_id.
// ---------------------------------------------------------------------------

#[test]
fn export_flow_audit_shows_import_detect_approve_share_in_order() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    export_document(&mut wired.mgr, &doc_id);

    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(doc_id.clone()),
            event_type: None,
            after_sequence: None,
            limit: None,
        })
        .expect("list_audit_events");

    let types: Vec<EventType> = out.events.iter().map(|e| e.event_type).collect();
    assert_eq!(
        types,
        vec![
            EventType::Import,
            EventType::Detect,
            EventType::Approve,
            EventType::Share,
        ]
    );
    for e in &out.events {
        assert_eq!(e.doc_id.as_deref(), Some(doc_id.as_str()));
    }
    assert_eq!(out.next_sequence, None, "fewer than a page: no more rows");

    // `no_originals_left_device` present only on the share event.
    for e in &out.events {
        match e.event_type {
            EventType::Share => assert!(e.no_originals_left_device.is_some()),
            _ => assert!(e.no_originals_left_device.is_none()),
        }
    }

    // Spot-check payload shapes (api.md §5.8) and C-API-2: no span text, no field
    // plaintext, no key material anywhere in any payload.
    let payloads_text: String = out
        .events
        .iter()
        .map(|e| e.payload.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!payloads_text.contains("PG-CANARY"));
    assert!(!payloads_text.contains(PASSPHRASE));

    let import_ev = out
        .events
        .iter()
        .find(|e| e.event_type == EventType::Import)
        .expect("import event");
    assert_eq!(import_ev.payload["retention"], "discard");
    assert_eq!(import_ev.payload["source_filename"], "letter.txt");
    assert!(import_ev.payload["detector_id"].is_null());

    let detect_ev = out
        .events
        .iter()
        .find(|e| e.event_type == EventType::Detect)
        .expect("detect event");
    assert!(detect_ev.payload["field_ids"].is_array());
    assert!(detect_ev.payload["labels"].is_array());

    let approve_ev = out
        .events
        .iter()
        .find(|e| e.event_type == EventType::Approve)
        .expect("approve event");
    let decisions = approve_ev.payload["decisions"]
        .as_array()
        .expect("decisions array");
    assert!(!decisions.is_empty());
    for d in decisions {
        assert!(d["field_id"].is_string());
        assert!(d["label"].is_string());
        assert!(d["decision"].is_string());
    }

    let share_ev = out
        .events
        .iter()
        .find(|e| e.event_type == EventType::Share)
        .expect("share event");
    assert_eq!(share_ev.payload["kind"], "export_to_person");
    assert_eq!(share_ev.payload["has_ai_instruction"], false);
    assert!(share_ev.payload["doc_ids"].is_array());
}

// ---------------------------------------------------------------------------
// AC-4 through the W27 Cloud AI path: read side proves has_ai_instruction: true with no
// instruction text (write side already guarantees this — this proves the DTO surfaces it).
// ---------------------------------------------------------------------------

#[test]
fn ai_share_flow_audit_shows_has_ai_instruction_true_with_no_instruction_text() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_secret(mock.secret("gpt-x"))
        .expect("test_only_set_cloud_ai_secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");

    let preview = wired
        .mgr
        .preview_share(ai_request(
            vec![doc_id.clone()],
            Some("Summarize this letter."),
        ))
        .expect("preview");
    wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit");

    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(doc_id),
            event_type: Some(EventType::Share),
            after_sequence: None,
            limit: None,
        })
        .expect("list_audit_events");

    assert_eq!(out.events.len(), 1);
    let share_ev = &out.events[0];
    assert_eq!(share_ev.payload["kind"], "share_to_ai");
    assert_eq!(share_ev.payload["has_ai_instruction"], true);
    let payload_text = share_ev.payload.to_string();
    assert!(!payload_text.contains("Summarize this letter"));
    assert!(!payload_text.contains("sk-test-key"));
    assert!(!payload_text.contains(PASSPHRASE));
}

// ---------------------------------------------------------------------------
// Pagination: more rows than one page.
// ---------------------------------------------------------------------------

#[test]
fn pagination_pages_through_every_row_exactly_once_in_order() {
    let mut wired = fresh_confirmed();
    // Each import+approve produces 3 audit rows (import/detect/approve); 3 docs => 9 rows.
    let mut doc_ids = Vec::new();
    for i in 0..3 {
        let doc_id = import_and_approve(&mut wired.mgr, &format!("letter-{i}.txt"));
        doc_ids.push(doc_id);
    }

    let mut seen_sequences: Vec<u64> = Vec::new();
    let mut cursor: Option<u64> = None;
    loop {
        let out = wired
            .mgr
            .list_audit_events(ListAuditEventsIn {
                doc_id: None,
                event_type: None,
                after_sequence: cursor,
                limit: Some(4),
            })
            .expect("list_audit_events");
        assert!(out.events.len() <= 4);
        for e in &out.events {
            seen_sequences.push(e.sequence);
        }
        match out.next_sequence {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(seen_sequences.len(), 9, "every row seen exactly once");
    let mut sorted = seen_sequences.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 9, "no duplicates");
    assert_eq!(seen_sequences, sorted, "ascending sequence order");
}

// ---------------------------------------------------------------------------
// doc_id filter isolates one document's events out of a multi-document session.
// ---------------------------------------------------------------------------

#[test]
fn doc_id_filter_isolates_one_document() {
    let mut wired = fresh_confirmed();
    let doc_a = import_and_approve(&mut wired.mgr, "a.txt");
    let doc_b = import_and_approve(&mut wired.mgr, "b.txt");

    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(doc_a.clone()),
            event_type: None,
            after_sequence: None,
            limit: None,
        })
        .expect("list_audit_events");

    assert_eq!(out.events.len(), 3, "import/detect/approve for doc_a only");
    for e in &out.events {
        assert_eq!(e.doc_id.as_deref(), Some(doc_a.as_str()));
        assert_ne!(e.doc_id.as_deref(), Some(doc_b.as_str()));
    }
}

// ---------------------------------------------------------------------------
// event_type filter isolates one type.
// ---------------------------------------------------------------------------

#[test]
fn event_type_filter_isolates_one_type() {
    let mut wired = fresh_confirmed();
    let _doc_a = import_and_approve(&mut wired.mgr, "a.txt");
    let _doc_b = import_and_approve(&mut wired.mgr, "b.txt");

    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: Some(EventType::Approve),
            after_sequence: None,
            limit: None,
        })
        .expect("list_audit_events");

    assert_eq!(out.events.len(), 2);
    for e in &out.events {
        assert_eq!(e.event_type, EventType::Approve);
    }
}

// ---------------------------------------------------------------------------
// Degraded-integrity session: only the verified prefix (sequences < first_bad_sequence).
// ---------------------------------------------------------------------------

#[test]
fn degraded_session_returns_only_the_verified_prefix() {
    let mut wired = fresh_confirmed();
    // import_and_approve appends sequence 1 (import), 2 (detect), 3 (approve).
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let _ = doc_id;

    wired
        .vault
        .test_only_corrupt_payload(2)
        .expect("corrupt payload 2 (detect)");
    wired.mgr.lock().expect("lock");
    let out = wired
        .mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds — passphrase is correct");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let integrity = out.integrity.expect("degraded unlock carries a report");
    assert_eq!(integrity.first_bad_sequence, Some(2));

    let listed = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: None,
        })
        .expect("list_audit_events allowed while degraded");
    assert_eq!(listed.events.len(), 1, "only sequence 1 is before first_bad_sequence");
    assert_eq!(listed.events[0].sequence, 1);
    assert_eq!(listed.events[0].event_type, EventType::Import);
}

// ---------------------------------------------------------------------------
// Out-of-range explicit `limit` is `invalid_input` (precedent: ai_instruction 1..=4000,
// save_variant name length — explicit out-of-range values are rejected, not clamped).
// ---------------------------------------------------------------------------

#[test]
fn zero_limit_is_invalid_input() {
    let wired = fresh_confirmed();
    let err = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: Some(0),
        })
        .expect_err("limit 0 is out of range");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn limit_over_200_is_invalid_input() {
    let wired = fresh_confirmed();
    let err = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: Some(201),
        })
        .expect_err("limit 201 is out of range");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn limit_absent_defaults_to_50() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(doc_id),
            event_type: None,
            after_sequence: None,
            limit: None,
        })
        .expect("list_audit_events");
    assert!(out.events.len() <= 50);
}

#[test]
fn not_in_session_before_unlock() {
    let mut dir_dropped_at_end = tempfile::tempdir().expect("temp dir");
    let path = dir_dropped_at_end.path().join("vault.db");
    let vault = Arc::new(SqlCipherVault::new(path.clone()));
    let keystore = Arc::new(pg_core::keystore::InMemoryKeystore::new());
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let mgr = SessionManager::new_full(keystore, accounts, backend, audit, config);
    let err = mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: None,
        })
        .expect_err("first_run: not_in_session");
    assert_eq!(err.code, ErrorCode::NotInSession);
    let _ = &mut dir_dropped_at_end;
}

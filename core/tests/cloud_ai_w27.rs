//! W27 — Cloud AI plugin (mock HTTP), AC-3.
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §8 (plugin host capabilities), §9 (Cloud AI
//!   authentication and network: where the secret lives, which process speaks HTTP,
//!   redirect-refusal, failure auditing).
//! - `docs/specs/api.md` §5.6 (`preview_share`/`commit_share` AI kind — identity
//!   guarantee, `cloud_ai_not_configured` before assembling), §5.7 (`cloud_ai_set_config`
//!   / `get` / `clear` / `test`).
//! - `docs/specs/testing.md` §6.3 AC-3; C-API-4 (key never in outputs); C-TEST-7 (OQ-6
//!   oracle covers the AI payload too).
//! - `docs/dev-plan.md` W27 ("Tests first: not configured -> `cloud_ai_not_configured`
//!   before assemble; mock server receives **approved** body identical to preview;
//!   redacted canaries absent (OQ-6); redirect-to-other-host refused; `get` has
//!   `key_last4` only." "Do not: bundled API key; send originals.")
//!
//! # TLS-mock gap
//! There is no TLS testing double anywhere in this repo. `endpoint_url` must be
//! `https://` at `cloud_ai_set_config` time — that check is unit-tested directly in
//! `core/src/cloud_ai.rs`. The HTTP-send tests here reach a plain-HTTP mock via
//! [`SessionManager::test_only_set_cloud_ai_secret`], which bypasses that validation the
//! same way production `cloud_ai_set_config` never would (see `core/src/cloud_ai.rs`
//! module docs for what this does and does not prove).
//!
//! Seam: [`SessionManager::cloud_ai_set_config`] / `_get_config` / `_clear_config` /
//! `_test`, and `preview_share` / `commit_share`'s `share_to_ai` kind.

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
    command_allowed, CloudAiSetConfigIn, CommitShareIn, CreateAccountIn, FieldDecisionDto,
    FieldDecisionKind, ImportDocumentIn, OpenApprovalIn, PreviewShareIn, SessionManager,
    SessionState, SetFieldDecisionsIn, SetRetentionDefaultIn, ShareKind, ShareRequestDto,
    SubmitApprovalIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const BODY: &[u8] = b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.";

// ---------------------------------------------------------------------------
// Wiring (same shape as core/tests/share_w24.rs).
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Mock Cloud AI HTTP server: plain HTTP, one endpoint, OpenAI-compatible chat-completion
// response shape (`core/src/cloud_ai.rs`'s wire format). Same `TcpListener` pattern as
// `core/tests/ollama_w15b.rs`'s `MockOllama`.
// ---------------------------------------------------------------------------

struct MockState {
    status: u16,
    response_body: String,
    redirect_location: Option<String>,
    calls: u32,
    bodies: Vec<String>,
}

struct MockCloudAi {
    addr: SocketAddr,
    state: Arc<Mutex<MockState>>,
    connections: Arc<AtomicU32>,
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
            redirect_location: None,
            calls: 0,
            bodies: Vec::new(),
        }));
        let connections = Arc::new(AtomicU32::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let state_t = Arc::clone(&state);
        let conn_t = Arc::clone(&connections);
        let shutdown_t = Arc::clone(&shutdown);
        let thread = thread::spawn(move || loop {
            if shutdown_t.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    conn_t.fetch_add(1, Ordering::SeqCst);
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
            connections,
            shutdown,
            thread: Some(thread),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/v1/chat/completions", self.addr)
    }

    fn set_response(&self, status: u16, body: String) {
        let mut s = self.state.lock().expect("state");
        s.status = status;
        s.response_body = body;
        s.redirect_location = None;
    }

    fn set_redirect(&self, location: &str) {
        let mut s = self.state.lock().expect("state");
        s.status = 302;
        s.redirect_location = Some(location.to_string());
    }

    fn calls(&self) -> u32 {
        self.state.lock().expect("state").calls
    }

    fn bodies(&self) -> Vec<String> {
        self.state.lock().expect("state").bodies.clone()
    }

    fn connections(&self) -> u32 {
        self.connections.load(Ordering::SeqCst)
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
    let req = String::from_utf8_lossy(&data);
    let body = req.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

    let mut st = state.lock().expect("state");
    st.calls += 1;
    st.bodies.push(body);
    if let Some(loc) = st.redirect_location.clone() {
        let _ = write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }
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
// Session gating (api.md §2 generic config/document row).
// ---------------------------------------------------------------------------

#[test]
fn cloud_ai_commands_unlocked_only() {
    for command in [
        "cloud_ai_set_config",
        "cloud_ai_get_config",
        "cloud_ai_clear_config",
        "cloud_ai_test",
    ] {
        assert!(!command_allowed(command, SessionState::FirstRun));
        assert!(!command_allowed(command, SessionState::Locked));
        assert!(command_allowed(command, SessionState::Unlocked));
        assert!(!command_allowed(command, SessionState::DegradedIntegrity));
    }
}

// ---------------------------------------------------------------------------
// `cloud_ai_set_config` validation.
// ---------------------------------------------------------------------------

#[test]
fn set_config_rejects_http_scheme() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "http://api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-abc".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn set_config_rejects_file_scheme() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "file:///etc/passwd".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-abc".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn set_config_rejects_userinfo() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://user:pass@api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-abc".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn set_config_accepts_a_valid_https_endpoint_and_returns_host_and_last4() {
    let mut wired = fresh_confirmed();
    let out = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1/chat".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-abcd1234".to_string(),
        })
        .expect("set_config");
    assert!(out.configured);
    assert_eq!(out.endpoint_host, "api.example.com");
    assert_eq!(out.model, "gpt-x");
    assert_eq!(out.key_last4, "1234");
}

// ---------------------------------------------------------------------------
// `cloud_ai_get_config` never returns the key.
// ---------------------------------------------------------------------------

#[test]
fn get_config_never_includes_the_api_key() {
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-super-secret-value".to_string(),
        })
        .expect("set_config");
    let out = wired.mgr.cloud_ai_get_config().expect("get_config");
    assert!(out.configured);
    assert_eq!(out.key_last4.as_deref(), Some("alue"));
    let serialized = serde_json::to_string(&out).expect("serialize");
    assert!(!serialized.contains("sk-super-secret-value"));
    assert!(!serialized.contains("api_key"));
}

#[test]
fn get_config_when_unconfigured_reports_configured_false() {
    let wired = fresh_confirmed();
    let out = wired.mgr.cloud_ai_get_config().expect("get_config");
    assert!(!out.configured);
    assert!(out.endpoint_url.is_none());
    assert!(out.key_last4.is_none());
}

#[test]
fn clear_config_makes_it_unconfigured_and_is_idempotent() {
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-abcd1234".to_string(),
        })
        .expect("set_config");
    let out = wired.mgr.cloud_ai_clear_config().expect("clear");
    assert!(!out.configured);
    assert!(!wired.mgr.cloud_ai_get_config().expect("get").configured);
    // Idempotent — clearing again is still Ok.
    let out2 = wired.mgr.cloud_ai_clear_config().expect("clear again");
    assert!(!out2.configured);
}

// ---------------------------------------------------------------------------
// AC-3: not configured -> `cloud_ai_not_configured` before assembling anything.
// ---------------------------------------------------------------------------

#[test]
fn share_to_ai_without_config_is_not_configured_and_touches_no_mock_connection() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let err = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], Some("Summarize this.")))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNotConfigured);
    assert_eq!(mock.connections(), 0, "no HTTP attempted before configured check");
}

// ---------------------------------------------------------------------------
// Empty ai_instruction -> invalid_input.
// ---------------------------------------------------------------------------

#[test]
fn empty_ai_instruction_is_invalid_input() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let err = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], Some("")))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn missing_ai_instruction_is_invalid_input() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let err = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], None))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------------------
// Full round trip against the mock: preview shows the approved text, commit POSTs the
// byte-identical body, redacted canaries never appear (OQ-6).
// ---------------------------------------------------------------------------

fn configure(wired: &mut Wired, mock: &MockCloudAi) {
    wired
        .mgr
        .test_only_set_cloud_ai_secret(mock.secret("gpt-x"))
        .expect("test_only_set_cloud_ai_secret");
}

#[test]
fn ac3_mock_receives_the_approved_body_identical_to_the_preview_and_no_redacted_canary() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    configure(&mut wired, &mock);
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");

    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], Some("Summarize this letter.")))
        .expect("preview");
    assert!(preview.pdf_bytes.is_none());
    assert!(preview.suggested_filename.is_none());
    let ai_text = preview.ai_payload_preview.clone().expect("ai payload");
    assert!(ai_text.contains("PG-CANARY-X2"));
    assert!(!ai_text.contains("PG-CANARY-X1"));

    let commit = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit");
    assert_eq!(commit.kind, ShareKind::ShareToAi);
    assert_eq!(commit.output_text.as_deref(), Some("ok"));
    assert!(commit.pdf_bytes.is_none());

    assert_eq!(mock.calls(), 1);
    let bodies = mock.bodies();
    let sent = &bodies[0];
    // The exact approved-document body POSTed is identical to `ai_payload_preview` — the
    // plugin only wraps it with the instruction/preamble, never mutates it (api.md §5.6).
    assert!(sent.contains(&ai_text), "sent body must contain the exact preview text verbatim");

    // `common::oracle::check`'s "kept" arm is PDF-extraction-based (W25) and does not apply
    // to a plain-text AI payload; its "redacted" arm is a raw-byte scan and does apply here.
    common::oracle::check(sent.as_bytes(), &["PG-CANARY-X1"], &[])
        .expect("OQ-6 oracle holds for the AI payload");
    assert!(sent.contains("PG-CANARY-X2"));

    // The audit trail records the attempt but never the instruction text or the response.
    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains("\"has_ai_instruction\":true"));
    assert!(!share.payload_jcs.contains("Summarize this letter"));
    assert!(!share.payload_jcs.contains("PG-CANARY"));
    assert!(!share.payload_jcs.contains("sk-test-key"));
}

// ---------------------------------------------------------------------------
// Redirect to a different host is refused.
// ---------------------------------------------------------------------------

#[test]
fn redirect_to_a_different_host_is_refused() {
    let mock = MockCloudAi::start();
    mock.set_redirect("http://127.0.0.2:9/somewhere-else");
    let mut wired = fresh_confirmed();
    configure(&mut wired, &mock);
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");

    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], Some("Summarize this letter.")))
        .expect("preview");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiRefused);

    // The failed send is still audited, with an error class and no secret material.
    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains("\"error_class\":\"refused\""));
    assert!(!share.payload_jcs.contains("sk-test-key"));
}

// ---------------------------------------------------------------------------
// Failed HTTP (non-redirect) still audits the attempt with an error class.
// ---------------------------------------------------------------------------

#[test]
fn failed_http_still_audits_the_attempt_with_error_class_and_no_secret() {
    let mock = MockCloudAi::start();
    mock.set_response(500, "server error".to_string());
    let mut wired = fresh_confirmed();
    configure(&mut wired, &mock);
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");

    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], Some("Summarize this letter.")))
        .expect("preview");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNetwork);

    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains("\"error_class\":\"network\""));
    assert!(!share.payload_jcs.contains("sk-test-key"));
    assert!(!share.payload_jcs.contains("server error"));
}

// ---------------------------------------------------------------------------
// `cloud_ai_test` sends no vault document content — a lightweight probe.
// ---------------------------------------------------------------------------

#[test]
fn cloud_ai_test_sends_no_document_content() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    configure(&mut wired, &mock);
    // A doc-bearing session exists, but `cloud_ai_test` must not touch it.
    let _doc_id = import_and_approve(&mut wired.mgr, "letter.txt");

    let out = wired.mgr.cloud_ai_test().expect("cloud_ai_test");
    assert!(out.ok);
    assert!(out.error_class.is_none());
    assert_eq!(mock.calls(), 1);
    let bodies = mock.bodies();
    assert!(!bodies[0].contains("PG-CANARY"));
    assert!(!bodies[0].contains("Dear Sir"));
}

#[test]
fn cloud_ai_test_without_config_reports_not_ok() {
    let wired = fresh_confirmed();
    let out = wired.mgr.cloud_ai_test().expect("cloud_ai_test");
    assert!(!out.ok);
    assert!(out.error_class.is_some());
}

#[test]
fn cloud_ai_test_reports_failure_class_on_a_bad_endpoint() {
    let mock = MockCloudAi::start();
    mock.set_response(500, "nope".to_string());
    let mut wired = fresh_confirmed();
    configure(&mut wired, &mock);
    let out = wired.mgr.cloud_ai_test().expect("cloud_ai_test");
    assert!(!out.ok);
    assert_eq!(out.error_class.as_deref(), Some("network"));
}

//! W27 — Cloud AI plugin (mock HTTP).
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §8–§9 (plugin host capabilities; credential storage;
//!   Rust-side-only HTTP; TLS; allowlist; redirect refusal)
//! - `docs/specs/api.md` §5.6 (`share_to_ai` on `preview_share`/`commit_share`), §5.7
//!   (`cloud_ai_set_config` / `get` / `clear` / `test`)
//! - `docs/dev-plan.md` W27 ("Tests first: not configured → `cloud_ai_not_configured`
//!   before assemble; mock server receives **approved** body identical to preview;
//!   redacted canaries absent (OQ-6); redirect-to-other-host refused; `get` has
//!   `key_last4` only... Failed HTTP still audits attempt")
//! - `docs/specs/testing.md` §6.3 AC-3
//!
//! Seam: [`SessionManager::cloud_ai_set_config`] / `get` / `clear` / `test`, and the
//! `share_to_ai` branch of `preview_share`/`commit_share`. No real vendor in CI: every test
//! here runs against an in-process HTTP mock, same style as `ollama_w15b.rs`'s
//! `MockOllama` (`docs/specs/testing.md` §10).

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
use pg_core::cloud_ai::{key_last4, CloudAiSecret, CloudAiStore};
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
const KEEP_CANARY: &str = "PG-CANARY-AI-KEEP1";
const REDACT_CANARY: &str = "PG-CANARY-AI-REDACT-9Z2Q";
const BODY: &[u8] = b"Dear Sir, we discuss PG-CANARY-AI-KEEP1 and PG-CANARY-AI-REDACT-9Z2Q here.";

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
    let cloud_ai: Arc<dyn CloudAiStore> = vault.clone();
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents)
        .with_cloud_ai(cloud_ai)
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
                decision: if text.contains(REDACT_CANARY) {
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

fn ai_request(doc_ids: Vec<String>, instruction: &str) -> PreviewShareIn {
    PreviewShareIn {
        request: ShareRequestDto {
            kind: ShareKind::ShareToAi,
            doc_ids,
            per_doc_overrides: std::collections::HashMap::new(),
            applied_variant_ids: std::collections::HashMap::new(),
            recipient_note: None,
            ai_instruction: Some(instruction.to_string()),
        },
    }
}

fn secret_for(mock: &MockCloudAi, model: &str, api_key: &str) -> CloudAiSecret {
    CloudAiSecret {
        endpoint_url: format!("http://{}/v1/chat/completions", mock.addr),
        model: model.to_string(),
        api_key: api_key.to_string(),
        key_last4: key_last4(api_key),
    }
}

// ---------------------------------------------------------------------------
// In-process Cloud AI mock (testing.md §10), same shape as `ollama_w15b.rs`'s MockOllama.
// ---------------------------------------------------------------------------

fn happy_chat_response(text: &str) -> String {
    serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": text } }]
    })
    .to_string()
}

struct MockState {
    status: u16,
    body: String,
    redirect_location: Option<String>,
    requests: Vec<String>,
}

struct MockCloudAi {
    addr: SocketAddr,
    state: Arc<Mutex<MockState>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockCloudAi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(Mutex::new(MockState {
            status: 200,
            body: happy_chat_response("a helpful, redaction-respecting summary"),
            redirect_location: None,
            requests: Vec::new(),
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

    /// A loopback address nothing is listening on — used for the "network failure"
    /// case (connect refused), same trick as binding-then-dropping would give, but without
    /// the bind-then-drop race.
    fn dead_addr() -> SocketAddr {
        "127.0.0.1:1".parse().expect("literal addr")
    }

    fn set_status(&self, status: u16, body: &str) {
        let mut s = self.state.lock().expect("state");
        s.status = status;
        s.body = body.to_string();
    }

    fn set_redirect(&self, location: &str) {
        let mut s = self.state.lock().expect("state");
        s.redirect_location = Some(location.to_string());
    }

    fn requests(&self) -> Vec<String> {
        self.state.lock().expect("state").requests.clone()
    }

    fn call_count(&self) -> usize {
        self.requests().len()
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
    st.requests.push(body);
    if let Some(loc) = st.redirect_location.clone() {
        let _ = write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }
    reply(&mut stream, st.status, &st.body);
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
// Raw-egress oracle for the AI JSON body (testing.md §7.2). No PDF/FlateDecode involved,
// so unlike `common::oracle`, this is a plain substring scan of the HTTP body bytes.
// ---------------------------------------------------------------------------

fn assert_absent(haystack: &str, needle: &str) {
    assert!(
        !haystack.contains(needle),
        "canary {needle:?} leaked into egress: {haystack:?}"
    );
}

fn assert_present(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "keep canary {needle:?} missing from egress: {haystack:?}"
    );
}

// ---------------------------------------------------------------------------
// Session gating (api.md §2)
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
// cloud_ai_set_config / get / clear (api.md §5.7)
// ---------------------------------------------------------------------------

#[test]
fn set_config_rejects_non_https_and_userinfo() {
    let mut wired = fresh_confirmed();
    for bad in [
        "http://api.example.com/v1",
        "file:///etc/passwd",
        "https://user:pass@api.example.com/v1",
        "not-a-url",
    ] {
        let err = wired
            .mgr
            .cloud_ai_set_config(CloudAiSetConfigIn {
                endpoint_url: bad.to_string(),
                model: "gpt-test".to_string(),
                api_key: "sk-test".to_string(),
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "url {bad:?}");
    }
    let get = wired.mgr.cloud_ai_get_config().expect("get");
    assert!(!get.configured);
}

#[test]
fn set_config_rejects_empty_model_and_key() {
    let mut wired = fresh_confirmed();
    let err = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: String::new(),
            api_key: "sk-test".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    let err = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-test".to_string(),
            api_key: String::new(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn set_config_then_get_reports_host_model_and_last4_never_the_key() {
    let mut wired = fresh_confirmed();
    let out = wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com:8443/v1/chat".to_string(),
            model: "gpt-test".to_string(),
            api_key: "sk-abcdefgh".to_string(),
        })
        .expect("set_config");
    assert!(out.configured);
    assert_eq!(out.endpoint_host, "api.example.com:8443");
    assert_eq!(out.model, "gpt-test");
    assert_eq!(out.key_last4, "efgh");

    let get = wired.mgr.cloud_ai_get_config().expect("get_config");
    assert!(get.configured);
    assert_eq!(get.endpoint_host.as_deref(), Some("api.example.com:8443"));
    assert_eq!(get.model.as_deref(), Some("gpt-test"));
    assert_eq!(get.key_last4.as_deref(), Some("efgh"));
    // Structural guarantee, not just a runtime check: `CloudAiGetConfigOut` has no
    // `api_key` field at all (architecture §9.1) — see the type definition in
    // `pg_core::session`.
}

#[test]
fn clear_config_reports_unconfigured_and_is_idempotent() {
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-test".to_string(),
            api_key: "sk-test".to_string(),
        })
        .expect("set_config");
    let cleared = wired.mgr.cloud_ai_clear_config().expect("clear once");
    assert!(!cleared.configured);
    assert!(!wired.mgr.cloud_ai_get_config().expect("get").configured);
    // Idempotent: clearing an already-unconfigured secret is not an error.
    let cleared_again = wired.mgr.cloud_ai_clear_config().expect("clear twice");
    assert!(!cleared_again.configured);
}

// ---------------------------------------------------------------------------
// cloud_ai_test (api.md §5.7; C-API-4: sends no vault document content)
// ---------------------------------------------------------------------------

#[test]
fn test_without_config_is_not_configured() {
    let wired = fresh_confirmed();
    let err = wired.mgr.cloud_ai_test().unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNotConfigured);
}

#[test]
fn test_against_mock_ok_sends_no_document_content() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let out = wired.mgr.cloud_ai_test().expect("test");
    assert!(out.ok);
    assert!(out.error_class.is_none());
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    assert!(
        !reqs[0].contains("\"document\""),
        "cloud_ai_test must never send document content: {:?}",
        reqs[0]
    );
}

#[test]
fn test_against_mock_refused_status() {
    let mock = MockCloudAi::start();
    mock.set_status(401, r#"{"error":"bad key"}"#);
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-wrong"))
        .expect("store secret");
    let out = wired.mgr.cloud_ai_test().expect("test");
    assert!(!out.ok);
    assert_eq!(out.error_class.as_deref(), Some("cloud_ai_refused"));
}

#[test]
fn test_against_unreachable_host_is_network_error() {
    let mut wired = fresh_confirmed();
    let secret = CloudAiSecret {
        endpoint_url: format!("http://{}/v1/chat", MockCloudAi::dead_addr()),
        model: "gpt-test".to_string(),
        api_key: "sk-test".to_string(),
        key_last4: "test".to_string(),
    };
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret)
        .expect("store secret");
    let out = wired.mgr.cloud_ai_test().expect("test");
    assert!(!out.ok);
    assert_eq!(out.error_class.as_deref(), Some("cloud_ai_network"));
}

// ---------------------------------------------------------------------------
// preview_share (share_to_ai) — validation ahead of the existing
// `share_to_ai_is_cloud_ai_not_configured` test in `share_w24.rs`.
// ---------------------------------------------------------------------------

#[test]
fn preview_ai_requires_non_empty_instruction() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let err = wired
        .mgr
        .preview_share(ai_request(vec![doc_id.clone()], ""))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    let too_long = "x".repeat(4001);
    let err = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], &too_long))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(mock.call_count(), 0, "invalid input must not touch the network");
}

#[test]
fn preview_ai_rejects_a_recipient_note() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let mut req = ai_request(vec![doc_id], "Summarise this.");
    req.request.recipient_note = Some("for the caseworker".to_string());
    let err = wired.mgr.preview_share(req).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn preview_ai_fails_before_touching_the_network_when_unconfigured() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let err = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], "Summarise this."))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNotConfigured);
    assert_eq!(mock.call_count(), 0);
}

// ---------------------------------------------------------------------------
// AC-3 — Cloud AI, approved content only (testing.md §6.3)
// ---------------------------------------------------------------------------

#[test]
fn commit_ai_posts_identical_body_and_audits_share_with_no_instruction_text() {
    let mock = MockCloudAi::start();
    mock.set_status(200, &happy_chat_response("a redaction-respecting summary"));
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");

    let instruction = "Summarise this letter in one sentence.";
    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id.clone()], instruction))
        .expect("preview");
    assert_eq!(preview.kind, ShareKind::ShareToAi);
    assert!(preview.pdf_bytes.is_none());
    assert!(preview.suggested_filename.is_none());
    let payload_preview = preview.ai_payload_preview.clone().expect("ai payload");
    assert_present(&payload_preview, KEEP_CANARY);
    assert_absent(&payload_preview, REDACT_CANARY);
    // The instruction itself is never in the preview body (api.md §5.6).
    assert_absent(&payload_preview, instruction);

    let commit = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token.clone(),
        })
        .expect("commit");
    assert_eq!(commit.kind, ShareKind::ShareToAi);
    assert!(commit.pdf_bytes.is_none());
    assert_eq!(
        commit.output_text.as_deref(),
        Some("a redaction-respecting summary")
    );
    assert!(commit.audit_event_id >= 1);

    // Identity guarantee (api.md §5.6): the exact `document` field POSTed equals the
    // preview body, byte for byte.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    let sent: serde_json::Value = serde_json::from_str(&reqs[0]).expect("json body");
    assert_eq!(
        sent.get("document").and_then(serde_json::Value::as_str),
        Some(payload_preview.as_str())
    );
    assert_eq!(sent.get("model").and_then(serde_json::Value::as_str), Some("gpt-test"));

    // OQ-6 oracle on the raw wire bytes (§7.2): no redacted plaintext anywhere in the
    // egress; the keep canary is present; the instruction the user typed does appear
    // (it is sent, just not in the `document` field, and it is not vault document text).
    assert_absent(&reqs[0], REDACT_CANARY);
    assert_present(&reqs[0], KEEP_CANARY);

    // A second commit with the same (now-dropped) token is `preview_expired`, not a
    // second HTTP call (api.md §5.6 "Drops the token after success").
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PreviewExpired);
    assert_eq!(mock.call_count(), 1);

    // Audit: `share_to_ai`, has_ai_instruction true, no instruction text, no canaries,
    // doc_id present, error_class null.
    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains(&doc_id));
    assert!(share.payload_jcs.contains("share_to_ai"));
    assert!(share.payload_jcs.contains("\"has_ai_instruction\":true"));
    assert!(share.payload_jcs.contains("\"error_class\":null"));
    assert_absent(&share.payload_jcs, instruction);
    assert_absent(&share.payload_jcs, REDACT_CANARY);
    assert_absent(&share.payload_jcs, KEEP_CANARY);
}

#[test]
fn commit_ai_network_failure_still_audits_the_attempt_and_drops_the_token() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let unreachable = CloudAiSecret {
        endpoint_url: format!("http://{}/v1/chat", MockCloudAi::dead_addr()),
        model: "gpt-test".to_string(),
        api_key: "sk-test".to_string(),
        key_last4: "test".to_string(),
    };
    wired
        .mgr
        .test_only_set_cloud_ai_config(unreachable)
        .expect("store secret");
    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id.clone()], "Summarise this."))
        .expect("preview");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token.clone(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNetwork);

    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit recorded even on failure");
    assert!(share.payload_jcs.contains("\"error_class\":\"cloud_ai_network\""));

    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PreviewExpired, "token dropped after definitive failure");
}

#[test]
fn commit_ai_refused_status_still_audits_the_attempt() {
    let mock = MockCloudAi::start();
    mock.set_status(500, r#"{"error":"server exploded"}"#);
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], "Summarise this."))
        .expect("preview");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiRefused);

    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share.payload_jcs.contains("\"error_class\":\"cloud_ai_refused\""));
}

#[test]
fn commit_ai_not_configured_when_secret_cleared_after_preview() {
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let preview = wired
        .mgr
        .preview_share(ai_request(vec![doc_id], "Summarise this."))
        .expect("preview");
    wired.mgr.cloud_ai_clear_config().expect("clear");
    let err = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNotConfigured);
    assert_eq!(mock.call_count(), 0, "must not send once unconfigured");

    let share = wired
        .vault
        .replay()
        .expect("replay")
        .into_iter()
        .find(|r| r.event_type == EventType::Share)
        .expect("share audit");
    assert!(share
        .payload_jcs
        .contains("\"error_class\":\"cloud_ai_not_configured\""));
}

// ---------------------------------------------------------------------------
// Redirect refusal (architecture §9.2)
// ---------------------------------------------------------------------------

#[test]
fn redirect_to_another_host_is_refused_not_followed() {
    let mock = MockCloudAi::start();
    mock.set_redirect("http://203.0.113.1/v1/chat");
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret_for(&mock, "gpt-test", "sk-test"))
        .expect("store secret");
    let out = wired.mgr.cloud_ai_test().expect("test");
    assert!(!out.ok);
    assert_eq!(out.error_class.as_deref(), Some("cloud_ai_refused"));
    // Exactly one request — the redirect target was never dialed.
    assert_eq!(mock.call_count(), 1);
}

// ---------------------------------------------------------------------------
// Informational: real Ollama Cloud (testing.md §11 style, mirrors
// `ollama_w15b.rs`'s `nightly_real_ollama_handshake_against_pinned_tag`). Ignored by
// default — dev-plan.md W27 "No real vendor in CI." A local `ollama serve` already
// signed in to an Ollama account exposes an OpenAI-compatible endpoint at
// `/v1/chat/completions` that proxies `*-cloud`-tagged models to Ollama's hosted
// inference; this is a convenient **stand-in vendor** for a real end-to-end run of the
// Cloud AI plugin's HTTP path (architecture §9.1 "OpenAI-compatible base URL"), not a
// claim that Ollama Cloud is v1's supported vendor. Run explicitly with
// `cargo test -p pg-core --test cloud_ai_w27 -- --ignored real_ollama_cloud`.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real Ollama Cloud; manual/informational only, needs a signed-in local daemon"]
fn real_ollama_cloud_smoke_test_end_to_end_share() {
    // `127.0.0.1:11434` is right when this test runs on the same host as `ollama serve`.
    // Inside this repo's dev container the daemon is a sibling container instead, so
    // `PG_OLLAMA_ADDR` (e.g. the compose bridge gateway) overrides it for that case.
    let addr = std::env::var("PG_OLLAMA_ADDR").unwrap_or_else(|_| "127.0.0.1:11434".to_string());
    let mut wired = fresh_confirmed();
    let secret = CloudAiSecret {
        endpoint_url: format!("http://{addr}/v1/chat/completions"),
        model: "gpt-oss:120b-cloud".to_string(),
        api_key: "local-daemon-does-not-require-one".to_string(),
        key_last4: "none".to_string(),
    };
    wired
        .mgr
        .test_only_set_cloud_ai_config(secret)
        .expect("store secret");

    let handshake = wired.mgr.cloud_ai_test().expect("cloud_ai_test");
    assert!(handshake.ok, "handshake: {:?}", handshake.error_class);

    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    let preview = wired
        .mgr
        .preview_share(ai_request(
            vec![doc_id],
            "Reply with exactly the two words: test ok",
        ))
        .expect("preview");
    let commit = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit against real Ollama Cloud");
    let output = commit.output_text.expect("model output");
    assert!(!output.is_empty());
    println!("real Ollama Cloud output: {output:?}");
}

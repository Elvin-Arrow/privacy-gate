//! W15b — `pg-hybrid-ollama-v1` (loopback Ollama NER + W13 patterns).
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §10.1.1 (IP-literal loopback, no DNS, no proxy,
//!   handshake before content), §10.1.2 (allowlist / `-cloud` / digest pin),
//!   §10.1.4 (verify-then-trust offsets; rejection-threshold fallback)
//! - `docs/specs/testing.md` §7.4 (address assertion, no ambient proxy, handshake
//!   gate, offset-verification self-test); §10 (Ollama mock double)
//! - `docs/dev-plan.md` W15b ("Tests first: against the Ollama mock"; "Do not: wire
//!   this as the default backend yet")
//!
//! Seam: [`HybridOllamaV1`] implementing [`Detector`]. SessionManager's default remains
//! [`StubDetector`]. No real Ollama process is required.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pg_core::detector::{
    verify_chunk_entity, AllowlistEntry, Detector, FallbackReason, HybridOllamaV1, OllamaClient,
    StubDetector, GEMMA4_E2B_CONTEXT_TOKENS, HYBRID_OLLAMA_V1_ID, OLLAMA_ALLOWLISTED_TAG,
    OFFSET_REJECT_THRESHOLD,
};
use pg_core::importer;

const DOC_ID: &str = "00000000-0000-4000-8000-000000000016";
const GOLDEN_NI: &str = "QQ123456C";
const GOLDEN_PERSON: &str = "Alice Example";
const FIXTURE_DIGEST: &str = "sha256:fixture-digest-not-a-real-model";

fn allowlist() -> Vec<AllowlistEntry> {
    vec![AllowlistEntry {
        tag: OLLAMA_ALLOWLISTED_TAG,
        digest: FIXTURE_DIGEST,
    }]
}

fn happy_tags(tag: &str, digest: &str) -> String {
    serde_json::json!({
        "models": [{
            "name": tag,
            "model": tag,
            "digest": digest,
            "size": 1
        }]
    })
    .to_string()
}

fn happy_show(digest: &str) -> String {
    serde_json::json!({
        "details": { "format": "gguf", "family": "gemma4" },
        "digest": digest,
        "modified_at": "2026-01-01T00:00:00Z"
    })
    .to_string()
}

fn generate_entities(entities: serde_json::Value) -> String {
    let inner = serde_json::json!({ "entities": entities }).to_string();
    serde_json::json!({
        "model": OLLAMA_ALLOWLISTED_TAG,
        "response": inner,
        "done": true
    })
    .to_string()
}

fn detect_text(hybrid: &HybridOllamaV1, text: &str) -> pg_core::detector::OllamaDetectOutcome {
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");
    hybrid.detect_with_outcome(&doc)
}

struct MockState {
    tags_body: String,
    show_body: String,
    generate_body: String,
    tags_status: u16,
    show_status: u16,
    generate_status: u16,
    redirect_location: Option<String>,
    generate_calls: u32,
    generate_bodies: Vec<String>,
}

struct MockOllama {
    addr: SocketAddr,
    state: Arc<Mutex<MockState>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockOllama {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(Mutex::new(MockState {
            tags_body: happy_tags(OLLAMA_ALLOWLISTED_TAG, FIXTURE_DIGEST),
            show_body: happy_show(FIXTURE_DIGEST),
            generate_body: generate_entities(serde_json::json!([])),
            tags_status: 200,
            show_status: 200,
            generate_status: 200,
            redirect_location: None,
            generate_calls: 0,
            generate_bodies: Vec::new(),
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
        self.state.lock().expect("state").generate_body = generate_entities(entities);
    }

    fn set_tags(&self, body: String) {
        self.state.lock().expect("state").tags_body = body;
    }

    fn set_show(&self, body: String) {
        self.state.lock().expect("state").show_body = body;
    }

    fn set_redirect(&self, location: &str) {
        let mut s = self.state.lock().expect("state");
        s.tags_status = 302;
        s.redirect_location = Some(location.to_string());
    }

    fn generate_calls(&self) -> u32 {
        self.state.lock().expect("state").generate_calls
    }

    fn generate_bodies(&self) -> Vec<String> {
        self.state.lock().expect("state").generate_bodies.clone()
    }

    fn client(&self) -> OllamaClient {
        OllamaClient::connect(self.addr, allowlist()).expect("loopback connect")
    }

    fn hybrid(&self) -> HybridOllamaV1 {
        HybridOllamaV1::new(self.client())
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
        if let Some(header_end) = find_double_crlf(&data) {
            let header = std::str::from_utf8(&data[..header_end]).unwrap_or("");
            let want = content_length(header).unwrap_or(0);
            if data.len() >= header_end + 4 + want {
                break;
            }
        }
    }
    let req = String::from_utf8_lossy(&data);
    let first = req.lines().next().unwrap_or("");
    let body = req.split("\r\n\r\n").nth(1).unwrap_or("");

    let mut st = state.lock().expect("state");
    if first.starts_with("GET /api/tags") {
        if let Some(loc) = st.redirect_location.clone() {
            let _ = write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return;
        }
        reply(&mut stream, st.tags_status, &st.tags_body);
        return;
    }
    if first.starts_with("POST /api/show") {
        reply(&mut stream, st.show_status, &st.show_body);
        return;
    }
    if first.starts_with("POST /api/generate") {
        st.generate_calls += 1;
        st.generate_bodies.push(body.to_string());
        let gen = st.generate_body.clone();
        let status = st.generate_status;
        drop(st);
        reply(&mut stream, status, &gen);
        return;
    }
    reply(&mut stream, 404, "{}");
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

#[test]
fn hybrid_ollama_v1_id_is_the_architecture_identity() {
    let mock = MockOllama::start();
    assert_eq!(HYBRID_OLLAMA_V1_ID, "pg-hybrid-ollama-v1");
    assert_eq!(mock.hybrid().id(), HYBRID_OLLAMA_V1_ID);
}

#[test]
fn session_manager_default_detector_is_still_the_stub() {
    assert_eq!(StubDetector.id(), "pg-detector-stub-v1");
}

#[test]
fn gemma4_e2b_context_window_is_unverified_until_nightly() {
    // architecture §10.1.5: do not ship an unverified figure.
    assert!(GEMMA4_E2B_CONTEXT_TOKENS.is_none());
}

#[test]
fn non_loopback_socket_is_refused() {
    let addr = SocketAddr::from(([8, 8, 8, 8], 11434));
    assert!(OllamaClient::connect(addr, allowlist()).is_err());
}

#[test]
fn redirect_off_loopback_is_not_followed() {
    let mock = MockOllama::start();
    mock.set_redirect("http://203.0.113.1/api/tags");
    let out = detect_text(&mock.hybrid(), GOLDEN_PERSON);
    assert_eq!(
        out.fallback_reason,
        Some(FallbackReason::SchemaVerificationFailed)
    );
    assert_eq!(mock.generate_calls(), 0);
}

#[test]
fn ambient_http_proxy_sees_zero_requests() {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _env = ENV_LOCK.lock().expect("env lock");

    let proxy_hits = Arc::new(AtomicU32::new(0));
    let hits = Arc::clone(&proxy_hits);
    let proxy = TcpListener::bind("127.0.0.1:0").expect("proxy bind");
    proxy.set_nonblocking(true).expect("nonblocking");
    let proxy_addr = proxy.local_addr().expect("proxy addr");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = Arc::clone(&stop);
    let proxy_thread = thread::spawn(move || loop {
        if stop_t.load(Ordering::SeqCst) {
            break;
        }
        match proxy.accept() {
            Ok(_) => {
                hits.fetch_add(1, Ordering::SeqCst);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    });

    std::env::set_var("HTTP_PROXY", format!("http://{proxy_addr}"));
    std::env::set_var("ALL_PROXY", format!("http://{proxy_addr}"));
    std::env::set_var("http_proxy", format!("http://{proxy_addr}"));

    let mock = MockOllama::start();
    let hybrid = mock.hybrid();
    let _ = detect_text(&hybrid, GOLDEN_PERSON);

    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("http_proxy");

    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect_timeout(&proxy_addr, Duration::from_millis(50));
    let _ = proxy_thread.join();

    assert_eq!(proxy_hits.load(Ordering::SeqCst), 0);
    assert!(mock.generate_calls() >= 1);
}

#[test]
fn malformed_tags_is_schema_verification_failed_and_sends_no_generate() {
    let mock = MockOllama::start();
    mock.set_tags(r#"{"not":"ollama"}"#.into());
    let out = detect_text(&mock.hybrid(), GOLDEN_PERSON);
    assert_eq!(
        out.fallback_reason,
        Some(FallbackReason::SchemaVerificationFailed)
    );
    assert_eq!(mock.generate_calls(), 0);
}

#[test]
fn unallowlisted_tag_sends_no_generate() {
    let mock = MockOllama::start();
    mock.set_tags(happy_tags("some-other:tag", FIXTURE_DIGEST));
    let out = detect_text(&mock.hybrid(), GOLDEN_PERSON);
    assert_eq!(out.fallback_reason, Some(FallbackReason::ModelNotAllowlisted));
    assert_eq!(mock.generate_calls(), 0);
}

#[test]
fn cloud_suffixed_tag_is_not_allowlisted_and_sends_no_generate() {
    let mock = MockOllama::start();
    mock.set_tags(happy_tags("gemma4:31b-cloud", FIXTURE_DIGEST));
    let out = detect_text(&mock.hybrid(), GOLDEN_PERSON);
    assert_eq!(out.fallback_reason, Some(FallbackReason::ModelNotAllowlisted));
    assert_eq!(mock.generate_calls(), 0);
}

#[test]
fn digest_mismatch_sends_no_generate() {
    let mock = MockOllama::start();
    mock.set_tags(happy_tags(OLLAMA_ALLOWLISTED_TAG, "sha256:other-digest"));
    mock.set_show(happy_show("sha256:other-digest"));
    let out = detect_text(&mock.hybrid(), GOLDEN_PERSON);
    assert_eq!(out.fallback_reason, Some(FallbackReason::DigestMismatch));
    assert_eq!(mock.generate_calls(), 0);
}

#[test]
fn unreachable_listener_is_ollama_unreachable() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 1));
    let client = OllamaClient::connect(addr, allowlist()).expect("loopback addr is constructible");
    let hybrid = HybridOllamaV1::new(client);
    let out = detect_text(&hybrid, GOLDEN_PERSON);
    assert_eq!(out.fallback_reason, Some(FallbackReason::OllamaUnreachable));
}

#[test]
fn handshake_ok_runs_patterns_and_verified_ner() {
    let mock = MockOllama::start();
    mock.set_generate_entities(serde_json::json!([{
        "start": 0,
        "length": GOLDEN_PERSON.len(),
        "label": "person",
        "text": GOLDEN_PERSON
    }]));
    let text = format!("{GOLDEN_PERSON}. NI {GOLDEN_NI}");
    let out = detect_text(&mock.hybrid(), &text);
    assert_eq!(out.fallback_reason, None);
    assert_eq!(out.model_tag.as_deref(), Some(OLLAMA_ALLOWLISTED_TAG));
    let person: Vec<&str> = out
        .fields
        .iter()
        .filter(|f| f.label == "person")
        .map(|f| f.span.text.as_str())
        .collect();
    let ninos: Vec<&str> = out
        .fields
        .iter()
        .filter(|f| f.label == "uk_nino")
        .map(|f| f.span.text.as_str())
        .collect();
    assert_eq!(person, vec![GOLDEN_PERSON]);
    assert_eq!(ninos, vec![GOLDEN_NI]);
    let bodies = mock.generate_bodies();
    assert!(!bodies.is_empty());
    let body: serde_json::Value = serde_json::from_str(&bodies[0]).expect("generate json");
    assert!(body.get("format").and_then(|f| f.as_object()).is_some());
    assert_ne!(
        body.get("format"),
        Some(&serde_json::Value::String("json".into()))
    );
}

#[test]
fn handshake_failure_still_runs_the_pattern_pack() {
    let mock = MockOllama::start();
    mock.set_tags(r#"{"not":"ollama"}"#.into());
    let text = format!("NI {GOLDEN_NI}");
    let out = detect_text(&mock.hybrid(), &text);
    assert!(out.fallback_reason.is_some());
    let ninos: Vec<&str> = out
        .fields
        .iter()
        .filter(|f| f.label == "uk_nino")
        .map(|f| f.span.text.as_str())
        .collect();
    assert_eq!(ninos, vec![GOLDEN_NI]);
    assert!(out.fields.iter().all(|f| f.classification != "ner"));
}

#[test]
fn mismatched_offset_entity_is_rejected_not_searched() {
    assert!(!verify_chunk_entity(
        "Alice Example lives here",
        6,
        GOLDEN_PERSON.len() as u32,
        GOLDEN_PERSON
    ));
    let mock = MockOllama::start();
    mock.set_generate_entities(serde_json::json!([{
        "start": 6,
        "length": GOLDEN_PERSON.len(),
        "label": "person",
        "text": GOLDEN_PERSON
    }]));
    let out = detect_text(&mock.hybrid(), GOLDEN_PERSON);
    assert!(out.fields.iter().all(|f| f.label != "person"));
    assert_eq!(
        out.fallback_reason,
        Some(FallbackReason::OffsetVerificationFailed)
    );
}

#[test]
fn rejection_rate_over_threshold_fails_the_whole_ollama_pass() {
    assert!(2.0 / 3.0 > OFFSET_REJECT_THRESHOLD);
    let mock = MockOllama::start();
    mock.set_generate_entities(serde_json::json!([
        {"start": 0, "length": 5, "label": "person", "text": "XXXXX"},
        {"start": 1, "length": 5, "label": "person", "text": "XXXXX"},
        {"start": 0, "length": GOLDEN_PERSON.len(), "label": "person", "text": GOLDEN_PERSON}
    ]));
    let text = format!("{GOLDEN_PERSON}. NI {GOLDEN_NI}");
    let out = detect_text(&mock.hybrid(), &text);
    assert_eq!(
        out.fallback_reason,
        Some(FallbackReason::OffsetVerificationFailed)
    );
    assert!(out.fields.iter().all(|f| f.classification != "ner"));
    let ninos: Vec<&str> = out
        .fields
        .iter()
        .filter(|f| f.label == "uk_nino")
        .map(|f| f.span.text.as_str())
        .collect();
    assert_eq!(ninos, vec![GOLDEN_NI]);
}

/// Informational: real-Ollama nightly (testing.md §11). Ignored unless the runner
/// actually has Ollama; the workflow skips the job when `ollama` is missing.
#[test]
#[ignore = "real Ollama; nightly only when the runner has the daemon"]
fn nightly_real_ollama_handshake_against_pinned_tag() {
    let digest = pg_core::detector::OLLAMA_GEMMA4_E2B_DIGEST
        .expect("record gemma4:e2b digest before enabling the real-Ollama nightly");
    let addr = SocketAddr::from(([127, 0, 0, 1], 11434));
    let client = OllamaClient::connect(
        addr,
        vec![AllowlistEntry {
            tag: OLLAMA_ALLOWLISTED_TAG,
            digest,
        }],
    )
    .expect("loopback");
    client.handshake().expect("handshake against local Ollama");
}

//! W37 — Acceptance pack AC-1..AC-7 (testing.md §6).
//!
//! Spec sources:
//! - `docs/specs/testing.md` §6.1–§6.7 (command-level scenarios; stub detector; OQ-6
//!   oracle for AC-2/AC-3/AC-4 share; C-TEST-3 no real Cloud AI host)
//! - `docs/dev-plan.md` W37 ("Tests first: already written; this chunk is the gate that
//!   they all run in CI as one job." "Integrate: `cargo test` acceptance binary/module."
//!   "Done when: AC-1..AC-7 listed in CI logs." "Do not: hit a real Cloud AI host.")
//!
//! Seam: in-process [`SessionManager`] commands — the same functions Tauri IPC calls
//! (testing.md §2). Earlier chunks hold partial coverage; this binary is the named pack
//! CI prints, and it fills the remaining command-level gaps (AC-2 multi-doc order,
//! AC-5 stolen `vault.db` after import+approve, AC-6 retain-default + discard override).

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as LoDocument, Object, Stream};

use pg_core::account::AccountStore;
use pg_core::api::ErrorCode;
use pg_core::audit::{AuditStore, EventType};
use pg_core::catalog::{DocumentStore, EffectiveRetention};
use pg_core::cloud_ai::CloudAiSecret;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::importer::SourceFormat;
use pg_core::keys::unwrap_master_key;
use pg_core::keystore::{FileKeystore, InMemoryKeystore, KeystoreBackend, FALLBACK_FILE_NAME};
use pg_core::session::{
    CloudAiSetConfigIn, CommitShareIn, CreateAccountIn, FieldDecisionDto, FieldDecisionKind,
    GetDocumentIn, ImportDocumentIn, ListAuditEventsIn, OpenApprovalIn, PreviewShareIn,
    SessionManager, SessionState, SetFieldDecisionsIn, SetRetentionDefaultIn, ShareKind,
    ShareRequestDto, SubmitApprovalIn, UnlockIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend, VaultError};

const PASSPHRASE: &str = "correct horse battery staple";
const WRONG_PASSPHRASE: &str = "incorrect horse battery staple";
const LETTER: &[u8] = b"Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.";
const BODY_A: &[u8] = b"DocA PG-CANARY-X1 PG-CANARY-KEEP-A";
const BODY_B: &[u8] = b"DocB PG-CANARY-X1 PG-CANARY-KEEP-B";

struct Wired {
    mgr: SessionManager,
    _dir: tempfile::TempDir,
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

fn wired(keystore: Arc<dyn KeystoreBackend>, dir: tempfile::TempDir, vault_path: &Path) -> Wired {
    let vault = Arc::new(SqlCipherVault::new(vault_path.to_path_buf()));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let plugin_secrets: Arc<dyn pg_core::cloud_ai::PluginSecretStore> = vault.clone();
    let mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents)
        .with_plugin_secrets(plugin_secrets)
        .with_detector(Arc::new(StubDetector));
    Wired {
        mgr,
        _dir: dir,
    }
}

fn fresh_unconfirmed() -> Wired {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault_path = dir.path().join("vault.db");
    let mut out = wired(Arc::new(InMemoryKeystore::new()), dir, &vault_path);
    out.mgr.create_account(create_in()).expect("create_account");
    out
}

fn fresh_confirmed() -> Wired {
    let mut out = fresh_unconfirmed();
    out.mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Discard,
        })
        .expect("confirm retention");
    out
}

fn export_request(doc_ids: Vec<String>, overrides: HashMap<String, Vec<FieldDecisionDto>>) -> PreviewShareIn {
    PreviewShareIn {
        request: ShareRequestDto {
            kind: ShareKind::ExportToPerson,
            doc_ids,
            per_doc_overrides: overrides,
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
            recipient_note: None,
            ai_instruction: instruction.map(str::to_string),
        },
    }
}

struct Approved {
    doc_id: String,
    redact_id: String,
}

fn import_and_approve(mgr: &mut SessionManager, filename: &str, bytes: &[u8]) -> Approved {
    let doc_id = mgr
        .import_document(ImportDocumentIn {
            filename: filename.to_string(),
            bytes: bytes.to_vec(),
            retention_override: None,
        })
        .expect("import_document")
        .summary
        .doc_id;
    let view = mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .expect("open_approval");
    assert!(
        view.fields.iter().all(|f| f.span.byte_length > 0),
        "AC-1: detected spans must be locatable"
    );
    let mut redact_id = String::new();
    let mut saw_keep = false;
    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| {
            let text = f.span.text.as_deref().expect("C-API-2: approval view has span text");
            let redact = text.contains("PG-CANARY-X1");
            if redact {
                redact_id = f.id.clone();
            } else {
                saw_keep = true;
            }
            FieldDecisionDto {
                field_id: f.id.clone(),
                decision: if redact {
                    FieldDecisionKind::Redact
                } else {
                    FieldDecisionKind::KeepVisible
                },
            }
        })
        .collect();
    assert!(
        !redact_id.is_empty() && saw_keep,
        "stub must locate a redact canary and a keep canary"
    );
    let decided = mgr
        .set_field_decisions(SetFieldDecisionsIn {
            approval_session_id: view.approval_session_id.clone(),
            decisions,
        })
        .expect("set_field_decisions");
    assert_eq!(decided.lifecycle, pg_core::session::ApprovalLifecycle::Decided);
    mgr.submit_approval(SubmitApprovalIn {
        approval_session_id: view.approval_session_id,
    })
    .expect("submit_approval");
    Approved {
        doc_id,
        redact_id,
    }
}

fn build_text_pdf(text: &str) -> Vec<u8> {
    let mut doc = LoDocument::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("lopdf fixture");
    bytes
}

fn dump_has_canary(value: &impl serde::Serialize) -> bool {
    serde_json::to_string(value)
        .expect("json")
        .contains("PG-CANARY-")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("stolen dir");
    for entry in std::fs::read_dir(from).expect("read origin") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("ft").is_file() {
            std::fs::copy(entry.path(), dest).expect("copy stolen file");
        }
    }
}

// ---------------------------------------------------------------------------
// testing.md §6.1 AC-1 — Import, detect, approve, store
// ---------------------------------------------------------------------------

#[test]
fn ac1_import_detect_approve_store() {
    eprintln!("AC-1");
    let mut wired = fresh_confirmed();
    let pdf = build_text_pdf("Dear Sir, PG-CANARY-X1 and PG-CANARY-X2 both appear here.");
    let approved = import_and_approve(&mut wired.mgr, "letter.pdf", &pdf);

    let listed = wired.mgr.list_documents().expect("list_documents");
    let row = listed
        .documents
        .iter()
        .find(|d| d.doc_id == approved.doc_id)
        .expect("catalog row");
    assert!(row.has_approved_version);
    assert_eq!(row.source_format, SourceFormat::Pdf);
    assert!(!dump_has_canary(&listed), "AC-1: original bytes never on list_documents");

    let err = wired
        .mgr
        .open_approval(OpenApprovalIn {
            doc_id: approved.doc_id.clone(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AlreadyApproved);

    wired.mgr.lock().expect("lock");
    wired
        .mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    let got = wired
        .mgr
        .get_document(GetDocumentIn {
            doc_id: approved.doc_id.clone(),
        })
        .expect("get_document after lock/unlock");
    assert!(got.summary.has_approved_version);
    assert_eq!(got.summary.source_filename, "letter.pdf");
    assert!(
        !dump_has_canary(&got.summary),
        "AC-1: original bytes never on get_document after re-unlock"
    );
}

// ---------------------------------------------------------------------------
// testing.md §6.2 AC-2 — Export with ephemeral override + preview
// ---------------------------------------------------------------------------

#[test]
fn ac2_export_with_ephemeral_override_and_multi_doc() {
    eprintln!("AC-2");
    let mut wired = fresh_confirmed();
    let a = import_and_approve(&mut wired.mgr, "alpha.txt", BODY_A);
    let b = import_and_approve(&mut wired.mgr, "beta.txt", BODY_B);

    let mut overrides = HashMap::new();
    overrides.insert(
        a.doc_id.clone(),
        vec![FieldDecisionDto {
            field_id: a.redact_id.clone(),
            decision: FieldDecisionKind::KeepVisible,
        }],
    );
    let preview = wired
        .mgr
        .preview_share(export_request(vec![a.doc_id.clone()], overrides))
        .expect("overridden preview");
    assert!(preview.overrides_in_effect);
    let pdf = preview.pdf_bytes.expect("export pdf");
    common::oracle::check(&pdf, &[], &["PG-CANARY-X1", "PG-CANARY-KEEP-A"])
        .expect("C-TEST-7: override reveal still has no extra redacted leak; both kept strings present");

    let commit = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit");
    assert_eq!(commit.pdf_bytes.as_deref(), Some(pdf.as_slice()));

    // Multi-doc: user selection order is [B, A], not import order.
    let bundle = wired
        .mgr
        .preview_share(export_request(
            vec![b.doc_id.clone(), a.doc_id.clone()],
            HashMap::new(),
        ))
        .expect("bundle preview");
    assert_eq!(
        bundle.manifest.iter().map(|m| m.doc_id.as_str()).collect::<Vec<_>>(),
        vec![b.doc_id.as_str(), a.doc_id.as_str()]
    );
    let bundle_pdf = bundle.pdf_bytes.expect("bundle pdf");
    common::oracle::check(
        &bundle_pdf,
        &["PG-CANARY-X1"],
        &["PG-CANARY-KEEP-B", "PG-CANARY-KEEP-A"],
    )
    .expect("C-TEST-7: bundle omits redacted canary");
    let extracted = pdf_extract::extract_text_from_mem(&bundle_pdf).expect("extract");
    let pos_b = extracted.find("PG-CANARY-KEEP-B").expect("keep B in bundle");
    let pos_a = extracted.find("PG-CANARY-KEEP-A").expect("keep A in bundle");
    assert!(
        pos_b < pos_a,
        "design §3.7: selection order B then A must be preserved in the bundle"
    );
    let name = bundle.suggested_filename.expect("filename");
    assert!(name.starts_with("privacy-gate-2docs-redacted-"));
    assert!(name.ends_with(".pdf"));
    let as_str = String::from_utf8_lossy(&bundle_pdf);
    assert!(!as_str.contains("/Author"));
    assert!(!as_str.contains("/Subject"));
    assert!(!as_str.contains("/Keywords"));
    assert!(!as_str.contains("PG-CANARY-X1"));
}

// ---------------------------------------------------------------------------
// testing.md §6.3 AC-3 — Cloud AI, approved content only (mock host)
// ---------------------------------------------------------------------------

struct MockState {
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

impl MockCloudAi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let state = Arc::new(Mutex::new(MockState {
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

    fn calls(&self) -> u32 {
        self.state.lock().expect("state").calls
    }

    fn bodies(&self) -> Vec<String> {
        self.state.lock().expect("state").bodies.clone()
    }

    fn connections(&self) -> u32 {
        self.connections.load(Ordering::SeqCst)
    }

    fn secret(&self) -> CloudAiSecret {
        CloudAiSecret {
            endpoint_url: self.url(),
            model: "gpt-x".to_string(),
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
        if let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
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
    drop(st);
    let resp = serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": "ok" } }]
    })
    .to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}",
        resp.len()
    );
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

#[test]
fn ac3_cloud_ai_approved_content_only() {
    eprintln!("AC-3");
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    let approved = import_and_approve(&mut wired.mgr, "letter.txt", LETTER);

    let err = wired
        .mgr
        .preview_share(ai_request(vec![approved.doc_id.clone()], Some("Summarize this.")))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CloudAiNotConfigured);
    assert_eq!(mock.connections(), 0, "C-API-4: no HTTP before configured");

    wired
        .mgr
        .cloud_ai_set_config(CloudAiSetConfigIn {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-super-secret-value".to_string(),
        })
        .expect("set_config");
    let got = wired.mgr.cloud_ai_get_config().expect("get_config");
    assert!(got.configured);
    assert_eq!(got.key_last4.as_deref(), Some("alue"));
    let serialized = serde_json::to_string(&got).expect("serialize");
    assert!(!serialized.contains("sk-super-secret-value"));
    assert!(!serialized.contains("api_key"));

    // HTTP send is proven against a loopback mock (testing.md TLS-mock gap, same as W27).
    wired
        .mgr
        .test_only_set_cloud_ai_secret(mock.secret())
        .expect("point Cloud AI at the mock");

    let probe = wired.mgr.cloud_ai_test().expect("cloud_ai_test");
    assert!(probe.ok);
    assert_eq!(mock.calls(), 1);
    assert!(
        !mock.bodies()[0].contains("PG-CANARY"),
        "C-API-4: cloud_ai_test must not send vault documents"
    );

    let preview = wired
        .mgr
        .preview_share(ai_request(
            vec![approved.doc_id.clone()],
            Some("Summarize this letter."),
        ))
        .expect("preview");
    let payload = preview.ai_payload_preview.clone().expect("ai payload");
    assert!(!payload.contains("PG-CANARY-X1"));
    assert!(payload.contains("PG-CANARY-X2"));

    let commit = wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit");
    assert_eq!(commit.output_text.as_deref(), Some("ok"));
    let sent = mock.bodies().last().cloned().expect("POST body");
    assert!(sent.contains(&payload), "commit POSTs the previewed body");
    common::oracle::check(sent.as_bytes(), &["PG-CANARY-X1"], &[])
        .expect("C-TEST-7: AI wire body has no redacted canary");
}

// ---------------------------------------------------------------------------
// testing.md §6.4 AC-4 — Audit trail answers "what did I share?"
// ---------------------------------------------------------------------------

#[test]
fn ac4_audit_trail_answers_what_did_i_share() {
    eprintln!("AC-4");
    let mock = MockCloudAi::start();
    let mut wired = fresh_confirmed();
    let approved = import_and_approve(&mut wired.mgr, "letter.txt", LETTER);

    let export = wired
        .mgr
        .preview_share(export_request(vec![approved.doc_id.clone()], HashMap::new()))
        .expect("export preview");
    let pdf = export.pdf_bytes.expect("pdf");
    common::oracle::check(&pdf, &["PG-CANARY-X1"], &["PG-CANARY-X2"])
        .expect("C-TEST-7: independent of no_originals_left_device");
    wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: export.preview_token,
        })
        .expect("export commit");

    wired
        .mgr
        .test_only_set_cloud_ai_secret(mock.secret())
        .expect("mock ai");
    let ai = wired
        .mgr
        .preview_share(ai_request(vec![approved.doc_id.clone()], Some("Summarize.")))
        .expect("ai preview");
    wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: ai.preview_token,
        })
        .expect("ai commit");

    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(approved.doc_id.clone()),
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
            EventType::Share,
        ]
    );
    let payloads = out
        .events
        .iter()
        .map(|e| e.payload.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!payloads.contains("PG-CANARY"));
    assert!(!payloads.contains("Summarize"));
    for e in &out.events {
        match e.event_type {
            EventType::Share => {
                assert!(e.no_originals_left_device.is_some());
                assert!(e.payload.get("entry_signature").is_none());
                assert!(e.payload.get("prev_entry_hash").is_none());
            }
            _ => assert!(e.no_originals_left_device.is_none()),
        }
    }
    let kinds: Vec<&str> = out
        .events
        .iter()
        .filter(|e| e.event_type == EventType::Share)
        .map(|e| e.payload["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, vec!["export_to_person", "share_to_ai"]);
}

// ---------------------------------------------------------------------------
// testing.md §6.5 AC-5 — Stolen data file, vault locked
// ---------------------------------------------------------------------------

#[test]
fn ac5_stolen_data_file_vault_locked() {
    eprintln!("AC-5");
    let origin = tempfile::tempdir().expect("origin");
    let vault_path = origin.path().join("vault.db");
    let keystore = Arc::new(FileKeystore::in_dir(origin.path()));
    let mut wired = wired(keystore, origin, &vault_path);
    wired.mgr.create_account(create_in()).expect("create_account");
    wired
        .mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Retain,
        })
        .expect("retain so the original exists inside the ciphertext");
    let approved = import_and_approve(&mut wired.mgr, "letter.txt", LETTER);
    assert!(
        wired
            .mgr
            .get_document(GetDocumentIn {
                doc_id: approved.doc_id.clone(),
            })
            .expect("get")
            .summary
            .has_retained_original
    );
    wired.mgr.lock().expect("lock");

    let stolen_dir = tempfile::tempdir().expect("stolen copy");
    copy_tree(wired._dir.path(), stolen_dir.path());
    let stolen_db = std::fs::read(stolen_dir.path().join("vault.db")).expect("read stolen db");
    let stolen_ks = std::fs::read(stolen_dir.path().join(FALLBACK_FILE_NAME)).expect("read stolen keystore");
    let db_utf = String::from_utf8_lossy(&stolen_db);
    let ks_utf = String::from_utf8_lossy(&stolen_ks);
    assert!(
        !db_utf.contains("PG-CANARY") && !ks_utf.contains("PG-CANARY"),
        "stolen files must not contain catalog/document plaintext"
    );
    assert!(!db_utf.contains(PASSPHRASE) && !ks_utf.contains(PASSPHRASE));

    let stolen_vault = SqlCipherVault::new(stolen_dir.path().join("vault.db"));
    let open = stolen_vault.open(&zeroize::Zeroizing::new([0x22u8; 32]));
    assert_eq!(open, Err(VaultError::WrongKey));

    let item = FileKeystore::in_dir(stolen_dir.path())
        .load()
        .expect("load stolen keystore")
        .expect("item present");
    assert!(
        unwrap_master_key(WRONG_PASSPHRASE, &item).is_none(),
        "architecture §3.2: wrap still holds without the passphrase"
    );
    assert!(unwrap_master_key(PASSPHRASE, &item).is_some());

    let stolen_ks_backend = Arc::new(FileKeystore::in_dir(stolen_dir.path()));
    let stolen_sql = Arc::new(SqlCipherVault::new(stolen_dir.path().join("vault.db")));
    let accounts: Arc<dyn AccountStore> = stolen_sql.clone();
    let backend: Arc<dyn VaultBackend> = stolen_sql.clone();
    let audit: Arc<dyn AuditStore> = stolen_sql.clone();
    let config: Arc<dyn ConfigStore> = stolen_sql.clone();
    let documents: Arc<dyn DocumentStore> = stolen_sql;
    let mut attacker = SessionManager::new_full(stolen_ks_backend, accounts, backend, audit, config)
        .with_documents(documents)
        .with_detector(Arc::new(StubDetector));
    assert_eq!(attacker.get_session_state().state, SessionState::Locked);
    let err = attacker
        .unlock(UnlockIn {
            passphrase: WRONG_PASSPHRASE.to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnlockFailed);
    assert!(attacker.list_documents().is_err());
}

// ---------------------------------------------------------------------------
// testing.md §6.6 AC-6 — Paranoid retention default
// ---------------------------------------------------------------------------

#[test]
fn ac6_paranoid_retention_default() {
    eprintln!("AC-6");
    let mut wired = fresh_unconfirmed();
    wired
        .mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::NeverRetain,
        })
        .expect("confirm never_retain");

    let err = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "nope.txt".to_string(),
            bytes: b"hello world".to_vec(),
            retention_override: Some(EffectiveRetention::Retain),
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RetentionLoosenForbidden);
    assert!(wired.mgr.list_documents().expect("list").documents.is_empty());

    let tightened = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "ok.txt".to_string(),
            bytes: b"hello world".to_vec(),
            retention_override: Some(EffectiveRetention::Discard),
        })
        .expect("per-import discard is allowed (tighten)");
    assert_eq!(tightened.summary.retention, EffectiveRetention::Discard);

    wired
        .mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Retain,
        })
        .expect("global retain");
    let discarded = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "drop.txt".to_string(),
            bytes: b"hello world".to_vec(),
            retention_override: Some(EffectiveRetention::Discard),
        })
        .expect("per-import discard against retain default is allowed");
    assert_eq!(discarded.summary.retention, EffectiveRetention::Discard);
}

// ---------------------------------------------------------------------------
// testing.md §6.7 AC-7 — Factory discard and first-import confirmation
// ---------------------------------------------------------------------------

#[test]
fn ac7_factory_discard_and_first_import_confirmation() {
    eprintln!("AC-7");
    let mut wired = fresh_unconfirmed();
    let factory = wired.mgr.get_retention_default().expect("get");
    assert_eq!(factory.policy, RetentionPolicy::Discard);
    assert!(!factory.confirmed);

    let err = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: b"hello world".to_vec(),
            retention_override: None,
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RetentionPolicyUnset);
    assert!(wired.mgr.list_documents().expect("list").documents.is_empty());

    let err_override = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: b"hello world".to_vec(),
            retention_override: Some(EffectiveRetention::Retain),
        })
        .unwrap_err();
    assert_eq!(err_override.code, ErrorCode::RetentionPolicyUnset);

    let confirmed = wired
        .mgr
        .set_retention_default(SetRetentionDefaultIn {
            policy: RetentionPolicy::Discard,
        })
        .expect("confirm");
    assert!(confirmed.confirmed);

    let first = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: b"hello world".to_vec(),
            retention_override: Some(EffectiveRetention::Retain),
        })
        .expect("confirming discard then overriding first import to retain is allowed");
    assert_eq!(first.summary.retention, EffectiveRetention::Retain);
    assert!(first.summary.has_retained_original);
}

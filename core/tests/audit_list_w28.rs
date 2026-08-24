//! W28 — `list_audit_events` (AC-4).
//!
//! Spec sources:
//! - `docs/specs/api.md` §5.8 (`list_audit_events` In/Out, `AuditEventDto`, per-event-type
//!   payload shapes, "webview does not verify the chain", degraded → verified prefix only)
//! - `docs/specs/data-model.md` §5.8.1 (`EventPayload` keys per `event_type`)
//! - `docs/specs/testing.md` §6.4 AC-4 ("what did I share?"; no span text/originals/keys;
//!   degraded session returns only the verified prefix)
//! - `docs/dev-plan.md` W28 ("Tests first: AC-4 'what did I share?'; C-API-1/2 on DTOs")
//!
//! Seam: [`SessionManager::list_audit_events`]. Read path only — every payload asserted
//! here was already written by an earlier chunk's command (W10/W12/W18/W20/W21/W24/W27);
//! this chunk does not change what gets written, only what can be read back.

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::{self, AuditStore, EventType, OriginalsFlag};
use pg_core::catalog::DocumentStore;
use pg_core::cloud_ai::CloudAiStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    command_allowed, CommitShareIn, CreateAccountIn, FieldDecisionDto, FieldDecisionKind,
    ImportDocumentIn, ListAuditEventsIn, OpenApprovalIn, PreviewShareIn, SessionManager,
    SessionState, SetFieldDecisionsIn, SetRetentionDefaultIn, ShareKind, ShareRequestDto,
    SubmitApprovalIn, UnlockIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";
const KEEP_CANARY: &str = "PG-CANARY-AUDIT-KEEP1";
const REDACT_CANARY: &str = "PG-CANARY-AUDIT-REDACT-8K3M";
const BODY: &[u8] = b"Dear Sir, we cite PG-CANARY-AUDIT-KEEP1 and PG-CANARY-AUDIT-REDACT-8K3M here.";

struct Wired {
    mgr: SessionManager,
    vault: Arc<SqlCipherVault>,
    keystore: Arc<InMemoryKeystore>,
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
    let keystore = Arc::new(InMemoryKeystore::new());
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit_store: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let cloud_ai: Arc<dyn CloudAiStore> = vault.clone();
    let mut mgr = SessionManager::new_full(keystore.clone(), accounts, backend, audit_store, config)
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
        keystore,
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

fn export_share(mgr: &mut SessionManager, doc_id: &str) {
    let preview = mgr
        .preview_share(PreviewShareIn {
            request: ShareRequestDto {
                kind: ShareKind::ExportToPerson,
                doc_ids: vec![doc_id.to_string()],
                per_doc_overrides: std::collections::HashMap::new(),
                applied_variant_ids: std::collections::HashMap::new(),
                recipient_note: Some("caseworker".to_string()),
                ai_instruction: None,
            },
        })
        .expect("preview export");
    mgr.commit_share(CommitShareIn {
        preview_token: preview.preview_token,
    })
    .expect("commit export");
}

fn list_all(mgr: &SessionManager, doc_id: Option<&str>) -> Vec<pg_core::session::AuditEventDto> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = mgr
            .list_audit_events(ListAuditEventsIn {
                doc_id: doc_id.map(str::to_string),
                event_type: None,
                after_sequence: after,
                limit: 200,
            })
            .expect("list_audit_events");
        let got_next = page.next_sequence;
        out.extend(page.events);
        match got_next {
            Some(n) => after = Some(n),
            None => break,
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Session gating (api.md §2)
// ---------------------------------------------------------------------------

#[test]
fn list_audit_events_unlocked_or_degraded_only() {
    assert!(!command_allowed("list_audit_events", SessionState::FirstRun));
    assert!(!command_allowed("list_audit_events", SessionState::Locked));
    assert!(command_allowed("list_audit_events", SessionState::Unlocked));
    assert!(command_allowed(
        "list_audit_events",
        SessionState::DegradedIntegrity
    ));
}

#[test]
fn refused_before_unlock() {
    let wired = fresh_confirmed();
    // A brand new manager over the same backends, still locked.
    let keystore = wired.keystore.clone();
    let vault = wired.vault.clone();
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit_store: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let locked = SessionManager::new_full(keystore, accounts, backend, audit_store, config);
    let err = locked
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: 50,
        })
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::NotInSession);
}

#[test]
fn limit_zero_or_over_200_is_invalid_input() {
    let wired = fresh_confirmed();
    for limit in [0u32, 201] {
        let err = wired
            .mgr
            .list_audit_events(ListAuditEventsIn {
                doc_id: None,
                event_type: None,
                after_sequence: None,
                limit,
            })
            .unwrap_err();
        assert_eq!(err.code, pg_core::api::ErrorCode::InvalidInput, "limit {limit}");
    }
    // 1 and 200 are the accepted boundary.
    for limit in [1u32, 200] {
        wired
            .mgr
            .list_audit_events(ListAuditEventsIn {
                doc_id: None,
                event_type: None,
                after_sequence: None,
                limit,
            })
            .unwrap_or_else(|e| panic!("limit {limit} should be accepted: {e:?}"));
    }
}

// ---------------------------------------------------------------------------
// AC-4 — "what did I share?" (testing.md §6.4)
// ---------------------------------------------------------------------------

#[test]
fn ac4_full_flow_shows_import_detect_approve_and_two_share_kinds() {
    let mut wired = fresh_confirmed();
    wired
        .mgr
        .test_only_set_cloud_ai_config(pg_core::cloud_ai::CloudAiSecret {
            // Port 1 on loopback: nothing listens there, so the TCP connect itself is
            // refused immediately, before any TLS negotiation would even start (same
            // trick `cloud_ai_w27.rs`'s `MockCloudAi::dead_addr` uses) — deterministic and
            // fast, unlike relying on DNS failure for a reserved TLD. `https://` (not
            // `http://`) so `validate_endpoint_url` still resolves a real `endpoint_host`
            // for the audit row, matching what a real `cloud_ai_set_config` call requires.
            endpoint_url: "https://127.0.0.1:1/v1/chat".to_string(),
            model: "gpt-test".to_string(),
            api_key: "sk-test".to_string(),
            key_last4: "test".to_string(),
        })
        .expect("store secret");
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    export_share(&mut wired.mgr, &doc_id);

    // AI share: this endpoint is unreachable, so the send fails — but dev-plan W27 says a
    // failed attempt still audits, and that's exactly the row AC-4 needs here too (a
    // "what did I try to share" row, not only successes).
    let ai_preview = wired
        .mgr
        .preview_share(PreviewShareIn {
            request: ShareRequestDto {
                kind: ShareKind::ShareToAi,
                doc_ids: vec![doc_id.clone()],
                per_doc_overrides: std::collections::HashMap::new(),
                applied_variant_ids: std::collections::HashMap::new(),
                recipient_note: None,
                ai_instruction: Some("Summarise this letter.".to_string()),
            },
        })
        .expect("preview ai");
    let _ = wired.mgr.commit_share(CommitShareIn {
        preview_token: ai_preview.preview_token,
    });
    // (intentionally ignoring the network error — the point is the audited attempt)

    let events = list_all(&wired.mgr, Some(&doc_id));
    let types: Vec<EventType> = events.iter().map(|e| e.event_type).collect();
    assert!(types.contains(&EventType::Import), "{types:?}");
    assert!(types.contains(&EventType::Detect), "{types:?}");
    assert!(types.contains(&EventType::Approve), "{types:?}");
    assert_eq!(
        types.iter().filter(|t| **t == EventType::Share).count(),
        2,
        "one export share + one (failed) AI share: {types:?}"
    );

    // Every event names the doc_id we filtered on.
    assert!(events.iter().all(|e| e.doc_id.as_deref() == Some(doc_id.as_str())));

    // `no_originals_left_device` only on share events.
    for e in &events {
        match e.event_type {
            EventType::Share => assert!(e.no_originals_left_device.is_some(), "{e:?}"),
            _ => assert!(e.no_originals_left_device.is_none(), "{e:?}"),
        }
    }

    // C-API-1/2: no span text, no originals, no keys, no ai_instruction text — anywhere in
    // any payload, over the whole flow (including detect's field labels/ids and approve's
    // decisions, which api.md §5.8.1 says are id/label/decision only).
    let dump = serde_json::to_string(&events).expect("serialize dump");
    assert!(!dump.contains(KEEP_CANARY), "{dump}");
    assert!(!dump.contains(REDACT_CANARY), "{dump}");
    assert!(!dump.contains("Summarise this letter"), "{dump}");
    assert!(!dump.contains("sk-test"), "{dump}");

    // Approve payload shape: field_id/label/decision only (api.md §5.8.1).
    let approve = events
        .iter()
        .find(|e| e.event_type == EventType::Approve)
        .expect("approve event");
    let decisions = approve.payload["decisions"].as_array().expect("decisions array");
    assert!(!decisions.is_empty());
    for d in decisions {
        assert!(d.get("field_id").is_some());
        assert!(d.get("label").is_some());
        assert!(d.get("decision").is_some());
        assert_eq!(d.as_object().expect("object").len(), 3, "no extra keys: {d}");
    }

    // Share payload shapes: export has endpoint_host null / no error; AI has an
    // endpoint_host and a network error_class, and has_ai_instruction true with no
    // instruction text (already asserted above via the whole-dump scan).
    let shares: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == EventType::Share)
        .collect();
    let export = shares
        .iter()
        .find(|e| e.payload["kind"] == "export_to_person")
        .expect("export share row");
    assert_eq!(export.payload["endpoint_host"], serde_json::Value::Null);
    assert_eq!(export.payload["error_class"], serde_json::Value::Null);
    assert_eq!(export.payload["has_ai_instruction"], false);
    assert_eq!(export.no_originals_left_device, Some(true));

    let ai = shares
        .iter()
        .find(|e| e.payload["kind"] == "share_to_ai")
        .expect("ai share row");
    assert_eq!(ai.payload["has_ai_instruction"], true);
    assert!(ai.payload["endpoint_host"].is_string());
    assert!(ai.payload["error_class"].is_string());
}

// ---------------------------------------------------------------------------
// Filtering and pagination
// ---------------------------------------------------------------------------

#[test]
fn filters_by_event_type() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    export_share(&mut wired.mgr, &doc_id);

    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: Some(EventType::Share),
            after_sequence: None,
            limit: 200,
        })
        .expect("list");
    assert!(!out.events.is_empty());
    assert!(out.events.iter().all(|e| e.event_type == EventType::Share));
}

#[test]
fn unknown_doc_id_filter_is_empty_not_an_error() {
    let mut wired = fresh_confirmed();
    import_and_approve(&mut wired.mgr, "letter.txt");
    let out = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some("00000000-0000-4000-8000-000000000099".to_string()),
            event_type: None,
            after_sequence: None,
            limit: 200,
        })
        .expect("list");
    assert!(out.events.is_empty());
    assert!(out.next_sequence.is_none());
}

#[test]
fn pagination_after_sequence_and_next_sequence() {
    let mut wired = fresh_confirmed();
    let doc_id = import_and_approve(&mut wired.mgr, "letter.txt");
    export_share(&mut wired.mgr, &doc_id);
    // import + detect + approve + share = 4 rows.
    let all = list_all(&wired.mgr, Some(&doc_id));
    assert_eq!(all.len(), 4);

    let first_page = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(doc_id.clone()),
            event_type: None,
            after_sequence: None,
            limit: 2,
        })
        .expect("page 1");
    assert_eq!(first_page.events.len(), 2);
    assert_eq!(first_page.events[0].sequence, all[0].sequence);
    assert_eq!(first_page.events[1].sequence, all[1].sequence);
    let cursor = first_page.next_sequence.expect("more rows remain");
    assert_eq!(cursor, all[1].sequence);

    let second_page = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: Some(doc_id.clone()),
            event_type: None,
            after_sequence: Some(cursor),
            limit: 2,
        })
        .expect("page 2");
    assert_eq!(second_page.events.len(), 2);
    assert_eq!(second_page.events[0].sequence, all[2].sequence);
    assert_eq!(second_page.events[1].sequence, all[3].sequence);
    assert!(
        second_page.next_sequence.is_none(),
        "exactly 4 rows total, page 2 exhausts them"
    );
}

// ---------------------------------------------------------------------------
// Degraded session: verified prefix only (api.md §5.8, testing.md §6.4)
// ---------------------------------------------------------------------------

fn mac_key(mgr: &SessionManager) -> [u8; 32] {
    *mgr.audit_mac_key().expect("session must be unlocked")
}

fn persist_head_for(keystore: &Arc<InMemoryKeystore>, row: &audit::AuditRow) {
    use pg_core::keystore::KeystoreBackend;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(audit::canonical_bytes(row));
    let head_hash: [u8; 32] = hasher.finalize().into();
    let mut item = keystore.load().expect("load").expect("item exists");
    item.audit_head = pg_core::keystore::AuditHead {
        sequence: row.sequence,
        head_hash,
    };
    keystore.store(&item).expect("store updated head");
}

#[test]
fn degraded_session_returns_only_the_verified_prefix() {
    let mut wired = fresh_confirmed();
    let key = mac_key(&wired.mgr);

    // Three clean rows, head persisted at the third.
    let row1 = audit::append(
        wired.vault.as_ref(),
        &key,
        EventType::Import,
        Some("doc-a"),
        1_000,
        OriginalsFlag::Unset,
        r#"{"marker":"row1"}"#,
    )
    .expect("append 1");
    let _row2 = audit::append(
        wired.vault.as_ref(),
        &key,
        EventType::Import,
        Some("doc-b"),
        2_000,
        OriginalsFlag::Unset,
        r#"{"marker":"row2"}"#,
    )
    .expect("append 2");
    let row3 = audit::append(
        wired.vault.as_ref(),
        &key,
        EventType::Import,
        Some("doc-c"),
        3_000,
        OriginalsFlag::Unset,
        r#"{"marker":"row3"}"#,
    )
    .expect("append 3");
    persist_head_for(&wired.keystore, &row3);

    // Corrupt row 2 — the chain is internally broken starting there, so the verified
    // prefix is exactly row 1 (`first_bad_sequence == 2`, `tail_sequence == 1`).
    wired
        .vault
        .test_only_corrupt_payload(2)
        .expect("corrupt row 2");
    wired.mgr.lock().expect("lock");
    let out = wired
        .mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock still succeeds");
    assert_eq!(out.state, SessionState::DegradedIntegrity);
    let report = out.integrity.expect("degraded carries a report");
    assert_eq!(report.first_bad_sequence, Some(2));
    assert_eq!(report.tail_sequence, 1);

    let listed = wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: 200,
        })
        .expect("degraded session may still list_audit_events");
    assert_eq!(listed.events.len(), 1, "{listed:?}");
    assert_eq!(listed.events[0].sequence, row1.sequence);
    assert_eq!(listed.events[0].doc_id.as_deref(), Some("doc-a"));
    assert!(listed.next_sequence.is_none());
}

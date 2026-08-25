//! W39 — Performance budgets (design.md §7, testing.md §8).
//!
//! Spec sources:
//! - `docs/specs/design.md` §7: import ≤2s/1MB, detect ≤5s/1MB, approval payload ≤1s for
//!   ≤200 fields, export (single doc, ≤25MB) ≤5s, vault unlock ≤1s after passphrase, audit
//!   query (last 1000 events) ≤500ms — mainstream laptop (8 GB RAM, SSD).
//! - `docs/specs/testing.md` §8 "Unlock budget: Perf job: ≤ 1 s after passphrase... on the
//!   documented runner."
//! - `docs/dev-plan.md` W39 ("Tests first: perf job may be assert-with-timeout."
//!   "Integrate: nightly CI, not flaky PR gate." "Do not: make perf a PR flake gate.")
//!
//! Every test is `#[ignore]`d (same convention as `ollama_w15b.rs`'s
//! `nightly_real_ollama`) and run explicitly via
//! `cargo test -p pg-core --test perf_w39 -- --ignored --nocapture` from
//! `.github/workflows/nightly.yml`, never from the PR `test` job — wall-clock assertions on
//! a shared CI runner are exactly the kind of flake dev-plan W39 says not to gate PRs on.
//!
//! `import_document` fuses import + detect into one call (W14: detect runs synchronously
//! inside it, not as a separate command), so there is no seam to time detect in isolation.
//! The import+detect test budgets the fused call at the sum of both design.md §7 numbers
//! (2s + 5s = 7s) rather than inventing a detect-only entry point that does not exist.
//!
//! The audit-query budget is measured at `list_audit_events`'s real maximum `limit` of 200
//! (session.rs), not the design.md §7 "last 1000 events" — see that test's comment for the
//! design/implementation mismatch this exposes.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::DocumentStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keystore::{InMemoryKeystore, KeystoreBackend};
use pg_core::session::{
    CommitShareIn, CreateAccountIn, ImportDocumentIn, ListAuditEventsIn, OpenApprovalIn,
    PreviewShareIn, SessionManager, SetRetentionDefaultIn, ShareKind, ShareRequestDto, UnlockIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple perf";

struct Wired {
    mgr: SessionManager,
    _dir: tempfile::TempDir,
}

fn fresh() -> Wired {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault_path = dir.path().join("vault.db");
    let vault = Arc::new(SqlCipherVault::new(vault_path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let keystore: Arc<dyn KeystoreBackend> = Arc::new(InMemoryKeystore::new());
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents)
        .with_detector(Arc::new(StubDetector));
    mgr.create_account(CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    })
    .expect("create_account");
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm retention");
    Wired { mgr, _dir: dir }
}

/// A ~1 MB fixture. Real prose padding around a handful of `PG-CANARY-` tokens so the
/// stub detector has something to find without inflating the field count past 200.
fn one_megabyte_text() -> Vec<u8> {
    let mut s = String::with_capacity(1_100_000);
    while s.len() < 1_000_000 {
        s.push_str("The quick brown fox jumps over the lazy dog. ");
    }
    s.push_str(" Reference PG-CANARY-PERF-1 and PG-CANARY-PERF-2 appear here.");
    s.into_bytes()
}

#[test]
#[ignore = "nightly perf budget (design.md §7); wall-clock, not a PR gate"]
fn unlock_within_one_second_of_passphrase() {
    let mut wired = fresh();
    wired.mgr.lock().expect("lock");

    let start = Instant::now();
    wired
        .mgr
        .unlock(UnlockIn {
            passphrase: PASSPHRASE.to_string(),
        })
        .expect("unlock");
    let elapsed = start.elapsed();

    assert!(
        elapsed <= Duration::from_secs(1),
        "design.md §7: unlock must be ≤1s after passphrase, took {elapsed:?}"
    );
}

#[test]
#[ignore = "nightly perf budget (design.md §7); wall-clock, not a PR gate"]
fn import_and_detect_a_1mb_document_within_budget() {
    let mut wired = fresh();
    let bytes = one_megabyte_text();
    assert!(bytes.len() <= 1_000_000 + 1024, "fixture must stay at the ≤1MB budget tier");

    let start = Instant::now();
    let out = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "perf.txt".to_string(),
            bytes,
            retention_override: None,
        })
        .expect("import_document");
    let elapsed = start.elapsed();

    assert!(!out.over_budget, "a ~1MB fixture must stay under the 25MB over_budget threshold");
    // design.md §7: import ≤2s + detect ≤5s, fused into one call (see module doc).
    assert!(
        elapsed <= Duration::from_secs(7),
        "design.md §7: fused import+detect of a ≤1MB document must be ≤7s, took {elapsed:?}"
    );
}

#[test]
#[ignore = "nightly perf budget (design.md §7); wall-clock, not a PR gate"]
fn approval_payload_for_up_to_200_fields_within_one_second() {
    let mut wired = fresh();
    let mut body = String::new();
    for i in 0..200 {
        body.push_str(&format!("PG-CANARY-{i:03} "));
    }
    let doc_id = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "fields.txt".to_string(),
            bytes: body.into_bytes(),
            retention_override: None,
        })
        .expect("import_document")
        .summary
        .doc_id;

    let start = Instant::now();
    let view = wired
        .mgr
        .open_approval(OpenApprovalIn { doc_id })
        .expect("open_approval");
    let elapsed = start.elapsed();

    assert!(view.fields.len() >= 100, "fixture should produce a large field set");
    assert!(
        elapsed <= Duration::from_secs(1),
        "design.md §7: approval payload for ≤200 fields must be ≤1s, took {elapsed:?}"
    );
}

#[test]
#[ignore = "nightly perf budget (design.md §7); wall-clock, not a PR gate"]
fn export_a_single_document_within_budget() {
    let mut wired = fresh();
    let doc_id = wired
        .mgr
        .import_document(ImportDocumentIn {
            filename: "export.txt".to_string(),
            bytes: b"Nothing sensitive here.".to_vec(),
            retention_override: None,
        })
        .expect("import_document")
        .summary
        .doc_id;
    let view = wired
        .mgr
        .open_approval(OpenApprovalIn {
            doc_id: doc_id.clone(),
        })
        .expect("open_approval");
    wired
        .mgr
        .set_field_decisions(pg_core::session::SetFieldDecisionsIn {
            approval_session_id: view.approval_session_id.clone(),
            decisions: vec![],
        })
        .expect("set_field_decisions");
    wired
        .mgr
        .submit_approval(pg_core::session::SubmitApprovalIn {
            approval_session_id: view.approval_session_id,
        })
        .expect("submit_approval");

    let start = Instant::now();
    let preview = wired
        .mgr
        .preview_share(PreviewShareIn {
            request: ShareRequestDto {
                kind: ShareKind::ExportToPerson,
                doc_ids: vec![doc_id],
                per_doc_overrides: Default::default(),
                applied_variant_ids: Default::default(),
                recipient_note: Some("caseworker".to_string()),
                ai_instruction: None,
            },
        })
        .expect("preview_share");
    wired
        .mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit_share");
    let elapsed = start.elapsed();

    assert!(
        elapsed <= Duration::from_secs(5),
        "design.md §7: single-document export (≤25MB) must be ≤5s, took {elapsed:?}"
    );
}

#[test]
#[ignore = "nightly perf budget (design.md §7); wall-clock, not a PR gate"]
fn audit_query_within_budget() {
    // design.md §7 budgets the *last 1000 events*, but `list_audit_events`'s real `limit`
    // is capped at 200 (session.rs: "limit must be 1..=200" — api.md §5.8 never documents
    // a 1000 tier). That is a design.md-vs-implementation mismatch, not something this test
    // should paper over: it queries at the actual maximum (200) against a much smaller
    // corpus (one import's worth of audit rows) rather than inventing a 1000-row fixture the
    // command cannot even request. The 500ms assertion is unchanged from the spec number.
    let mut wired = fresh();
    for i in 0..20 {
        wired
            .mgr
            .import_document(ImportDocumentIn {
                filename: format!("doc-{i}.txt"),
                bytes: b"filler".to_vec(),
                retention_override: None,
            })
            .expect("import_document");
    }

    let start = Instant::now();
    wired
        .mgr
        .list_audit_events(ListAuditEventsIn {
            doc_id: None,
            event_type: None,
            after_sequence: None,
            limit: Some(200),
        })
        .expect("list_audit_events");
    let elapsed = start.elapsed();

    assert!(
        elapsed <= Duration::from_millis(500),
        "design.md §7: audit trail query must be ≤500ms, took {elapsed:?}"
    );
}

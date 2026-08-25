//! W39 — No-plaintext-to-disk watcher (testing.md §8, architecture §5).
//!
//! Spec sources:
//! - `docs/specs/testing.md` §8 "No plaintext-to-disk": "Component test: import/detect/
//!   export with a temp-dir watcher; no file under the sandbox contains fixture plaintext
//!   except `vault.db` ciphertext (must not match plaintext) and the Linux fallback wrap
//!   blob (must not match passphrase or document text)."
//! - `docs/dev-plan.md` W39 ("watcher fails if fixture plaintext appears outside
//!   ciphertext"; "Integrate: nightly CI, not flaky PR gate" — that line targets the perf
//!   half of W39, not this one: this check is deterministic, not timing-based, so it runs
//!   on every PR like any other component test).
//! - `docs/specs/architecture.md` §5 (no new plaintext-to-disk path, ever).
//!
//! Approach: run a full import -> detect -> approve (redact one field, keep another) ->
//! export -> lock flow entirely inside one `tempfile::tempdir()` sandbox, using
//! [`FileKeystore`] so the Linux 0600 fallback blob is also written into that sandbox
//! (exercised on every OS this test runs on, same as AC-5). After `lock()` closes every
//! handle, recursively walk the sandbox and assert that no file — named or not — contains
//! the passphrase or the canary marker, in raw UTF-8 or UTF-16 form. `vault.db` and the
//! fallback blob are expected to *exist*; they are not exempted from the *content* check.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::DocumentStore;
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::detector::StubDetector;
use pg_core::keystore::{FileKeystore, KeystoreBackend};
use pg_core::session::{
    CommitShareIn, CreateAccountIn, FieldDecisionDto, FieldDecisionKind, ImportDocumentIn,
    OpenApprovalIn, PreviewShareIn, SessionManager, SetFieldDecisionsIn, SetRetentionDefaultIn,
    ShareKind, ShareRequestDto, SubmitApprovalIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple w39";
// Both markers must be whitespace-delimited tokens for `StubDetector` (core/src/
// detector/mod.rs: scans whitespace-delimited tokens containing "PG-CANARY-").
const REDACT_CANARY: &str = "PG-CANARY-DISK-9Q7Z";
const KEEP_CANARY: &str = "PG-CANARY-KEEP-4M2B";

fn recursive_walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            recursive_walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn contains_utf8(haystack: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    !n.is_empty() && haystack.windows(n.len()).any(|w| w == n)
}

fn contains_utf16(haystack: &[u8], needle: &str) -> bool {
    let le: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let be: Vec<u8> = needle.encode_utf16().flat_map(u16::to_be_bytes).collect();
    haystack.windows(le.len()).any(|w| w == le.as_slice())
        || haystack.windows(be.len()).any(|w| w == be.as_slice())
}

fn assert_absent_everywhere(sandbox: &Path, needle: &str, label: &str) {
    let mut files = Vec::new();
    recursive_walk(sandbox, &mut files);
    assert!(!files.is_empty(), "sandbox should contain vault.db + keystore fallback at least");
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        assert!(
            !contains_utf8(&bytes, needle) && !contains_utf16(&bytes, needle),
            "testing.md §8 no-plaintext-to-disk: {path:?} contains {label} ({needle:?})"
        );
    }
}

#[test]
fn no_fixture_plaintext_survives_import_detect_approve_export_lock() {
    let dir = tempfile::tempdir().expect("temp dir");
    let sandbox = dir.path().to_path_buf();
    let vault_path = sandbox.join("vault.db");
    let fallback_path = sandbox.join("keystore.json");

    let vault = Arc::new(SqlCipherVault::new(vault_path.clone()));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let keystore: Arc<dyn KeystoreBackend> = Arc::new(FileKeystore::new(fallback_path.clone()));

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

    let letter = format!(
        "Dear Sir, your reference is {REDACT_CANARY} and {KEEP_CANARY} here."
    );

    let doc_id = mgr
        .import_document(ImportDocumentIn {
            filename: "letter.txt".to_string(),
            bytes: letter.into_bytes(),
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
    assert!(!view.fields.is_empty(), "stub detector must find both canary tokens");

    let decisions: Vec<FieldDecisionDto> = view
        .fields
        .iter()
        .map(|f| {
            let text = f.span.text.as_deref().expect("C-API-2: span text on open_approval");
            let redact = text.contains(REDACT_CANARY);
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

    mgr.set_field_decisions(SetFieldDecisionsIn {
        approval_session_id: view.approval_session_id.clone(),
        decisions,
    })
    .expect("set_field_decisions");
    mgr.submit_approval(SubmitApprovalIn {
        approval_session_id: view.approval_session_id,
    })
    .expect("submit_approval");

    let preview = mgr
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
    let commit = mgr
        .commit_share(CommitShareIn {
            preview_token: preview.preview_token,
        })
        .expect("commit_share");
    // The export artifact (redacted PDF bytes) is returned in-process to the caller —
    // never written to disk by the core (architecture §5: only the webview's save dialog,
    // outside this seam, writes it). Dropping it here without writing anywhere is the
    // point: nothing in this test ever calls `std::fs::write` with fixture bytes.
    drop(commit.pdf_bytes);

    mgr.lock().expect("lock closes the DB handle and zeroizes key material");
    drop(mgr);

    assert!(vault_path.exists(), "vault.db must exist under the sandbox");
    assert!(fallback_path.exists(), "Linux fallback keystore blob must exist under the sandbox");

    assert_absent_everywhere(&sandbox, REDACT_CANARY, "the redacted canary");
    assert_absent_everywhere(&sandbox, PASSPHRASE, "the passphrase");
    // The kept field is still approved-content plaintext; it must only ever exist as
    // AEAD ciphertext inside vault.db, never as a raw byte match on disk.
    assert_absent_everywhere(&sandbox, KEEP_CANARY, "the kept canary (must be ciphertext-only)");
}

#[test]
#[should_panic(expected = "contains the passphrase")]
fn self_test_catches_a_planted_leak() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("leak.txt"), PASSPHRASE.as_bytes()).expect("plant leak");
    assert_absent_everywhere(dir.path(), PASSPHRASE, "the passphrase");
}

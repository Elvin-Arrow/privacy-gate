//! W11 — Import blocked until retention confirmed.
//!
//! Spec sources:
//! - `docs/decisions/0007-retention-default-discard.md` ("No import succeeds until the
//!   policy is confirmed")
//! - `docs/specs/api.md` §5.3 (`import_document`'s `retention_policy_unset` /
//!   `retention_loosen_forbidden`)
//! - `docs/specs/testing.md` §6.6 (AC-6 — paranoid retention default), §6.7 (AC-7 —
//!   factory discard and first-import confirmation)
//! - `docs/dev-plan.md` W11 ("Tests first: AC-7 command scenario; AC-6 paranoid loosen
//!   forbidden.")
//!
//! Out of W11 scope and deliberately absent here: the first-import modal UI (W32) — this
//! file only asserts the API gate and factory value, per testing.md §6.7's own framing
//! ("Pre-select chrome is UI spec; this scenario only asserts the API gate and factory
//! value").

use std::sync::Arc;

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::{DocumentStore, EffectiveRetention};
use pg_core::config::{ConfigStore, RetentionPolicy};
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::{
    retention_override_forbidden, CreateAccountIn, ImportDocumentIn, SessionManager,
    SetRetentionDefaultIn,
};
use pg_core::vault::{SqlCipherVault, VaultBackend};

const PASSPHRASE: &str = "correct horse battery staple";

fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.db");
    (dir, path)
}

fn create_in() -> CreateAccountIn {
    CreateAccountIn {
        display_name: "Alex".to_string(),
        passphrase: PASSPHRASE.to_string(),
    }
}

fn fresh_unconfirmed() -> (SessionManager, tempfile::TempDir) {
    let (dir, path) = temp_db_path();
    let keystore = Arc::new(InMemoryKeystore::new());
    let vault = Arc::new(SqlCipherVault::new(path));
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault;
    let mut mgr = SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents);
    mgr.create_account(create_in()).expect("create_account");
    (mgr, dir)
}

fn import_in(filename: &str, bytes: &[u8], retention_override: Option<EffectiveRetention>) -> ImportDocumentIn {
    ImportDocumentIn {
        filename: filename.to_string(),
        bytes: bytes.to_vec(),
        retention_override,
    }
}

// ---------------------------------------------------------------------------
// testing.md §6.7 AC-7 — Factory discard and first-import confirmation
// ---------------------------------------------------------------------------

#[test]
fn ac7_factory_default_is_discard_and_unconfirmed_after_create_account() {
    let (mgr, _dir) = fresh_unconfirmed();
    let out = mgr.get_retention_default().expect("get_retention_default");
    assert_eq!(out.policy, RetentionPolicy::Discard);
    assert!(!out.confirmed);
}

#[test]
fn ac7_import_before_confirmation_is_retention_policy_unset_with_no_override() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    let err = mgr
        .import_document(import_in("letter.txt", b"hello world", None))
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::RetentionPolicyUnset);

    let listed = mgr.list_documents().expect("list_documents");
    assert!(listed.documents.is_empty(), "no catalog row on an unconfirmed import");
}

/// AC-7: "`import_document` (any override, including null) → `retention_policy_unset`" —
/// even an override that would otherwise be perfectly valid does not bypass confirmation.
#[test]
fn ac7_import_before_confirmation_is_retention_policy_unset_even_with_an_override() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    let err = mgr
        .import_document(import_in(
            "letter.txt",
            b"hello world",
            Some(EffectiveRetention::Discard),
        ))
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::RetentionPolicyUnset);
}

#[test]
fn ac7_set_retention_default_confirms_for_any_of_the_three_policies() {
    for policy in [
        RetentionPolicy::Discard,
        RetentionPolicy::Retain,
        RetentionPolicy::NeverRetain,
    ] {
        let (mut mgr, _dir) = fresh_unconfirmed();
        let out = mgr
            .set_retention_default(SetRetentionDefaultIn { policy })
            .expect("set_retention_default");
        assert!(out.confirmed);
    }
}

#[test]
fn ac7_import_proceeds_after_confirmation() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm discard");

    let out = mgr
        .import_document(import_in("letter.txt", b"hello world", None))
        .expect("import_document must proceed once confirmed");
    assert_eq!(out.summary.source_filename, "letter.txt");
}

/// AC-7: "Confirming discard then overriding that first import to `retain` is allowed
/// (FR-1.3 vs FR-1.4 stay distinct)."
#[test]
fn ac7_confirming_discard_then_overriding_first_import_to_retain_is_allowed() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::Discard,
    })
    .expect("confirm discard");

    let out = mgr
        .import_document(import_in(
            "letter.txt",
            b"hello world",
            Some(EffectiveRetention::Retain),
        ))
        .expect("per-import override to retain, against a discard default, must succeed");
    assert_eq!(out.summary.retention, EffectiveRetention::Retain);
    assert!(out.summary.has_retained_original);
}

// ---------------------------------------------------------------------------
// testing.md §6.6 AC-6 — Paranoid retention default
// ---------------------------------------------------------------------------

#[test]
fn ac6_never_retain_default_forbids_per_import_retain_override() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::NeverRetain,
    })
    .expect("confirm never_retain");

    let err = mgr
        .import_document(import_in(
            "letter.txt",
            b"hello world",
            Some(EffectiveRetention::Retain),
        ))
        .unwrap_err();
    assert_eq!(err.code, pg_core::api::ErrorCode::RetentionLoosenForbidden);

    let listed = mgr.list_documents().expect("list_documents");
    assert!(listed.documents.is_empty(), "no catalog row on a forbidden loosen attempt");
}

/// AC-6: "Per-import discard is allowed (tighten)." Only the *retain* direction is
/// forbidden against `never_retain` — discard, which is already the enforced outcome, must
/// not be refused as if it were a loosening attempt.
#[test]
fn ac6_never_retain_default_allows_per_import_discard_override() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::NeverRetain,
    })
    .expect("confirm never_retain");

    let out = mgr
        .import_document(import_in(
            "letter.txt",
            b"hello world",
            Some(EffectiveRetention::Discard),
        ))
        .expect("explicit discard override against never_retain must be allowed");
    assert_eq!(out.summary.retention, EffectiveRetention::Discard);
}

/// AC-6's implicit case: no override at all against `never_retain` must import as
/// `discard` (data-model §6.1), not fail — there is nothing to "loosen" when no override
/// was given.
#[test]
fn ac6_never_retain_default_with_no_override_imports_as_discard() {
    let (mut mgr, _dir) = fresh_unconfirmed();
    mgr.set_retention_default(SetRetentionDefaultIn {
        policy: RetentionPolicy::NeverRetain,
    })
    .expect("confirm never_retain");

    let out = mgr
        .import_document(import_in("letter.txt", b"hello world", None))
        .expect("import with no override must succeed under never_retain");
    assert_eq!(out.summary.retention, EffectiveRetention::Discard);
}

/// testing.md §5.3 `retention_loosen_forbidden`: unit table so a `&&`→`||` or
/// `NeverRetain`→`Retain` mutant in the helper cannot hide behind command-level
/// `import_document` setup.
#[test]
fn retention_override_forbidden_is_only_never_retain_plus_retain() {
    for policy in [
        RetentionPolicy::Discard,
        RetentionPolicy::Retain,
        RetentionPolicy::NeverRetain,
    ] {
        assert!(
            !retention_override_forbidden(policy, None),
            "{policy:?} + None"
        );
        assert!(
            !retention_override_forbidden(policy, Some(EffectiveRetention::Discard)),
            "{policy:?} + discard"
        );
    }
    assert!(!retention_override_forbidden(
        RetentionPolicy::Discard,
        Some(EffectiveRetention::Retain),
    ));
    assert!(!retention_override_forbidden(
        RetentionPolicy::Retain,
        Some(EffectiveRetention::Retain),
    ));
    assert!(retention_override_forbidden(
        RetentionPolicy::NeverRetain,
        Some(EffectiveRetention::Retain),
    ));
}

/// A `discard` or `retain` default (not `never_retain`) never triggers
/// `retention_loosen_forbidden` — the restriction is specific to the paranoid default.
#[test]
fn non_paranoid_defaults_never_trigger_loosen_forbidden() {
    for policy in [RetentionPolicy::Discard, RetentionPolicy::Retain] {
        let (mut mgr, _dir) = fresh_unconfirmed();
        mgr.set_retention_default(SetRetentionDefaultIn { policy })
            .expect("confirm");
        let out = mgr
            .import_document(import_in(
                "letter.txt",
                b"hello world",
                Some(EffectiveRetention::Retain),
            ))
            .expect("retain override must never be forbidden against a non-paranoid default");
        assert_eq!(out.summary.retention, EffectiveRetention::Retain);
    }
}

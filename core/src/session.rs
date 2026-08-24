//! Session, account, config, and catalog commands (`api.md` §2, §5.1–§5.3;
//! `architecture.md` §3.2–§3.4, §6, §7, §10.1.3).
//!
//! Includes [`SessionManager::get_detector_preference`] /
//! [`SessionManager::set_detector_preference`] (W15c), catalog commands (W10), and
//! [`SessionManager::open_approval`] / [`SessionManager::get_approval_view`] /
//! [`SessionManager::set_field_decisions`] (W16) / [`SessionManager::submit_approval`] (W18).
//!
//! `dev-plan.md` §1: "**Integration seam for v1 core:** in-process API commands, not the
//! webview." Tauri IPC wiring is W29; these are the functions it will call.
//!
//! # Scope fence (dev-plan.md W6 "Do not: first-import modal UI (W32); per-import override
//! (W10)")
//!
//! Approval commands (`open_approval` / `get_approval_view` / `set_field_decisions`, W16;
//! `submit_approval`, W18; `abort_approval`, W19) hold one RAM session on [`OpenSession`].
//! Lock drops unapproved discard catalog rows (data-model §8).
//! `detector_preference` is read and written by W15c's commands and consulted on each
//! `import_document` detect (architecture §10.1.3) — not cached at unlock.
//!
//! # `degraded_integrity` (architecture §6.3, W5)
//!
//! `architecture.md` §3.3 defines unlock as: load the item, derive `wrap_key`, unwrap
//! `vault_master_key`, derive the subkeys, open the DB, **verify the audit chain against
//! `audit_head` (§6.3)**. `unlock` now does all of that: after the vault opens, the audit
//! chain (`crate::audit`) replays and verifies against the persisted `AuditHead`, and one
//! of three things happens — clean, crash-window fast-forward, or integrity failure. Only
//! the third produces [`SessionState::DegradedIntegrity`]; the other two both report
//! `"unlocked"` (api.md §5.1: crash-window fast-forward "returns `\"unlocked\"`", not a
//! fourth state).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::account::{
    new_account_id, now_rfc3339, AccountStore, AccountStoreError, LocalAccount,
};
use crate::api::{ApiError, ErrorCode};
use crate::audit::{AuditError, AuditRow, AuditStore, EventType, FailureKind, OriginalsFlag, VerifyOutcome};
use crate::catalog::{
    ApprovedVersion, CatalogError, DocumentMeta, DocumentStore, EffectiveRetention,
    FieldDecision, NullDocumentStore, OriginalRecord,
};
use crate::config::{ConfigError, ConfigStore, DetectorPreference, NullConfigStore, RetentionPolicy};
use crate::detector::{
    default_ollama_allowlist, AllowlistEntry, Detector, FallbackReason, HybridOllamaV1, HybridV1,
    OllamaClient, HYBRID_OLLAMA_V1_ID, HYBRID_V1_ID, OLLAMA_LOOPBACK_ADDR,
};
use crate::importer::{self, Document, SourceFormat};
use crate::keys::{unwrap_master_key, wrap_master_key, VaultMasterKey, KEY_LEN};
use crate::keystore::{
    Argon2idParams, AuditHead, KeystoreBackend, KeystoreBackendKind, KeystoreError, KeystoreItem,
};
use crate::vault::{NullVault, VaultBackend, VaultError};

/// The W2/W3-era no-op audit backend: `append_row` errors (nothing should ever call it —
/// there is no live connection to write to), `replay` returns an empty chain. Exists so
/// `SessionManager::new` and `SessionManager::new_with_vault` (both predate W5) keep
/// working unmodified — an empty replay against `AuditHead::GENESIS` is always
/// [`VerifyOutcome::Clean`], so a session built without a real audit store behaves exactly
/// as W2/W3 always did: `unlock` succeeds or fails on the passphrase alone.
#[derive(Debug, Default)]
struct NullAuditStore;

impl AuditStore for NullAuditStore {
    fn append_row(&self, _row: &AuditRow) -> Result<(), AuditError> {
        Err(AuditError::Backend("no audit store configured"))
    }
    fn replay(&self) -> Result<Vec<AuditRow>, AuditError> {
        Ok(Vec::new())
    }
}

/// api.md §6 event name. W29's Tauri shim emits under this string; in-process tests
/// assert the constant so a rename cannot silently desync the webview listener.
pub const DETECT_PROGRESS_EVENT: &str = "pg://detect-progress";

/// api.md §6 `phase` on `pg://detect-progress`. [`DetectPhase::Detecting`] is the
/// bundled/stub/fallback path. [`DetectPhase::WarmingModel`] is emitted only after a
/// successful Ollama handshake (architecture §10.1.5); handshake failure never uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectPhase {
    Detecting,
    WarmingModel,
}

/// api.md §6 `pg://detect-progress` payload. No field text, keys, or passphrases
/// (api.md §6 last line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectProgress {
    pub doc_id: String,
    pub fraction: f64,
    pub phase: DetectPhase,
}

/// In-process subscriber for [`DetectProgress`]. The default is a no-op; tests and the
/// W29 Tauri shim supply a real one. Emit is synchronous so a blocking-pool import
/// (api.md §5.3) can flush to the webview between fractions.
pub trait ProgressSink: Send + Sync {
    fn emit_detect_progress(&self, event: DetectProgress);
}

struct NullProgressSink;

impl ProgressSink for NullProgressSink {
    fn emit_detect_progress(&self, _event: DetectProgress) {}
}

/// api.md §5.1: "`passphrase` min length 8 (API floor; UI spec may urge longer)."
pub const MIN_PASSPHRASE_LEN: usize = 8;

/// api.md §5.1 / data-model §5.6: "`display_name` trimmed, 1..=80 chars."
pub const MAX_DISPLAY_NAME_CHARS: usize = 80;

// ---------------------------------------------------------------------------
// api.md §2 — session model
// ---------------------------------------------------------------------------

/// `SessionState = "first_run" | "locked" | "unlocked" | "degraded_integrity"` (api.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// No account exists yet. Only `get_session_state` and `create_account` are allowed.
    FirstRun,
    /// An account exists and the vault is closed.
    Locked,
    /// The vault is open; key material is resident.
    Unlocked,
    /// **Not reachable in W2.** The passphrase was correct but the audit chain failed
    /// verification (architecture §6.3): the session may return an integrity report and
    /// `get_account`'s id + display name, and must not decrypt any artifact. W5 produces
    /// this state; the variant exists now so adding it later is not a breaking change to
    /// this type or to `api.md`'s wire enum.
    DegradedIntegrity,
}

impl SessionState {
    /// The stable wire string (api.md §2).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionState::FirstRun => "first_run",
            SessionState::Locked => "locked",
            SessionState::Unlocked => "unlocked",
            SessionState::DegradedIntegrity => "degraded_integrity",
        }
    }
}

impl core::fmt::Display for SessionState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// api.md §2 — the session gating table (W4)
// ---------------------------------------------------------------------------

/// The single source of truth for which `SessionState`s each **registered** command may
/// run in — api.md §2's matrix, restated as data instead of scattered per-method `if`s.
///
/// dev-plan W4: "adding a command requires a new row in the table test (will fail until
/// filled)." That is literally true here: [`command_allowed`] returns `false` for any
/// command name not listed, so a new command with no row is refused in every state until
/// someone adds one — the failure mode is "nothing works" rather than "silently allowed
/// everywhere."
///
/// `get_session_state` is deliberately **absent**: api.md §2 says it is "callable in every
/// state (including before first run)," i.e. it has no gate at all, so it has no row.
///
/// Only commands that exist so far are listed (dev-plan W4 "Do not: implement
/// gated-but-unwritten commands"). `list_audit_events` and every share/variant
/// command remain unregistered. Config, catalog, and approval commands that do exist
/// share api.md §2's generic document/config row: `no | no | yes | no` — unavailable
/// while `degraded_integrity` (C-API-6), unlike `lock`/`get_account`/`get_integrity_report`.
///
/// [`SessionState::DegradedIntegrity`] appears in the table even though no command in this
/// codebase can put a live `SessionManager` into that state yet (W5's gap — see
/// [`SessionManager::verify_integrity_on_unlock`]). The table states the spec's answer for
/// that column now, so W5 only has to make the state reachable, not re-derive which
/// commands accept it.
const SESSION_TABLE: &[(&str, &[SessionState])] = &[
    (
        "create_account",
        &[SessionState::FirstRun],
    ),
    ("unlock", &[SessionState::Locked]),
    (
        "lock",
        &[SessionState::Unlocked, SessionState::DegradedIntegrity],
    ),
    ("change_passphrase", &[SessionState::Unlocked]),
    (
        "get_account",
        &[SessionState::Unlocked, SessionState::DegradedIntegrity],
    ),
    (
        // W5: api.md §2 row — `get_integrity_report` | no | no | yes | yes.
        "get_integrity_report",
        &[SessionState::Unlocked, SessionState::DegradedIntegrity],
    ),
    (
        // W6: api.md §2's generic config/document/... row — `no | no | yes | no`. Unlike
        // `get_account`/`get_integrity_report`/`lock`, config commands are **not**
        // available while degraded (C-API-6).
        "get_retention_default",
        &[SessionState::Unlocked],
    ),
    ("set_retention_default", &[SessionState::Unlocked]),
    (
        "get_detector_preference",
        &[SessionState::Unlocked],
    ),
    ("set_detector_preference", &[SessionState::Unlocked]),
    // W10: api.md §5.3, same generic config/document row as retention (`no | no | yes |
    // no` — unavailable while degraded, C-API-6).
    ("import_document", &[SessionState::Unlocked]),
    ("list_documents", &[SessionState::Unlocked]),
    ("get_document", &[SessionState::Unlocked]),
    // W16: api.md §5.4, same generic document row (`no | no | yes | no`).
    ("open_approval", &[SessionState::Unlocked]),
    ("get_approval_view", &[SessionState::Unlocked]),
    ("set_field_decisions", &[SessionState::Unlocked]),
    ("submit_approval", &[SessionState::Unlocked]),
    ("abort_approval", &[SessionState::Unlocked]),
    ("delete_document", &[SessionState::Unlocked]),
    ("delete_retained_original", &[SessionState::Unlocked]),
];

/// api.md §2: is `command` callable while the session is in `state`? `false` for any
/// command name absent from [`SESSION_TABLE`] — including a typo'd name, which is exactly
/// the "adding a command requires a new row" failure mode dev-plan W4 asks for.
///
/// `pub` so `core/tests/session_gating_w4.rs` can assert the full api.md §2 matrix
/// (including `degraded_integrity`, unreachable through a live `SessionManager` until W5)
/// directly against the table, not only through the states a live session can reach today.
/// W29's Tauri dispatcher is the other intended caller (dev-plan W4: "Integrate: single
/// gate in the command dispatcher").
#[must_use]
pub fn command_allowed(command: &str, state: SessionState) -> bool {
    SESSION_TABLE
        .iter()
        .find(|(name, _)| *name == command)
        .is_some_and(|(_, states)| states.contains(&state))
}

/// api.md §5.1 `get_integrity_report` Out shape.
///
/// Constructed by `unlock`/`create_account` (W5, architecture §6.3) and served back
/// unchanged by [`SessionManager::get_integrity_report`] for the life of the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityReport {
    /// True after a clean unlock or a crash-window fast-forward.
    pub ok: bool,
    /// `"ok" | "crash_window_fast_forwarded" | "truncation" | "modification"`.
    pub kind: String,
    /// Persisted `audit_head.sequence`.
    pub head_sequence: u64,
    /// Verified tail sequence of the DB chain.
    pub tail_sequence: u64,
    /// First sequence that failed verification, if any.
    pub first_bad_sequence: Option<u64>,
}

// ---------------------------------------------------------------------------
// api.md §5.1 — command In/Out shapes
//
// Input types that carry a passphrase implement `Debug` **by hand**. A derived one would
// print the passphrase into any `{:?}` log line, panic message, or `unwrap()` on a
// `Result` holding the input — which is precisely the C-API-1 leak this constraint
// forbids. `display_name` is not a secret (data-model §5.6) and stays visible.
// ---------------------------------------------------------------------------

/// `create_account` In: `{ display_name: string, passphrase: string }`.
#[derive(Clone, Serialize, Deserialize)]
pub struct CreateAccountIn {
    /// Trimmed to 1..=80 chars. Not a secret.
    pub display_name: String,
    /// Min length 8. Never stored, never echoed (C-API-1).
    pub passphrase: String,
}

impl core::fmt::Debug for CreateAccountIn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CreateAccountIn")
            .field("display_name", &self.display_name)
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

/// `create_account` Out: `{ account_id: string, state: "unlocked" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAccountOut {
    /// The new `LocalAccount.id`.
    pub account_id: String,
    /// Always [`SessionState::Unlocked`] — first-run opens the vault.
    pub state: SessionState,
}

/// `unlock` In: `{ passphrase: string }`.
#[derive(Clone, Serialize, Deserialize)]
pub struct UnlockIn {
    /// Never stored, never echoed (C-API-1).
    pub passphrase: String,
}

impl core::fmt::Debug for UnlockIn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UnlockIn")
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

/// `unlock` Out: `{ state, integrity: IntegrityReport | null }`.
///
/// api.md §5.1: "`integrity` is non-null iff `degraded_integrity`." W2 cannot reach that
/// state, so it is always `None` here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlockOut {
    /// `"unlocked"` or (from W5) `"degraded_integrity"`.
    pub state: SessionState,
    /// Non-null iff `state == degraded_integrity`.
    pub integrity: Option<IntegrityReport>,
}

/// `lock` Out: `{ state: "locked" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockOut {
    /// Always [`SessionState::Locked`].
    pub state: SessionState,
}

/// `get_session_state` Out: `{ state: SessionState }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStateOut {
    /// The current state.
    pub state: SessionState,
}

/// `change_passphrase` In: `{ current: string, new_passphrase: string }`.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChangePassphraseIn {
    /// The passphrase currently protecting the vault.
    pub current: String,
    /// The replacement. Min length 8, same API floor as `create_account`.
    pub new_passphrase: String,
}

impl core::fmt::Debug for ChangePassphraseIn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChangePassphraseIn")
            .field("current", &"<redacted>")
            .field("new_passphrase", &"<redacted>")
            .finish()
    }
}

/// `change_passphrase` Out: `{ ok: true }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePassphraseOut {
    /// Always `true`; failures are `ApiError`s.
    pub ok: bool,
}

/// `get_account` Out: `{ account_id, display_name, created_at }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetAccountOut {
    /// `LocalAccount.id`.
    pub account_id: String,
    /// `LocalAccount.display_name`, already trimmed.
    pub display_name: String,
    /// RFC 3339 UTC.
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// api.md §5.2 — config commands (W6)
// ---------------------------------------------------------------------------

/// `get_retention_default` Out, and `set_retention_default` Out (api.md §5.2: identical
/// shape, `confirmed` always `true` on the `set` response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionDefaultOut {
    pub policy: RetentionPolicy,
    pub confirmed: bool,
}

/// `set_retention_default` In: `{ policy: RetentionPolicy }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetRetentionDefaultIn {
    pub policy: RetentionPolicy,
}

/// `get_detector_preference` Out / `set_detector_preference` Out (api.md §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorPreferenceOut {
    pub preference: DetectorPreference,
}

/// `set_detector_preference` In: `{ preference: "auto" | "bundled_only" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetDetectorPreferenceIn {
    pub preference: DetectorPreference,
}

// ---------------------------------------------------------------------------
// api.md §5.3 — import and catalog commands (W10)
// ---------------------------------------------------------------------------

/// api.md §4 `DocumentSummary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub doc_id: String,
    /// Basename only — never a filesystem path (api.md §4).
    pub source_filename: String,
    pub source_format: SourceFormat,
    /// RFC 3339 UTC.
    pub imported_at: String,
    pub retention: EffectiveRetention,
    pub has_approved_version: bool,
    pub has_retained_original: bool,
    pub detected_field_count: u32,
}

/// `import_document` In: `{ filename, bytes, retention_override }` (api.md §5.3).
///
/// `retention_override` is `EffectiveRetention`, not `RetentionPolicy` — api.md types it as
/// `"retain" | "discard" | null`, the same two-value restriction `DocumentMeta.retention`
/// itself has (never `never_retain`), so an invalid override is a compile-time
/// impossibility here rather than a runtime check.
#[derive(Clone, Serialize, Deserialize)]
pub struct ImportDocumentIn {
    pub filename: String,
    /// Binary IPC; inbound original only (C-API-3) — never echoed in any command output.
    pub bytes: Vec<u8>,
    pub retention_override: Option<EffectiveRetention>,
}

impl core::fmt::Debug for ImportDocumentIn {
    /// `bytes` omitted — not a secret, but document content has no more business in a log
    /// line than a passphrase does (architecture §5.2: "Logs... never contain document
    /// text").
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImportDocumentIn")
            .field("filename", &self.filename)
            .field("bytes_len", &self.bytes.len())
            .field("retention_override", &self.retention_override)
            .finish()
    }
}

/// `import_document` Out: `{ summary, over_budget }` (api.md §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDocumentOut {
    pub summary: DocumentSummary,
    /// True if `bytes.len()` exceeds design §7's 25 MB interactive budget. Import still
    /// completes either way — this command never rejects an over-budget input.
    pub over_budget: bool,
}

/// `list_documents` Out: `{ documents }`, newest import first (api.md §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListDocumentsOut {
    pub documents: Vec<DocumentSummary>,
}

/// `get_document` In: `{ doc_id }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDocumentIn {
    pub doc_id: String,
}

/// `get_document` Out: `{ summary }`. No pages, no field text (api.md §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDocumentOut {
    pub summary: DocumentSummary,
}

// ---------------------------------------------------------------------------
// api.md §5.4 — approval commands (W16)
// ---------------------------------------------------------------------------

/// Re-export so IPC DTOs keep using `session::FieldDecisionKind` (data-model type lives
/// on [`crate::catalog`] so `ApprovedVersion` JSON does not create a catalog→session cycle).
pub use crate::catalog::FieldDecisionKind;

/// data-model §5.10 / api.md §5.4 `ApprovalLifecycle` (W16 uses awaiting/decided;
/// committed/aborted land with W18/W19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalLifecycle {
    AwaitingDecisions,
    Decided,
    Committed,
    Aborted,
}

/// api.md §4 `DetectedFieldDto.span`. `text` is present on approval commands only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedFieldSpanDto {
    pub byte_offset: u64,
    pub byte_length: u64,
    pub text: Option<String>,
    pub page_index: u32,
}

/// api.md §4 `DetectedFieldDto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedFieldDto {
    pub id: String,
    pub label: String,
    pub classification: String,
    pub span: DetectedFieldSpanDto,
    pub parent_field_id: Option<String>,
}

/// api.md §4 `FieldDecisionDto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecisionDto {
    pub field_id: String,
    pub decision: FieldDecisionKind,
}

/// One page of [`ApprovalView`] (api.md §5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPage {
    pub page_index: u32,
    pub spans: Vec<ApprovalPageSpan>,
}

/// Page span on [`ApprovalView`] — body text for the consent step (C-DES-1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPageSpan {
    pub byte_offset: u64,
    pub text: String,
    pub page_index: u32,
}

/// `open_approval` / `get_approval_view` Out (api.md §5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalView {
    pub approval_session_id: String,
    pub doc_id: String,
    pub lifecycle: ApprovalLifecycle,
    pub pages: Vec<ApprovalPage>,
    pub fields: Vec<DetectedFieldDto>,
}

/// `open_approval` In: `{ doc_id }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApprovalIn {
    pub doc_id: String,
}

/// `get_approval_view` In: `{ approval_session_id }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetApprovalViewIn {
    pub approval_session_id: String,
}

/// `set_field_decisions` In.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetFieldDecisionsIn {
    pub approval_session_id: String,
    pub decisions: Vec<FieldDecisionDto>,
}

/// `set_field_decisions` Out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetFieldDecisionsOut {
    pub lifecycle: ApprovalLifecycle,
    pub unresolved_field_ids: Vec<String>,
}

/// `submit_approval` In: `{ approval_session_id }` (api.md §5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitApprovalIn {
    pub approval_session_id: String,
}

/// `submit_approval` Out: `{ summary, lifecycle: "committed" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitApprovalOut {
    pub summary: DocumentSummary,
    pub lifecycle: ApprovalLifecycle,
}

/// `abort_approval` In: `{ approval_session_id }` (api.md §5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortApprovalIn {
    pub approval_session_id: String,
}

/// `abort_approval` Out: `{ lifecycle: "aborted" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortApprovalOut {
    pub lifecycle: ApprovalLifecycle,
}

/// `delete_document` In: `{ doc_id }` (api.md §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteDocumentIn {
    pub doc_id: String,
}

/// `delete_document` Out: `{ ok: true }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteDocumentOut {
    pub ok: bool,
}

/// `delete_retained_original` In: `{ doc_id }` (api.md §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteRetainedOriginalIn {
    pub doc_id: String,
}

/// `delete_retained_original` Out: `{ summary }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteRetainedOriginalOut {
    pub summary: DocumentSummary,
}

/// In-process approval session (data-model §5.10). Lives on [`OpenSession`] so lock drops it.
#[derive(Debug)]
struct ApprovalSession {
    approval_session_id: String,
    doc_id: String,
    lifecycle: ApprovalLifecycle,
    document: Document,
    fields: Vec<crate::catalog::DetectedField>,
    decisions: HashMap<String, FieldDecisionKind>,
}

impl ApprovalSession {
    fn unresolved_field_ids(&self) -> Vec<String> {
        self.fields
            .iter()
            .filter(|f| !self.decisions.contains_key(&f.id))
            .map(|f| f.id.clone())
            .collect()
    }

    fn refresh_lifecycle(&mut self) {
        if matches!(
            self.lifecycle,
            ApprovalLifecycle::Committed | ApprovalLifecycle::Aborted
        ) {
            return;
        }
        self.lifecycle = if self.unresolved_field_ids().is_empty() {
            ApprovalLifecycle::Decided
        } else {
            ApprovalLifecycle::AwaitingDecisions
        };
    }

    fn to_view(&self) -> ApprovalView {
        let pages = self
            .document
            .pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let page_index = page
                    .spans
                    .first()
                    .map(|s| s.page_index)
                    .unwrap_or(i as u32);
                ApprovalPage {
                    page_index,
                    spans: page
                        .spans
                        .iter()
                        .map(|s| ApprovalPageSpan {
                            byte_offset: s.byte_offset,
                            text: s.text.clone(),
                            page_index: s.page_index,
                        })
                        .collect(),
                }
            })
            .collect();
        let fields = self
            .fields
            .iter()
            .map(|f| DetectedFieldDto {
                id: f.id.clone(),
                label: f.label.clone(),
                classification: f.classification.clone(),
                span: DetectedFieldSpanDto {
                    byte_offset: f.span.byte_offset,
                    byte_length: f.span.byte_length,
                    text: Some(f.span.text.clone()),
                    page_index: f.span.page_index,
                },
                parent_field_id: f.parent_field_id.clone(),
            })
            .collect();
        ApprovalView {
            approval_session_id: self.approval_session_id.clone(),
            doc_id: self.doc_id.clone(),
            lifecycle: self.lifecycle,
            pages,
            fields,
        }
    }
}

/// Result of one `import_document` detect phase (W15c). Not an IPC DTO.
struct DetectionRun {
    fields: Vec<crate::catalog::DetectedField>,
    detector_id: &'static str,
    backend: Option<&'static str>,
    model_tag: Option<String>,
    fallback_reason: Option<&'static str>,
}

/// design §7: documents beyond this size are outside the v1 interactive budget.
/// `import_document` still completes; `over_budget` just becomes `true`.
pub const IMPORT_BUDGET_BYTES: usize = 25 * 1024 * 1024;

// ---------------------------------------------------------------------------
// The open session
// ---------------------------------------------------------------------------

/// Everything that exists only while the vault is open.
///
/// This struct is what `lock` **drops**. Keeping the master key here rather than in an
/// always-present field on `SessionManager` is the point: after `lock()` there is no
/// field holding key material to accidentally read, so "did lock really clear it?" is a
/// type-level question rather than a runtime one. `VaultMasterKey` wraps a `Dek`, which
/// is `ZeroizeOnDrop`, so the bytes are destroyed as it goes (architecture §3.3: "Lock:
/// zeroize master key, wrap key, DEKs …").
///
/// W3 adds the SQLCipher connection here; W5 adds the verified audit head. Both then also
/// die with the session, which is why they belong in this struct and not beside it.
///
/// `degraded` and `integrity_report` are W5: architecture §6.3's verification outcome,
/// resolved once at `unlock`/`create_account` time and held for the life of the session so
/// `get_integrity_report` (api.md §5.1) has something to return without re-replaying the
/// chain on every call. `degraded` is what makes [`SessionManager::state`] able to report
/// [`SessionState::DegradedIntegrity`] instead of unconditionally [`SessionState::Unlocked`]
/// whenever `open` is `Some` — the W2-era stub this replaces always returned the latter.
#[derive(Debug)]
struct OpenSession {
    account_id: String,
    master: VaultMasterKey,
    degraded: bool,
    integrity_report: IntegrityReport,
    /// The audit head as this session currently believes it, ahead of what's persisted in
    /// the keystore by `appends_since_persist` entries (architecture §6.2). Set to the
    /// trusted head at unlock/create_account time, then advanced by
    /// `SessionManager::record_audit_append` after each successful append (W10 is the
    /// first chunk with a command — `import_document` — that appends anything).
    live_head: AuditHead,
    /// How many appends since `live_head` was last persisted to the keystore. Persisted
    /// (and reset to 0) at 32 — architecture §6.2's "every 32 appends" — and unconditionally
    /// on `lock` if nonzero.
    appends_since_persist: u32,
    /// Page IR + `raw_bytes` for documents imported this unlock (data-model §6.1: not in
    /// meta). Discard-path approval reads this; lock drops it with the rest of [`OpenSession`].
    pending_bodies: HashMap<String, Document>,
    /// One active approval session (design §2.3). `None` when idle.
    approval: Option<ApprovalSession>,
}

/// The in-process session and account command surface.
///
/// Not internally synchronized: `&mut self` on the mutating commands makes the state
/// machine's transitions exclusive by construction. W29 wraps one of these in a Tauri
/// managed `Mutex`.
pub struct SessionManager {
    keystore: Arc<dyn KeystoreBackend>,
    accounts: Arc<dyn AccountStore>,
    vault: Arc<dyn VaultBackend>,
    audit: Arc<dyn AuditStore>,
    config: Arc<dyn ConfigStore>,
    documents: Arc<dyn DocumentStore>,
    /// `None` → W15c per-detect selection between [`HybridV1`] and [`HybridOllamaV1`].
    /// [`SessionManager::with_detector`] installs an override so AC-1..AC-4 can keep using
    /// [`crate::detector::StubDetector`].
    detector_override: Option<Arc<dyn Detector>>,
    ollama_addr: SocketAddr,
    ollama_allowlist: Vec<AllowlistEntry>,
    progress: Arc<dyn ProgressSink>,
    open: Option<OpenSession>,
}

impl core::fmt::Debug for SessionManager {
    /// Renders the state and backend only — never key material, never a passphrase.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SessionManager")
            .field("state", &self.state())
            .field("keystore", &self.keystore.kind())
            .finish()
    }
}

impl SessionManager {
    /// Build a manager over a keystore backend and an account store, with **no** SQLCipher
    /// vault (`crate::vault::NullVault`): `create_account` / `unlock` / `lock` never touch
    /// a database. Kept for W2-era session-layer tests that predate W3 and do not care
    /// about vault persistence — see [`Self::new_with_vault`] for the W3+ wiring.
    ///
    /// Backend choice is the caller's: [`crate::keystore::OsKeystore`] in production,
    /// [`crate::keystore::FileKeystore`] as the Linux fallback (architecture §3.2),
    /// [`crate::keystore::InMemoryKeystore`] in tests. Probing which one applies is W7.
    #[must_use]
    pub fn new(keystore: Arc<dyn KeystoreBackend>, accounts: Arc<dyn AccountStore>) -> Self {
        Self::new_with_vault(keystore, accounts, Arc::new(NullVault))
    }

    /// Build a manager over a keystore backend, an account store, and a SQLCipher vault
    /// (W3), with **no** real audit store (`NullAuditStore`): every unlock replays an empty
    /// chain against `AuditHead::GENESIS`, which is always [`VerifyOutcome::Clean`] — so
    /// `unlock` behaves exactly as it did before W5. Kept for W3/W4-era tests that predate
    /// W5 and do not exercise the audit chain — see [`Self::new_with_vault_and_audit`] for
    /// the W5+ wiring. `accounts` and `vault` are typically the **same** object (e.g. an
    /// `Arc<crate::vault::SqlCipherVault>` coerced to each trait object) so that opening
    /// the vault and reading/writing the account row share one live connection —
    /// see `crate::vault` module docs.
    #[must_use]
    pub fn new_with_vault(
        keystore: Arc<dyn KeystoreBackend>,
        accounts: Arc<dyn AccountStore>,
        vault: Arc<dyn VaultBackend>,
    ) -> Self {
        Self::new_with_vault_and_audit(keystore, accounts, vault, Arc::new(NullAuditStore))
    }

    /// Build a manager over a keystore backend, an account store, a SQLCipher vault, and an
    /// audit store (W5). `accounts`, `vault`, and `audit` are typically all the **same**
    /// object — e.g. one `Arc<crate::vault::SqlCipherVault>` coerced to each of the three
    /// trait objects, so the vault connection, the account row, and the audit chain all
    /// live behind one live SQLCipher connection (`crate::vault` module docs).
    #[must_use]
    pub fn new_with_vault_and_audit(
        keystore: Arc<dyn KeystoreBackend>,
        accounts: Arc<dyn AccountStore>,
        vault: Arc<dyn VaultBackend>,
        audit: Arc<dyn AuditStore>,
    ) -> Self {
        Self::new_full(keystore, accounts, vault, audit, Arc::new(NullConfigStore))
    }

    /// Build a manager over every backend, including config storage (W6). `accounts`,
    /// `vault`, `audit`, and `config` are typically all the **same** object — one
    /// `Arc<crate::vault::SqlCipherVault>` coerced to each of the four trait objects, so
    /// every backend lives behind one live SQLCipher connection (`crate::vault` module
    /// docs).
    #[must_use]
    pub fn new_full(
        keystore: Arc<dyn KeystoreBackend>,
        accounts: Arc<dyn AccountStore>,
        vault: Arc<dyn VaultBackend>,
        audit: Arc<dyn AuditStore>,
        config: Arc<dyn ConfigStore>,
    ) -> Self {
        Self {
            keystore,
            accounts,
            vault,
            audit,
            config,
            documents: Arc::new(NullDocumentStore),
            detector_override: None,
            ollama_addr: OLLAMA_LOOPBACK_ADDR,
            ollama_allowlist: default_ollama_allowlist(),
            progress: Arc::new(NullProgressSink),
            open: None,
        }
    }

    /// Override the document-catalog backend (W10). Builder-style — going forward, a new
    /// backend gets a `with_x` method here instead of another `new_with_x_and_y`
    /// positional-argument constructor, so the constructor list stops growing with every
    /// chunk. `new`/`new_with_vault`/`new_with_vault_and_audit`/`new_full` all default this
    /// to [`NullDocumentStore`].
    #[must_use]
    pub fn with_documents(mut self, documents: Arc<dyn DocumentStore>) -> Self {
        self.documents = documents;
        self
    }

    /// Override the Detector backend. Production import uses W15c's per-detect selection
    /// (architecture §10.1.3) when this is unset. Tests that need the W12 stub (AC-1..AC-4)
    /// pass [`crate::detector::StubDetector`] here so model drift cannot hide a vault bug.
    #[must_use]
    pub fn with_detector(mut self, detector: Arc<dyn Detector>) -> Self {
        self.detector_override = Some(detector);
        self
    }

    /// Where `"auto"` probes Ollama (architecture §10.1.1). Defaults to
    /// [`OLLAMA_LOOPBACK_ADDR`]. Tests point this at an in-process mock.
    #[must_use]
    pub fn with_ollama_endpoint(
        mut self,
        addr: SocketAddr,
        allowlist: Vec<AllowlistEntry>,
    ) -> Self {
        self.ollama_addr = addr;
        self.ollama_allowlist = allowlist;
        self
    }

    /// Tests only: retarget the `"auto"` probe without rebuilding the session, so a
    /// per-detect (not per-unlock) flip can be asserted on a live manager.
    pub fn set_ollama_endpoint(&mut self, addr: SocketAddr, allowlist: Vec<AllowlistEntry>) {
        self.ollama_addr = addr;
        self.ollama_allowlist = allowlist;
    }

    /// Override the `pg://detect-progress` sink (W14). Defaults to a no-op; W29's Tauri
    /// shim will supply an emitter that flushes to the webview. In-process tests pass a
    /// recording sink.
    #[must_use]
    pub fn with_progress_sink(mut self, progress: Arc<dyn ProgressSink>) -> Self {
        self.progress = progress;
        self
    }

    fn emit_detect_progress(&self, doc_id: &str, fraction: f64, phase: DetectPhase) {
        self.progress.emit_detect_progress(DetectProgress {
            doc_id: doc_id.to_string(),
            fraction,
            phase,
        });
    }

    /// Which keystore backend is in use (architecture §3.2 recording requirement).
    #[must_use]
    pub fn keystore_kind(&self) -> KeystoreBackendKind {
        self.keystore.kind()
    }

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------

    fn state(&self) -> SessionState {
        if let Some(open) = &self.open {
            return if open.degraded {
                SessionState::DegradedIntegrity
            } else {
                SessionState::Unlocked
            };
        }
        // `first_run` is answered by the keystore alone, because it is the only thing
        // readable while locked. A *failure* to read must not be reported as `first_run`:
        // that would offer to create an account over a live vault and overwrite the only
        // copy of the wrapped master key. Fail safe towards `locked`.
        match self.keystore.load() {
            Ok(None) => SessionState::FirstRun,
            Ok(Some(_)) | Err(_) => SessionState::Locked,
        }
    }

    /// `get_session_state` — api.md §5.1. Callable in every state, including before first
    /// run (api.md §2).
    #[must_use]
    pub fn get_session_state(&self) -> SessionStateOut {
        SessionStateOut {
            state: self.state(),
        }
    }

    /// True while the session holds `vault_master_key` material.
    ///
    /// The structural half of the `lock` contract (dev-plan W2 "Done when: … lock zeroizes
    /// session key material"): after `lock()` the whole [`OpenSession`] is dropped, so
    /// this is `false` and there is no field left that could hand key material out. The
    /// behavioural half is that [`Self::sqlcipher_key`] and [`Self::audit_mac_key`] then
    /// return `not_in_session`.
    #[must_use]
    pub fn has_resident_key_material(&self) -> bool {
        self.open.is_some()
    }

    /// The raw SQLCipher key for the open vault (architecture §3.1). W3's DB layer calls
    /// this; W2 exposes it so the lock contract is testable through behaviour.
    ///
    /// # Errors
    /// `not_in_session` when the vault is not open.
    pub fn sqlcipher_key(&self) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
        Ok(self.require_open()?.master.sqlcipher_key())
    }

    /// The audit MAC key for the open vault (architecture §3.1). W5's audit trail calls
    /// this.
    ///
    /// # Errors
    /// `not_in_session` when the vault is not open.
    pub fn audit_mac_key(&self) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
        Ok(self.require_open()?.master.audit_mac_key())
    }

    fn require_open(&self) -> Result<&OpenSession, ApiError> {
        self.open.as_ref().ok_or_else(ApiError::not_in_session)
    }

    fn require_open_mut(&mut self) -> Result<&mut OpenSession, ApiError> {
        self.open.as_mut().ok_or_else(ApiError::not_in_session)
    }

    /// Append one audit row and advance the live head (architecture §6.1/§6.2). Every
    /// command that mutates vault content calls this after the mutation succeeds — W10's
    /// `import_document` is the first. Persists to the keystore immediately at the 32nd
    /// unpersisted append (architecture §6.2's other cadence trigger besides `lock`); a
    /// persist failure there is surfaced as `internal` — unlike `lock`'s best-effort
    /// persist, a command that just wrote content the caller is relying on being audited
    /// should not silently swallow a failure to record that fact.
    fn record_audit_append(
        &mut self,
        event_type: EventType,
        doc_id: Option<&str>,
        originals_flag: OriginalsFlag,
        payload_jcs: &str,
    ) -> Result<(), ApiError> {
        let mac_key = self.audit_mac_key()?;
        let produced_at_unix_ms = now_unix_ms();
        let row = crate::audit::append(
            self.audit.as_ref(),
            &mac_key,
            event_type,
            doc_id,
            produced_at_unix_ms,
            originals_flag,
            payload_jcs,
        )
        .map_err(map_audit_err)?;

        let head = crate::audit::head_for(&row);
        let open = self.open.as_mut().ok_or_else(ApiError::not_in_session)?;
        open.live_head = head;
        open.appends_since_persist += 1;

        if u64::from(open.appends_since_persist) >= crate::audit::CRASH_WINDOW_MAX {
            let item = self
                .keystore
                .load()
                .map_err(map_keystore_err)?
                .ok_or_else(|| ApiError::internal("keystore item is missing"))?;
            self.keystore
                .store(&KeystoreItem {
                    account_id: item.account_id,
                    kdf: item.kdf,
                    wrapped_master_key: item.wrapped_master_key,
                    audit_head: head,
                })
                .map_err(map_keystore_err)?;
            // Re-borrow: the keystore calls above needed `&self`, so the earlier `&mut`
            // borrow of `self.open` had to end first.
            if let Some(open) = self.open.as_mut() {
                open.appends_since_persist = 0;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Commands
    // -----------------------------------------------------------------------

    /// `create_account` — api.md §5.1; first-run flow, architecture §3.4.
    ///
    /// Generates `vault_master_key` from the CSPRNG, derives a `wrap_key` from the
    /// passphrase and a fresh salt, wraps the master key, writes the `KeystoreItem` and
    /// the `LocalAccount`, and leaves the session unlocked. No network step (OQ-5,
    /// architecture §7).
    ///
    /// # Errors
    /// - `account_exists` when the session is not `first_run` (api.md §3).
    /// - `invalid_input` for a passphrase under 8 chars or a `display_name` that is not
    ///   1..=80 chars after trimming.
    /// - `internal` if key generation or a store write fails — in which case nothing is
    ///   left behind (see the rollback below).
    pub fn create_account(&mut self, input: CreateAccountIn) -> Result<CreateAccountOut, ApiError> {
        // api.md §2 (SESSION_TABLE, W4): `create_account` is `first_run`-only. The
        // disallowed-state error is `account_exists`, not the generic `not_in_session` —
        // api.md §3 documents this command's specific code for exactly this cell.
        if !command_allowed("create_account", self.state()) {
            return Err(ApiError::account_exists());
        }

        let display_name = validate_display_name(&input.display_name)?;
        validate_passphrase(&input.passphrase)?;

        let master = VaultMasterKey::generate()
            .map_err(|_| ApiError::internal("could not generate vault key material"))?;
        let kdf = Argon2idParams::generate().map_err(map_keystore_err)?;
        let wrapped_master_key = wrap_master_key(&input.passphrase, &master, &kdf)
            .map_err(|_| ApiError::internal("could not wrap vault key material"))?;

        // W3: open (creating) the vault DB before anything is written to it or to the
        // keystore. `first_run` is answered by the keystore alone (`self.state()`, api.md
        // §2), so reaching this line already proves no keystore item exists — which means
        // there is no salt and no wrapped master key anywhere that could ever recover a
        // vault file already sitting at this path (architecture §3.1). Such a file can
        // only be an orphan from a previously aborted `create_account` (this process or an
        // earlier one that crashed between opening the vault and writing the keystore
        // item). `destroy()` it first so a wrong-key failure from a *stale* orphan never
        // makes this call — or any retry — fail forever; then open fresh.
        self.vault.destroy();
        if let Err(e) = self.vault.open(&master.sqlcipher_key()) {
            self.vault.destroy();
            return Err(map_vault_err(e));
        }

        let account = LocalAccount {
            id: new_account_id(),
            display_name,
            created_at: now_rfc3339(),
        };
        let item = KeystoreItem {
            account_id: account.id.clone(),
            kdf,
            wrapped_master_key,
            // data-model §5.9: no chain exists yet. W5 replaces this with the real head.
            audit_head: AuditHead::GENESIS,
        };

        // Order matters. The account record goes first because the *keystore item* is
        // what flips `first_run` → `locked`: writing it last means the session is only
        // ever advertised as having an account once everything it needs is present. If
        // the keystore write then fails, roll the account record back so a retry sees a
        // clean `first_run` rather than a half-created account. `destroy()`, not `close()`,
        // on both paths below — for the same reason as above: nothing this call created is
        // recoverable without the keystore item it just failed to commit, so leaving the
        // file behind would only reproduce the same wedge on the next attempt.
        if let Err(e) = self.accounts.store(&account) {
            self.vault.destroy();
            return Err(map_account_err(e));
        }
        if let Err(e) = self.keystore.store(&item) {
            let _ = self.accounts.delete();
            self.vault.destroy();
            return Err(map_keystore_err(e));
        }

        let account_id = account.id.clone();
        self.open = Some(OpenSession {
            account_id: account.id,
            master,
            degraded: false,
            // A fresh vault has no chain to verify — this is trivially the same "T == H"
            // clean outcome unlock reports on an empty replay against `AuditHead::GENESIS`
            // (architecture §6.3), stated directly rather than routed through a replay of
            // zero rows.
            integrity_report: IntegrityReport {
                ok: true,
                kind: "ok".to_string(),
                head_sequence: 0,
                tail_sequence: 0,
                first_bad_sequence: None,
            },
            live_head: AuditHead::GENESIS,
            appends_since_persist: 0,
            pending_bodies: HashMap::new(),
            approval: None,
        });
        Ok(CreateAccountOut {
            account_id,
            state: SessionState::Unlocked,
        })
    }

    /// `unlock` — api.md §5.1; architecture §3.3.
    ///
    /// Loads the `KeystoreItem`, derives `wrap_key` from the passphrase and the stored
    /// Argon2id parameters, and unwraps `vault_master_key`. On success the subkeys become
    /// derivable ([`Self::sqlcipher_key`], [`Self::audit_mac_key`]).
    ///
    /// architecture §3.3: "Passphrase failure zeroizes and refuses (no partial open)" —
    /// on failure nothing is stored on `self`, and the `wrap_key` and any partial
    /// plaintext are dropped (and zeroized) inside `unwrap_master_key`.
    ///
    /// # Errors
    /// - `not_in_session` unless the state is `locked` (api.md §2 table).
    /// - `unlock_failed` for a wrong passphrase **and** for a missing keystore item —
    ///   api.md §3 makes them indistinguishable so `unlock` is not an enumeration oracle.
    pub fn unlock(&mut self, input: UnlockIn) -> Result<UnlockOut, ApiError> {
        // api.md §2 (SESSION_TABLE, W4): `locked`-only.
        if !command_allowed("unlock", self.state()) {
            return Err(ApiError::not_in_session());
        }

        // A read failure is not "no account" — but it is also not something to describe
        // to the caller in an unlock error, so it collapses into `unlock_failed` too.
        let item = match self.keystore.load() {
            Ok(Some(item)) => item,
            Ok(None) | Err(_) => return Err(ApiError::unlock_failed()),
        };

        let master = unwrap_master_key(&input.passphrase, &item).ok_or_else(ApiError::unlock_failed)?;

        // W3: the passphrase is verified — open the vault on the recovered key. A
        // failure here is not a passphrase problem (architecture §3.3's "Passphrase
        // failure zeroizes and refuses" already happened above), so it is `internal`,
        // not `unlock_failed` — do not turn a corrupt/missing DB into an
        // account-enumeration-shaped error.
        self.vault.open(&master.sqlcipher_key()).map_err(map_vault_err)?;

        // architecture §3.3/§6.3: "verif[y] the audit chain against `audit_head`."
        let (open_session, out_state, out_integrity, head_to_persist) =
            self.verify_integrity_on_unlock(&item, master);

        if let Some(new_head) = head_to_persist {
            // architecture §6.2: "Fast-forward audit_head to T" — persisted immediately
            // rather than deferred to the next `lock`, so a second crash before any new
            // append cannot widen the same gap further, and `get_integrity_report`
            // reflects reality on the very next call.
            //
            // On a store failure, close the vault before propagating the error: `open_session`
            // is a local we simply drop here (never installed into `self.open`), but the
            // vault was already opened above and nothing else on this path closes it —
            // leaving it open would mean `state()` reports `Locked` while the SQLCipher
            // connection is still live underneath, the exact "partial open" architecture
            // §3.3 forbids ("Passphrase failure zeroizes and refuses (no partial open)").
            if let Err(e) = self.keystore.store(&KeystoreItem {
                account_id: item.account_id,
                kdf: item.kdf,
                wrapped_master_key: item.wrapped_master_key,
                audit_head: new_head,
            }) {
                self.vault.close();
                return Err(map_keystore_err(e));
            }
        }

        self.open = Some(open_session);
        Ok(UnlockOut {
            state: out_state,
            integrity: out_integrity,
        })
    }

    /// architecture §6.3's three outcomes, replayed and classified. Returns the
    /// [`OpenSession`] to install, the `SessionState`/`integrity` pair `unlock` reports,
    /// and — only on a crash-window fast-forward — the new [`AuditHead`] the caller must
    /// persist.
    ///
    /// Takes `master` by value: it either moves into the returned [`OpenSession`], or — on
    /// a failure this function classifies as degraded — moves in anyway (architecture
    /// §3.3's "hold `vault_master_key` only far enough to verify the chain and serve the
    /// report" is exactly the degraded session's scope, not a reason to drop it here).
    fn verify_integrity_on_unlock(
        &self,
        item: &KeystoreItem,
        master: VaultMasterKey,
    ) -> (OpenSession, SessionState, Option<IntegrityReport>, Option<AuditHead>) {
        let mac_key = master.audit_mac_key();
        // A replay failure (corrupt row, backend I/O error) is itself evidence the chain
        // cannot be trusted — treat it the same as a verification failure rather than
        // letting an `Err` here accidentally fall through to "clean."
        let outcome = match self.audit.replay() {
            Ok(rows) => crate::audit::verify_against_head(&rows, &mac_key, item.audit_head),
            Err(_) => VerifyOutcome::Failure {
                kind: FailureKind::Modification,
                first_bad_sequence: None,
                verified_tail_sequence: 0,
            },
        };

        let (degraded, report, head_to_persist) = match outcome {
            VerifyOutcome::Clean => (
                false,
                IntegrityReport {
                    ok: true,
                    kind: "ok".to_string(),
                    head_sequence: item.audit_head.sequence,
                    tail_sequence: item.audit_head.sequence,
                    first_bad_sequence: None,
                },
                None,
            ),
            VerifyOutcome::FastForward { new_head } => (
                false,
                IntegrityReport {
                    ok: true,
                    kind: "crash_window_fast_forwarded".to_string(),
                    head_sequence: item.audit_head.sequence,
                    tail_sequence: new_head.sequence,
                    first_bad_sequence: None,
                },
                Some(new_head),
            ),
            VerifyOutcome::Failure {
                kind,
                first_bad_sequence,
                verified_tail_sequence,
            } => (
                true,
                IntegrityReport {
                    ok: false,
                    kind: match kind {
                        FailureKind::Truncation => "truncation".to_string(),
                        FailureKind::Modification => "modification".to_string(),
                    },
                    head_sequence: item.audit_head.sequence,
                    tail_sequence: verified_tail_sequence,
                    first_bad_sequence,
                },
                None,
            ),
        };

        let out_state = if degraded {
            SessionState::DegradedIntegrity
        } else {
            SessionState::Unlocked
        };
        // api.md §5.1: "`integrity` is non-null iff `degraded_integrity`" — a clean unlock
        // or a fast-forward both report `state: "unlocked"` with `integrity: null`; the
        // (still-`ok`) report is still available afterward through `get_integrity_report`.
        let out_integrity = if degraded { Some(report.clone()) } else { None };

        // Clean → the persisted head itself (T == H); FastForward → the newly-trusted T
        // (also what's about to be persisted); Failure → unchanged, since nothing new was
        // trusted. `head_to_persist` already encodes exactly the FastForward case, so this
        // is the one place all three outcomes agree without re-deriving the match.
        let live_head = head_to_persist.unwrap_or(item.audit_head);

        let open_session = OpenSession {
            account_id: item.account_id.clone(),
            master,
            degraded,
            integrity_report: report,
            live_head,
            appends_since_persist: 0,
            pending_bodies: HashMap::new(),
            approval: None,
        };

        (open_session, out_state, out_integrity, head_to_persist)
    }

    /// `lock` — api.md §5.1; architecture §3.3.
    ///
    /// > **Lock:** zeroize master key, wrap key, DEKs, decrypted artifact caches, and the
    /// > SQLCipher connection key; close the DB.
    ///
    /// Implemented by dropping the entire [`OpenSession`]: `VaultMasterKey` is
    /// `ZeroizeOnDrop`, and everything later chunks add (the DB handle, DEK caches,
    /// approval sessions, preview tokens) lives in the same struct and dies with it.
    /// After this returns, no field of `SessionManager` holds key material at all.
    ///
    /// # Errors
    /// `not_in_session` unless the vault is open (api.md §2 table).
    pub fn lock(&mut self) -> Result<LockOut, ApiError> {
        // api.md §2 (SESSION_TABLE, W4): `unlocked` or `degraded_integrity`.
        if !command_allowed("lock", self.state()) {
            return Err(ApiError::not_in_session());
        }

        // architecture §6.2: "Persist to the keystore... on: lock". Best-effort: a failure
        // here does not fail `lock` itself (api.md documents no error variant for it beyond
        // `not_in_session`) — the crash-window fast-forward at the next unlock exists
        // precisely to tolerate a missed persist, as long as fewer than
        // `audit::CRASH_WINDOW_MAX` appends happened since the last successful one.
        if let Some(open) = &self.open {
            if open.appends_since_persist > 0 {
                if let Ok(Some(item)) = self.keystore.load() {
                    let _ = self.keystore.store(&KeystoreItem {
                        account_id: item.account_id,
                        kdf: item.kdf,
                        wrapped_master_key: item.wrapped_master_key,
                        audit_head: open.live_head,
                    });
                }
            }
        }

        // data-model §8: lock while discard and not approved deletes the catalog row
        // (the RAM body is about to go with `OpenSession`). Retain unapproved rows stay.
        if self.open.is_some() {
            self.drop_unapproved_discards()?;
        }

        // Explicit drop rather than letting it fall out of scope, so the destruction is
        // the visible effect of the command.
        drop(self.open.take());
        // W3: "close the DB" (architecture §3.3's lock contract) — the SQLCipher
        // connection, and whatever key material SQLCipher itself holds, goes with it.
        self.vault.close();
        // architecture §10.2: unload in-process NER on lock. Production selection
        // constructs hosts per-detect (nothing resident); an override may hold weights.
        if let Some(detector) = &self.detector_override {
            detector.on_lock();
        }
        debug_assert!(!self.has_resident_key_material());
        Ok(LockOut {
            state: SessionState::Locked,
        })
    }

    /// `change_passphrase` — api.md §5.1; architecture §3.3.
    ///
    /// > **Change passphrase (v1):** re-derive a new `wrap_key` from the new passphrase
    /// > and a new salt; re-wrap the **same** `vault_master_key`. DEKs and ciphertext do
    /// > not rotate. This is KEK rotation, not master-key rotation.
    ///
    /// The `audit_head` is carried across unchanged — resetting it would silently defeat
    /// W5's anti-truncation check (architecture §6.2).
    ///
    /// # Errors
    /// - `not_in_session` unless the vault is open.
    /// - `invalid_input` if the new passphrase is under 8 chars. Checked **before** the
    ///   current one, so a rejected new passphrase never doubles as an oracle for the
    ///   current one.
    /// - `passphrase_mismatch` if `current` is wrong. Nothing is written on this path:
    ///   the stored item stays byte-identical.
    pub fn change_passphrase(
        &mut self,
        input: ChangePassphraseIn,
    ) -> Result<ChangePassphraseOut, ApiError> {
        // api.md §2 (SESSION_TABLE, W4): `unlocked` only — unlike `lock`/`get_account`,
        // `degraded_integrity` is explicitly "no" here (api.md §2 table).
        if !command_allowed("change_passphrase", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        validate_passphrase(&input.new_passphrase)?;

        let item = self
            .keystore
            .load()
            .map_err(map_keystore_err)?
            .ok_or_else(|| ApiError::internal("keystore item is missing"))?;

        // Verify `current` against what is actually stored, not against the live session:
        // the stored item is what the new wrap has to replace, and this is the only thing
        // that proves the caller knows the passphrase rather than merely holding an open
        // session.
        let recovered =
            unwrap_master_key(&input.current, &item).ok_or_else(ApiError::passphrase_mismatch)?;
        // Defence in depth: the item must be wrapping the master key this session is
        // actually using. A mismatch means the keystore was swapped underneath us.
        if !recovered.ct_eq(&open.master) {
            return Err(ApiError::internal("keystore item does not match the session"));
        }

        let kdf = Argon2idParams::generate().map_err(map_keystore_err)?;
        let wrapped_master_key = wrap_master_key(&input.new_passphrase, &open.master, &kdf)
            .map_err(|_| ApiError::internal("could not wrap vault key material"))?;

        self.keystore
            .store(&KeystoreItem {
                account_id: item.account_id,
                kdf,
                wrapped_master_key,
                // KEK rotation only: the chain head is untouched (architecture §6.2).
                audit_head: item.audit_head,
            })
            .map_err(map_keystore_err)?;

        // The session keeps the same `vault_master_key`, so the vault stays open and
        // every DEK and ciphertext remains valid.
        Ok(ChangePassphraseOut { ok: true })
    }

    /// `get_account` — api.md §5.1.
    ///
    /// `unlocked` or `degraded_integrity` (api.md §2 SESSION_TABLE, W4): the
    /// `LocalAccount` record lives inside the vault (data-model §5.6). Local-only, no
    /// network identity (architecture §7). api.md §2 notes the degraded cell returns
    /// "id + display_name only" — W2/W3 always return exactly those fields regardless of
    /// state, so there is nothing extra to withhold once W5 makes that state reachable.
    ///
    /// # Errors
    /// - `not_in_session` unless the vault is open.
    /// - `internal` if the record is missing behind an open session.
    pub fn get_account(&self) -> Result<GetAccountOut, ApiError> {
        // api.md §2 (SESSION_TABLE, W4).
        if !command_allowed("get_account", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let account = self
            .accounts
            .load()
            .map_err(map_account_err)?
            .filter(|a| a.id == open.account_id)
            .ok_or_else(|| ApiError::internal("account record is missing"))?;
        Ok(GetAccountOut {
            account_id: account.id,
            display_name: account.display_name,
            created_at: account.created_at,
        })
    }

    /// `get_integrity_report` — api.md §5.1; architecture §6.3 (W5).
    ///
    /// `unlocked` or `degraded_integrity` (api.md §2 SESSION_TABLE, W4/W5). Returns the
    /// [`IntegrityReport`] resolved once at `unlock`/`create_account` time — "Done when:
    /// `get_integrity_report` matches unlock outcome" (dev-plan W5) is exactly this: no
    /// re-replay here, just the report the session already settled on.
    ///
    /// # Errors
    /// `not_in_session` unless the vault is open.
    pub fn get_integrity_report(&self) -> Result<IntegrityReport, ApiError> {
        if !command_allowed("get_integrity_report", self.state()) {
            return Err(ApiError::not_in_session());
        }
        Ok(self.require_open()?.integrity_report.clone())
    }

    /// `get_retention_default` — api.md §5.2; decision 0007.
    ///
    /// `unlocked`-only (api.md §2's generic config row; C-API-6: unlike `get_account`/
    /// `get_integrity_report`, config is not available while degraded). A missing config
    /// row (should not happen once `create_account` has run) reads as
    /// [`Config::default`]'s factory values rather than an error — the same fail-open-to-
    /// factory posture `crate::keystore` uses for `first_run`.
    ///
    /// # Errors
    /// `not_in_session` unless the vault is open (and not degraded).
    pub fn get_retention_default(&self) -> Result<RetentionDefaultOut, ApiError> {
        if !command_allowed("get_retention_default", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let config = self
            .config
            .load(&open.master)
            .map_err(map_config_err)?
            .unwrap_or_default();
        Ok(RetentionDefaultOut {
            policy: config.policy,
            confirmed: config.confirmed,
        })
    }

    /// `set_retention_default` — api.md §5.2; decision 0007.
    ///
    /// Sets the global default and marks it confirmed. api.md §5.2: "Changing the
    /// **global** default from `never_retain` to `retain` is allowed (it is not a
    /// per-import override)" — this command has no paranoid-loosening restriction; that
    /// restriction is `import_document`'s per-import override (W11), not this command's.
    ///
    /// # Errors
    /// `not_in_session` unless the vault is open (and not degraded).
    pub fn set_retention_default(
        &mut self,
        input: SetRetentionDefaultIn,
    ) -> Result<RetentionDefaultOut, ApiError> {
        if !command_allowed("set_retention_default", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let mut config = self
            .config
            .load(&open.master)
            .map_err(map_config_err)?
            .unwrap_or_default();
        config.policy = input.policy;
        config.confirmed = true;
        self.config
            .store(&open.master, &config)
            .map_err(map_config_err)?;
        Ok(RetentionDefaultOut {
            policy: config.policy,
            confirmed: true,
        })
    }

    /// `get_detector_preference` — api.md §5.2; decision 0009.
    ///
    /// Factory `"auto"` (data-model §5.5). Unlocked-only, same generic config row as
    /// retention (C-API-6: unavailable while degraded).
    ///
    /// # Errors
    /// `not_in_session` unless the vault is open (and not degraded).
    pub fn get_detector_preference(&self) -> Result<DetectorPreferenceOut, ApiError> {
        if !command_allowed("get_detector_preference", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let config = self
            .config
            .load(&open.master)
            .map_err(map_config_err)?
            .unwrap_or_default();
        Ok(DetectorPreferenceOut {
            preference: config.detector_preference,
        })
    }

    /// `set_detector_preference` — api.md §5.2; decision 0009.
    ///
    /// Does not confirm retention. `"auto"` | `"bundled_only"` only — a third value is a
    /// type error (`DetectorPreference`), not a runtime string.
    ///
    /// # Errors
    /// `not_in_session` unless the vault is open (and not degraded).
    pub fn set_detector_preference(
        &mut self,
        input: SetDetectorPreferenceIn,
    ) -> Result<DetectorPreferenceOut, ApiError> {
        if !command_allowed("set_detector_preference", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let mut config = self
            .config
            .load(&open.master)
            .map_err(map_config_err)?
            .unwrap_or_default();
        config.detector_preference = input.preference;
        self.config
            .store(&open.master, &config)
            .map_err(map_config_err)?;
        Ok(DetectorPreferenceOut {
            preference: config.detector_preference,
        })
    }

    /// architecture §10.1.3: per-detect selection. Not cached on the session.
    fn detect_for_import(
        &self,
        doc: &crate::importer::Document,
        doc_id: &str,
        preference: DetectorPreference,
    ) -> DetectionRun {
        if let Some(detector) = &self.detector_override {
            self.emit_detect_progress(doc_id, 0.0, DetectPhase::Detecting);
            let fields = detector.detect(doc);
            self.emit_detect_progress(doc_id, 1.0, DetectPhase::Detecting);
            return DetectionRun {
                fields,
                detector_id: detector.id(),
                backend: None,
                model_tag: None,
                fallback_reason: None,
            };
        }
        match preference {
            DetectorPreference::BundledOnly => self.detect_hybrid(doc, doc_id, None),
            DetectorPreference::Auto => self.detect_auto(doc, doc_id),
        }
    }

    fn detect_hybrid(
        &self,
        doc: &crate::importer::Document,
        doc_id: &str,
        fallback_reason: Option<FallbackReason>,
    ) -> DetectionRun {
        self.emit_detect_progress(doc_id, 0.0, DetectPhase::Detecting);
        let fields = HybridV1::bundled().detect(doc);
        self.emit_detect_progress(doc_id, 1.0, DetectPhase::Detecting);
        DetectionRun {
            fields,
            detector_id: HYBRID_V1_ID,
            backend: Some("onnx"),
            model_tag: None,
            fallback_reason: fallback_reason.map(FallbackReason::as_str),
        }
    }

    fn detect_auto(&self, doc: &crate::importer::Document, doc_id: &str) -> DetectionRun {
        let client = match OllamaClient::connect(self.ollama_addr, self.ollama_allowlist.clone()) {
            Ok(c) => c,
            Err(reason) => return self.detect_hybrid(doc, doc_id, Some(reason)),
        };
        match client.handshake() {
            Err(reason) => self.detect_hybrid(doc, doc_id, Some(reason)),
            Ok(_) => {
                self.emit_detect_progress(doc_id, 0.0, DetectPhase::WarmingModel);
                self.emit_detect_progress(doc_id, 0.0, DetectPhase::Detecting);
                let outcome = HybridOllamaV1::new(client).detect_with_outcome(doc);
                match outcome.fallback_reason {
                    Some(reason) => {
                        let fields = HybridV1::bundled().detect(doc);
                        self.emit_detect_progress(doc_id, 1.0, DetectPhase::Detecting);
                        DetectionRun {
                            fields,
                            detector_id: HYBRID_V1_ID,
                            backend: Some("onnx"),
                            model_tag: None,
                            fallback_reason: Some(reason.as_str()),
                        }
                    }
                    None => {
                        self.emit_detect_progress(doc_id, 1.0, DetectPhase::Detecting);
                        DetectionRun {
                            fields: outcome.fields,
                            detector_id: HYBRID_OLLAMA_V1_ID,
                            backend: Some("ollama"),
                            model_tag: outcome.model_tag,
                            fallback_reason: None,
                        }
                    }
                }
            }
        }
    }

    /// `import_document` — api.md §5.3; FR-1.1–1.5.
    ///
    /// Extraction (`crate::importer`), catalog storage, W11 retention gates, and W15c
    /// per-detect backend selection (architecture §10.1.3). `with_detector` overrides
    /// selection so AC-1..AC-4 can keep the stub.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked (not degraded — C-API-6).
    /// - `invalid_input` for an empty filename or one containing a path separator.
    /// - `unsupported_document` for empty bytes, non-UTF-8 "text", a PDF `pdf-extract`
    ///   can't parse, or a PDF with no extractable text.
    pub fn import_document(&mut self, input: ImportDocumentIn) -> Result<ImportDocumentOut, ApiError> {
        if !command_allowed("import_document", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let filename = validate_import_filename(&input.filename)?;
        let over_budget = input.bytes.len() > IMPORT_BUDGET_BYTES;

        // W11 / decision 0007 / AC-7: read Config **before** touching the Importer or
        // Detector at all — "no detection, no catalog row" when the policy isn't
        // confirmed, not merely "no catalog row after detecting anyway."
        //
        // `&self.require_open()?.master` is re-borrowed at each call site below rather
        // than bound to a local: `VaultMasterKey` deliberately has no `Clone`
        // (`crate::keys` — every copy is another place key material must be destroyed),
        // and `record_audit_append` further down needs `&mut self`, which a
        // held-across-statements borrow of `self.open` would conflict with. Each
        // `self.require_open()?.master` below is its own short-lived borrow instead.
        let config = self
            .config
            .load(&self.require_open()?.master)
            .map_err(map_config_err)?
            .unwrap_or_default();

        // decision 0007 / AC-7: "No import succeeds until the policy is confirmed."
        if !config.confirmed {
            return Err(ApiError::retention_policy_unset());
        }

        // AC-6 / api.md §5.3: "never_retain default + retention_override: retain →
        // retention_loosen_forbidden." A per-import *discard* against never_retain is
        // tightening, not loosening, and is allowed — only the retain direction is
        // forbidden.
        if config.policy == RetentionPolicy::NeverRetain
            && input.retention_override == Some(EffectiveRetention::Retain)
        {
            return Err(ApiError::retention_loosen_forbidden());
        }

        // data-model §6.1: "Global policy never_retain... is not stored on the document.
        // Import under never_retain always writes retention: discard here." Otherwise the
        // per-import override, if given, else the default policy itself.
        let retention = match input.retention_override {
            Some(effective) => effective,
            None => match config.policy {
                RetentionPolicy::Retain => EffectiveRetention::Retain,
                RetentionPolicy::Discard | RetentionPolicy::NeverRetain => EffectiveRetention::Discard,
            },
        };

        // Format switch (dev-plan W10 "Integrate: Importer format switch") — sniff
        // content, not the filename: FR-1.1/1.2's "extractable text" is a property of the
        // bytes, and trusting a caller-supplied name would let a mislabeled file bypass
        // the PDF-vs-text extraction path meant for its real content.
        let doc_id = uuid::Uuid::new_v4().to_string();
        let is_pdf = input.bytes.starts_with(b"%PDF-");
        let doc = if is_pdf {
            importer::import_pdf(&input.bytes, &doc_id).map_err(|_| unsupported_document())?
        } else {
            importer::import_text(&input.bytes, &doc_id).map_err(|_| unsupported_document())?
        };

        // design §2.2: "Run the on-device detection model... produce a list of classified
        // fields." Production selection is W15c; `with_detector` keeps the W12 stub for
        // AC-1..AC-4. `field_ids`/`labels` are captured before `detected_fields` moves
        // into `meta`, for the audit `detect` event below.
        //
        // W14 / api.md §6: emit `pg://detect-progress` around detect, never 1.0 before
        // `detect` returns. `phase: "warming_model"` only after a successful Ollama
        // handshake (architecture §10.1.5); `"bundled_only"` and handshake failures stay
        // on `"detecting"`.
        let run = self.detect_for_import(&doc, &doc_id, config.detector_preference);
        let detected_fields = run.fields;
        let detected_field_count = detected_fields.len() as u32;
        let field_ids: Vec<String> = detected_fields.iter().map(|f| f.id.clone()).collect();
        let labels: Vec<String> = detected_fields.iter().map(|f| f.label.clone()).collect();
        let imported_at_unix_ms = now_unix_ms();

        let meta = DocumentMeta {
            source_filename: filename.to_string(),
            source_format: doc.source_format,
            imported_at_unix_ms,
            retention,
            detected_fields,
        };
        let original = matches!(retention, EffectiveRetention::Retain)
            .then(|| OriginalRecord::new(doc.source_format, &doc.raw_bytes));

        self.documents
            .insert(&self.require_open()?.master, &doc_id, &meta, original.as_ref(), imported_at_unix_ms)
            .map_err(map_catalog_err)?;

        let import_payload = serde_json::json!({
            "retention": retention.as_str(),
            "source_filename": filename,
            // data-model §5.8.1: "detector_id (null on import row)" — the `detect` event
            // below carries it.
            "detector_id": null,
        });
        let import_payload_jcs =
            serde_json::to_string(&import_payload).map_err(|_| ApiError::internal("audit payload encode failed"))?;
        self.record_audit_append(EventType::Import, Some(&doc_id), OriginalsFlag::Unset, &import_payload_jcs)?;

        // design §2.2: "Emit a detect event to the Audit Trail with the detected fields
        // and classifications." data-model §5.8.1: which identity actually ran, plus
        // `backend`/`model_tag`/`fallback_reason` so a fallback is never hidden.
        let detect_payload = serde_json::json!({
            "detector_id": run.detector_id,
            "field_ids": field_ids,
            "labels": labels,
            "backend": run.backend,
            "model_tag": run.model_tag,
            "fallback_reason": run.fallback_reason,
        });
        let detect_payload_jcs =
            serde_json::to_string(&detect_payload).map_err(|_| ApiError::internal("audit payload encode failed"))?;
        self.record_audit_append(EventType::Detect, Some(&doc_id), OriginalsFlag::Unset, &detect_payload_jcs)?;

        if let Some(open) = self.open.as_mut() {
            open.pending_bodies.insert(doc_id.clone(), doc);
        }

        let summary = DocumentSummary {
            doc_id: doc_id.clone(),
            source_filename: meta.source_filename,
            source_format: meta.source_format,
            imported_at: crate::account::format_rfc3339((imported_at_unix_ms / 1000) as i64),
            retention,
            has_approved_version: self.documents.has_approved_version(&doc_id).map_err(map_catalog_err)?,
            has_retained_original: self.documents.has_retained_original(&doc_id).map_err(map_catalog_err)?,
            detected_field_count,
        };

        Ok(ImportDocumentOut { summary, over_budget })
    }

    /// `list_documents` — api.md §5.3. Newest import first.
    ///
    /// # Errors
    /// `not_in_session` unless unlocked.
    pub fn list_documents(&self) -> Result<ListDocumentsOut, ApiError> {
        if !command_allowed("list_documents", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let ids = self.documents.list_ids_newest_first().map_err(map_catalog_err)?;
        let documents = ids
            .into_iter()
            .map(|doc_id| self.summarize(&open.master, doc_id))
            .collect::<Result<Vec<_>, ApiError>>()?;
        Ok(ListDocumentsOut { documents })
    }

    /// `get_document` — api.md §5.3. No pages, no field text — `DocumentSummary` only.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked.
    /// - `not_found` if `doc_id` doesn't exist.
    pub fn get_document(&self, input: GetDocumentIn) -> Result<GetDocumentOut, ApiError> {
        if !command_allowed("get_document", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let summary = self.summarize(&open.master, input.doc_id)?;
        Ok(GetDocumentOut { summary })
    }

    /// `open_approval` — api.md §5.4; design §2.3; C-DES-1 / C-API-2.
    ///
    /// One RAM session per process. Abort/lock catalog deletion is W19.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked (not degraded — C-API-6).
    /// - `not_found` if `doc_id` is unknown or the in-memory body is gone.
    /// - `already_approved` if a canonical `ApprovedVersion` already exists.
    /// - `approval_busy` if another approval session is already active.
    pub fn open_approval(&mut self, input: OpenApprovalIn) -> Result<ApprovalView, ApiError> {
        if !command_allowed("open_approval", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let meta = self
            .documents
            .load_meta(&self.require_open()?.master, &input.doc_id)
            .map_err(map_catalog_err)?
            .ok_or_else(ApiError::not_found)?;
        if self
            .documents
            .has_approved_version(&input.doc_id)
            .map_err(map_catalog_err)?
        {
            return Err(ApiError::already_approved());
        }
        if self.require_open()?.approval.is_some() {
            return Err(ApiError::approval_busy());
        }
        let document = self.approval_document(&input.doc_id)?;
        let open = self.require_open_mut()?;
        if open.approval.is_some() {
            return Err(ApiError::approval_busy());
        }
        open.pending_bodies
            .entry(input.doc_id.clone())
            .or_insert_with(|| document.clone());
        let session = ApprovalSession {
            approval_session_id: uuid::Uuid::new_v4().to_string(),
            doc_id: input.doc_id,
            lifecycle: ApprovalLifecycle::AwaitingDecisions,
            document,
            fields: meta.detected_fields,
            decisions: HashMap::new(),
        };
        let view = session.to_view();
        open.approval = Some(session);
        Ok(view)
    }

    /// `get_approval_view` — api.md §5.4. Same payload as `open_approval`; `lifecycle`
    /// may be `awaiting_decisions` | `decided`.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked.
    /// - `not_found` if `approval_session_id` is not the active session.
    /// - `approval_bad_state` if the session is committed or aborted (W18/W19).
    pub fn get_approval_view(&self, input: GetApprovalViewIn) -> Result<ApprovalView, ApiError> {
        if !command_allowed("get_approval_view", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open()?;
        let session = open.approval.as_ref().ok_or_else(ApiError::not_found)?;
        if session.approval_session_id != input.approval_session_id {
            return Err(ApiError::not_found());
        }
        if matches!(
            session.lifecycle,
            ApprovalLifecycle::Committed | ApprovalLifecycle::Aborted
        ) {
            return Err(ApiError::approval_bad_state());
        }
        Ok(session.to_view())
    }

    /// `set_field_decisions` — api.md §5.4. Partial updates allowed; `lifecycle` is
    /// `"decided"` iff every detected field has a decision.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked.
    /// - `not_found` if `approval_session_id` is not the active session.
    /// - `invalid_input` if a `field_id` is not in this session.
    /// - `approval_bad_state` if the session is committed or aborted (W18/W19).
    pub fn set_field_decisions(
        &mut self,
        input: SetFieldDecisionsIn,
    ) -> Result<SetFieldDecisionsOut, ApiError> {
        if !command_allowed("set_field_decisions", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let open = self.require_open_mut()?;
        let session = open.approval.as_mut().ok_or_else(ApiError::not_found)?;
        if session.approval_session_id != input.approval_session_id {
            return Err(ApiError::not_found());
        }
        if matches!(
            session.lifecycle,
            ApprovalLifecycle::Committed | ApprovalLifecycle::Aborted
        ) {
            return Err(ApiError::approval_bad_state());
        }
        let known: HashSet<&str> = session.fields.iter().map(|f| f.id.as_str()).collect();
        for d in &input.decisions {
            if !known.contains(d.field_id.as_str()) {
                return Err(ApiError::invalid_input("unknown field_id"));
            }
        }
        for d in input.decisions {
            session.decisions.insert(d.field_id, d.decision);
        }
        session.refresh_lifecycle();
        Ok(SetFieldDecisionsOut {
            lifecycle: session.lifecycle,
            unresolved_field_ids: session.unresolved_field_ids(),
        })
    }

    /// `submit_approval` — api.md §5.4; FR-3.1–3.2; data-model §6.3 / §8.
    ///
    /// Requires `lifecycle == decided`. Writes the canonical kind=1 `ApprovedVersion`
    /// (overlap rule applied core-side). Drops the RAM session and `pending_bodies` entry
    /// on Vault ack (design §2.1). Discard never had kind=2; retain leaves it.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked (not degraded — C-API-6).
    /// - `not_found` if `approval_session_id` is not the active session.
    /// - `approval_bad_state` unless `lifecycle == decided`.
    /// - `already_approved` if a canonical version is already stored (C-DM-4).
    pub fn submit_approval(&mut self, input: SubmitApprovalIn) -> Result<SubmitApprovalOut, ApiError> {
        if !command_allowed("submit_approval", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let (doc_id, document, fields, decisions) = {
            let open = self.require_open()?;
            let session = open.approval.as_ref().ok_or_else(ApiError::not_found)?;
            if session.approval_session_id != input.approval_session_id {
                return Err(ApiError::not_found());
            }
            if session.lifecycle != ApprovalLifecycle::Decided {
                return Err(ApiError::approval_bad_state());
            }
            (
                session.doc_id.clone(),
                session.document.clone(),
                session.fields.clone(),
                session.decisions.clone(),
            )
        };
        if self
            .documents
            .has_approved_version(&doc_id)
            .map_err(map_catalog_err)?
        {
            return Err(ApiError::already_approved());
        }

        let snapshot: Vec<FieldDecision> = fields
            .iter()
            .map(|f| {
                let decision = *decisions.get(&f.id).ok_or_else(ApiError::approval_bad_state)?;
                Ok(FieldDecision {
                    field: f.clone(),
                    decision,
                })
            })
            .collect::<Result<_, ApiError>>()?;
        let redacted_content = crate::overlap::redact_document(&document, &fields, &decisions);
        let approved = ApprovedVersion {
            produced_at_unix_ms: now_unix_ms(),
            decisions: snapshot,
            redacted_content,
        };
        self.documents
            .store_approved(&self.require_open()?.master, &doc_id, &approved)
            .map_err(map_catalog_err)?;

        let audit_decisions: Vec<serde_json::Value> = fields
            .iter()
            .map(|f| {
                serde_json::json!({
                    "field_id": f.id,
                    "label": f.label,
                    "decision": decisions[&f.id],
                })
            })
            .collect();
        let payload = serde_json::json!({ "decisions": audit_decisions });
        let payload_jcs = serde_json::to_string(&payload)
            .map_err(|_| ApiError::internal("audit payload encode failed"))?;
        self.record_audit_append(EventType::Approve, Some(&doc_id), OriginalsFlag::Unset, &payload_jcs)?;

        if let Some(open) = self.open.as_mut() {
            open.approval = None;
            open.pending_bodies.remove(&doc_id);
        }

        let summary = self.summarize(&self.require_open()?.master, doc_id)?;
        Ok(SubmitApprovalOut {
            summary,
            lifecycle: ApprovalLifecycle::Committed,
        })
    }

    /// `abort_approval` — api.md §5.4; data-model §8.
    ///
    /// No stored approved version. Retain: catalog and encrypted original remain, and the
    /// caller may `open_approval` again. Discard: RAM original is dropped and the catalog
    /// row is deleted.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked.
    /// - `not_found` if `approval_session_id` is not the active session.
    pub fn abort_approval(&mut self, input: AbortApprovalIn) -> Result<AbortApprovalOut, ApiError> {
        if !command_allowed("abort_approval", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let doc_id = {
            let open = self.require_open()?;
            let session = open.approval.as_ref().ok_or_else(ApiError::not_found)?;
            if session.approval_session_id != input.approval_session_id {
                return Err(ApiError::not_found());
            }
            session.doc_id.clone()
        };
        let retention = self
            .documents
            .load_meta(&self.require_open()?.master, &doc_id)
            .map_err(map_catalog_err)?
            .ok_or_else(ApiError::not_found)?
            .retention;
        if retention == EffectiveRetention::Discard {
            self.documents
                .drop_unapproved(&doc_id)
                .map_err(map_catalog_err)?;
        }
        if let Some(open) = self.open.as_mut() {
            open.approval = None;
            if retention == EffectiveRetention::Discard {
                open.pending_bodies.remove(&doc_id);
            }
        }
        Ok(AbortApprovalOut {
            lifecycle: ApprovalLifecycle::Aborted,
        })
    }

    /// `delete_document` — api.md §5.3; FR-4.6; architecture §4.3.
    ///
    /// Overwrite-and-drop every document-scoped artifact, then the catalog row. Audit
    /// `delete`. Idempotent absence is `not_found` (unknown id), not a second success.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked.
    /// - `not_found` if `doc_id` is unknown.
    pub fn delete_document(&mut self, input: DeleteDocumentIn) -> Result<DeleteDocumentOut, ApiError> {
        if !command_allowed("delete_document", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let doc_id = input.doc_id;
        let exists = self
            .documents
            .load_meta(&self.require_open()?.master, &doc_id)
            .map_err(map_catalog_err)?
            .is_some();
        if !exists {
            return Err(ApiError::not_found());
        }
        self.documents
            .destroy_document(&doc_id)
            .map_err(map_catalog_err)?;
        if let Some(open) = self.open.as_mut() {
            if open
                .approval
                .as_ref()
                .is_some_and(|s| s.doc_id == doc_id)
            {
                open.approval = None;
            }
            open.pending_bodies.remove(&doc_id);
        }
        let payload = serde_json::json!({ "doc_id": doc_id });
        let payload_jcs = serde_json::to_string(&payload)
            .map_err(|_| ApiError::internal("audit payload encode failed"))?;
        self.record_audit_append(EventType::Delete, Some(&doc_id), OriginalsFlag::Unset, &payload_jcs)?;
        Ok(DeleteDocumentOut { ok: true })
    }

    /// `delete_retained_original` — api.md §5.3; FR-4.6 sibling.
    ///
    /// Drops the kind=2 original only. Idempotent if already discarded. Audit
    /// `discard_original` only when an original was present. Canonical approved bytes are
    /// untouched.
    ///
    /// # Errors
    /// - `not_in_session` unless unlocked.
    /// - `not_found` if `doc_id` is unknown.
    pub fn delete_retained_original(
        &mut self,
        input: DeleteRetainedOriginalIn,
    ) -> Result<DeleteRetainedOriginalOut, ApiError> {
        if !command_allowed("delete_retained_original", self.state()) {
            return Err(ApiError::not_in_session());
        }
        let doc_id = input.doc_id;
        let master_present = self
            .documents
            .load_meta(&self.require_open()?.master, &doc_id)
            .map_err(map_catalog_err)?
            .is_some();
        if !master_present {
            return Err(ApiError::not_found());
        }
        let had_original = self
            .documents
            .destroy_original(&doc_id)
            .map_err(map_catalog_err)?;
        if had_original {
            let payload = serde_json::json!({ "doc_id": doc_id });
            let payload_jcs = serde_json::to_string(&payload)
                .map_err(|_| ApiError::internal("audit payload encode failed"))?;
            self.record_audit_append(
                EventType::DiscardOriginal,
                Some(&doc_id),
                OriginalsFlag::Unset,
                &payload_jcs,
            )?;
        }
        let summary = self.summarize(&self.require_open()?.master, doc_id)?;
        Ok(DeleteRetainedOriginalOut { summary })
    }

    /// Page IR for approval: this-unlock `pending_bodies`, or a retain original reconstructed
    /// from the vault after lock (api.md §5.4: retain may `open_approval` again).
    fn approval_document(&self, doc_id: &str) -> Result<Document, ApiError> {
        let open = self.require_open()?;
        if let Some(doc) = open.pending_bodies.get(doc_id) {
            return Ok(doc.clone());
        }
        let original = self
            .documents
            .load_original(&open.master, doc_id)
            .map_err(map_catalog_err)?
            .ok_or_else(ApiError::not_found)?;
        let bytes = original.raw_bytes().map_err(map_catalog_err)?;
        match original.source_format {
            SourceFormat::Text => importer::import_text(&bytes, doc_id)
                .map_err(|_| ApiError::internal("retained original reconstruct failed")),
            SourceFormat::Pdf => importer::import_pdf(&bytes, doc_id)
                .map_err(|_| ApiError::internal("retained original reconstruct failed")),
        }
    }

    /// data-model §8: every unapproved discard document is removed before lock drops RAM.
    fn drop_unapproved_discards(&mut self) -> Result<(), ApiError> {
        let ids = self
            .documents
            .list_ids_newest_first()
            .map_err(map_catalog_err)?;
        let mut drop_ids = Vec::new();
        {
            let open = self.require_open()?;
            for id in ids {
                if self
                    .documents
                    .has_approved_version(&id)
                    .map_err(map_catalog_err)?
                {
                    continue;
                }
                match self
                    .documents
                    .load_meta(&open.master, &id)
                    .map_err(map_catalog_err)?
                {
                    Some(meta) if meta.retention == EffectiveRetention::Discard => {
                        drop_ids.push(id);
                    }
                    _ => {}
                }
            }
        }
        for id in drop_ids {
            self.documents
                .drop_unapproved(&id)
                .map_err(map_catalog_err)?;
        }
        Ok(())
    }

    /// Shared by `list_documents`/`get_document`: decrypt one document's meta and compose
    /// its `DocumentSummary`.
    fn summarize(&self, master: &VaultMasterKey, doc_id: String) -> Result<DocumentSummary, ApiError> {
        let meta = self
            .documents
            .load_meta(master, &doc_id)
            .map_err(map_catalog_err)?
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "document not found"))?;
        Ok(DocumentSummary {
            imported_at: crate::account::format_rfc3339((meta.imported_at_unix_ms / 1000) as i64),
            has_approved_version: self.documents.has_approved_version(&doc_id).map_err(map_catalog_err)?,
            has_retained_original: self.documents.has_retained_original(&doc_id).map_err(map_catalog_err)?,
            detected_field_count: meta.detected_fields.len() as u32,
            source_filename: meta.source_filename,
            source_format: meta.source_format,
            retention: meta.retention,
            doc_id,
        })
    }
}

// ---------------------------------------------------------------------------
// Validation (api.md §5.1) and error mapping
// ---------------------------------------------------------------------------

/// api.md §5.1: "`passphrase` min length 8 (API floor; UI spec may urge longer)."
///
/// Counted in `chars`, so a short passphrase cannot pass the floor by being multi-byte.
/// There is no upper bound and no character-class rule: architecture §3.3 has no
/// recovery path, so an unnecessary restriction here is a way to lose a vault.
fn validate_passphrase(passphrase: &str) -> Result<(), ApiError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(ApiError::invalid_input(
            "passphrase must be at least 8 characters",
        ));
    }
    Ok(())
}

/// api.md §5.1: "`display_name` trimmed, 1..=80 chars. Empty display_name →
/// `invalid_input`." Returns the trimmed name to store.
fn validate_display_name(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim();
    let len = trimmed.chars().count();
    if len == 0 || len > MAX_DISPLAY_NAME_CHARS {
        return Err(ApiError::invalid_input(
            "display_name must be 1 to 80 characters after trimming",
        ));
    }
    Ok(trimmed.to_string())
}

/// api.md §5.3 `import_document`: "`filename`: basename; path separators rejected
/// (`invalid_input`)." Rejects both `/` and `\` so a Windows-style path can't smuggle a
/// separator past a Unix-only check, and rejects empty input.
fn validate_import_filename(raw: &str) -> Result<&str, ApiError> {
    if raw.is_empty() {
        return Err(ApiError::invalid_input("filename must not be empty"));
    }
    if raw.contains('/') || raw.contains('\\') {
        return Err(ApiError::invalid_input(
            "filename must be a basename; path separators are rejected",
        ));
    }
    Ok(raw)
}

/// FR-1.2 / api.md §3: empty bytes, non-UTF-8 "text", an unparseable PDF, and a PDF with
/// no extractable text all collapse to the same `unsupported_document` code — api.md does
/// not distinguish them, and none of the underlying reasons are anything to expose beyond
/// this fixed, non-secret class.
fn unsupported_document() -> ApiError {
    ApiError::new(ErrorCode::UnsupportedDocument, "no extractable text")
}

/// Keystore failures become non-secret `internal` classes. `Corrupt` deliberately does
/// **not** become `not_found`: a damaged item is not an absent one.
fn map_keystore_err(e: KeystoreError) -> ApiError {
    match e {
        KeystoreError::Corrupt => ApiError::internal("keystore item is corrupt"),
        KeystoreError::Unavailable => ApiError::internal("keystore backend is unavailable"),
        KeystoreError::Backend(_) => ApiError::internal("keystore backend failure"),
        KeystoreError::Rng => ApiError::internal("system CSPRNG failure"),
    }
}

fn map_account_err(_: AccountStoreError) -> ApiError {
    ApiError::internal("account store failure")
}

/// W3: vault open failures are `internal`, never `unlock_failed` — the passphrase has
/// already been verified by the time this is called (see `unlock`'s comment above the
/// call site). `VaultError::WrongKey` here would mean the keystore's wrapped master key
/// and the vault file have gone out of sync, not that the user mistyped anything.
fn map_vault_err(_: VaultError) -> ApiError {
    ApiError::internal("vault backend failure")
}

/// W6: config backend failures are `internal`, non-secret classes — same discipline as
/// every other error mapper here.
fn map_config_err(_: ConfigError) -> ApiError {
    ApiError::internal("config backend failure")
}

/// W5/W10: audit backend failures are `internal`, non-secret classes.
fn map_audit_err(_: AuditError) -> ApiError {
    ApiError::internal("audit backend failure")
}

/// W10: catalog backend failures are `internal`, non-secret classes.
fn map_catalog_err(_: CatalogError) -> ApiError {
    ApiError::internal("catalog backend failure")
}

/// Current Unix time in milliseconds — `DocumentMeta.imported_at_unix_ms` and the audit
/// row's `produced_at_unix_ms` (data-model §5.8/§6.1). Saturates to 0 rather than panicking
/// if the system clock is somehow before the epoch.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

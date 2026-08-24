//! Session, account, and config commands (`api.md` §2, §5.1–§5.2; `architecture.md`
//! §3.2–§3.4, §6, §7).
//!
//! Nine in-process command functions: [`SessionManager::get_session_state`],
//! [`SessionManager::create_account`], [`SessionManager::unlock`],
//! [`SessionManager::lock`], [`SessionManager::change_passphrase`],
//! [`SessionManager::get_account`], [`SessionManager::get_integrity_report`] (W5),
//! [`SessionManager::get_retention_default`], [`SessionManager::set_retention_default`]
//! (W6).
//!
//! `dev-plan.md` §1: "**Integration seam for v1 core:** in-process API commands, not the
//! webview." Tauri IPC wiring is W29; these are the functions it will call.
//!
//! # Scope fence (dev-plan.md W6 "Do not: first-import modal UI (W32); per-import override
//! (W10)")
//!
//! Deliberately absent: no concrete `EventPayload` for import/detect/approve/share/etc.
//! (`crate::audit` module docs) since those commands don't exist yet; Secret-Service
//! probing / backend selection (W7); every document, approval, share, variant command; any
//! UI; `detector_preference` read/write (W15c — the field exists in `crate::config::Config`
//! but no command here touches it yet).
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

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::account::{
    new_account_id, now_rfc3339, AccountStore, AccountStoreError, LocalAccount,
};
use crate::api::{ApiError, ErrorCode};
use crate::audit::{AuditError, AuditRow, AuditStore, EventType, FailureKind, OriginalsFlag, VerifyOutcome};
use crate::catalog::{
    CatalogError, DocumentMeta, DocumentStore, EffectiveRetention, NullDocumentStore,
    OriginalRecord,
};
use crate::config::{ConfigError, ConfigStore, NullConfigStore, RetentionPolicy};
use crate::detector::{Detector, StubDetector};
use crate::importer::{self, SourceFormat};
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

/// api.md §6 `phase` on `pg://detect-progress`. W14 only emits [`DetectPhase::Detecting`];
/// [`DetectPhase::WarmingModel`] is the W15b Ollama cold-start value, present on the type
/// so the payload shape is already the spec's.
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
/// gated-but-unwritten commands"; W2 "commands that exist"). `list_audit_events` and every
/// document/approval/share/variant command, plus `get_detector_preference`/
/// `set_detector_preference` (decision 0009), are unregistered — api.md §2 has a row for
/// them, this table does not, because no chunk through W6 implements them yet (dev-plan:
/// "prefer not registering them yet"). api.md §2's row for those is the generic "All
/// document / approval / share / config / cloud-ai / variant / delete" line: `no | no |
/// yes | no` — note that even once registered, none of that family is available while
/// `degraded_integrity` (C-API-6), unlike `lock`/`get_account`/`get_integrity_report`.
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
    // W10: api.md §5.3, same generic config/document row as retention (`no | no | yes |
    // no` — unavailable while degraded, C-API-6).
    ("import_document", &[SessionState::Unlocked]),
    ("list_documents", &[SessionState::Unlocked]),
    ("get_document", &[SessionState::Unlocked]),
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
    detector: Arc<dyn Detector>,
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
            detector: Arc::new(StubDetector),
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

    /// Override the Detector backend. Defaults to [`StubDetector`] (W12) — real detection
    /// (W13's patterns, W15a's ONNX, W15b's optional Ollama backend) replaces it here
    /// without touching `import_document`'s call site.
    #[must_use]
    pub fn with_detector(mut self, detector: Arc<dyn Detector>) -> Self {
        self.detector = detector;
        self
    }

    /// Override the `pg://detect-progress` sink (W14). Defaults to a no-op; W29's Tauri
    /// shim will supply an emitter that flushes to the webview. In-process tests pass a
    /// recording sink.
    #[must_use]
    pub fn with_progress_sink(mut self, progress: Arc<dyn ProgressSink>) -> Self {
        self.progress = progress;
        self
    }

    fn emit_detect_progress(&self, doc_id: &str, fraction: f64) {
        self.progress.emit_detect_progress(DetectProgress {
            doc_id: doc_id.to_string(),
            fraction,
            phase: DetectPhase::Detecting,
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

        // Explicit drop rather than letting it fall out of scope, so the destruction is
        // the visible effect of the command.
        drop(self.open.take());
        // W3: "close the DB" (architecture §3.3's lock contract) — the SQLCipher
        // connection, and whatever key material SQLCipher itself holds, goes with it.
        self.vault.close();
        // architecture §10.2: unload in-process NER on lock. No-op until a detector
        // actually holds weights (W15a's HybridV1 uses a fixture stage with no ONNX
        // session yet).
        self.detector.on_lock();
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

    /// `import_document` — api.md §5.3; FR-1.1–1.5.
    ///
    /// W10 scope: extraction (`crate::importer`), catalog storage
    /// (`crate::catalog`/`DocumentStore`), detection via whatever `self.detector` is
    /// (`NullDetector` — an empty field list — until W12), the `never_retain` → document
    /// `retention: discard` mapping (data-model §6.1), the audit `import` event, and
    /// `over_budget`. **Not yet implemented** (dev-plan W11, the very next chunk):
    /// `retention_policy_unset` when the global default isn't confirmed, and
    /// `retention_loosen_forbidden` for a `retain` override against a `never_retain`
    /// default — both documented in api.md §5.3 but out of this chunk's stated scope
    /// ("Do not: first-import modal UI (W32); per-import override (W10)" — W10 explicitly
    /// does not own the override *gate*, only storing whichever retention value results).
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
        // fields." W12's default is `StubDetector` (`crate::detector` module docs);
        // `field_ids`/`labels` are captured before `detected_fields` moves into `meta`, for
        // the audit `detect` event below.
        //
        // W14 / api.md §6: emit `pg://detect-progress` around detect, never 1.0 before
        // `detect` returns (dev-plan W14 "Do not: fake 100% before detect finishes").
        // `phase` is `"detecting"` until W15b's Ollama cold-start.
        self.emit_detect_progress(&doc_id, 0.0);
        let detected_fields = self.detector.detect(&doc);
        self.emit_detect_progress(&doc_id, 1.0);
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
        // and classifications." data-model §5.8.1's Detect payload also has
        // `backend`/`model_tag`/`fallback_reason` — all `null` until W15c's selection
        // path fills them. `detector_id` is whatever host `with_detector` installed
        // (default remains the W12 stub).
        let detect_payload = serde_json::json!({
            "detector_id": self.detector.id(),
            "field_ids": field_ids,
            "labels": labels,
            "backend": null,
            "model_tag": null,
            "fallback_reason": null,
        });
        let detect_payload_jcs =
            serde_json::to_string(&detect_payload).map_err(|_| ApiError::internal("audit payload encode failed"))?;
        self.record_audit_append(EventType::Detect, Some(&doc_id), OriginalsFlag::Unset, &detect_payload_jcs)?;

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

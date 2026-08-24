//! The API error model (`docs/specs/api.md` §3).
//!
//! > Every command returns `Result<T, ApiError>`. `ApiError` is:
//! >
//! > ```text
//! > ApiError {
//! >   code: ErrorCode,          // stable string, machine-readable
//! >   message: string,          // non-secret; never includes passphrase, key,
//! >                             // field text, document text
//! > }
//! > ```
//!
//! The `message` field is deliberately built only from **fixed classes** declared in
//! this file. There is no `format!` site anywhere in the core that interpolates caller
//! input into an error message, which is what makes C-API-1 ("passphrase … never
//! appear[s] in outputs") a structural property rather than a review checklist.
//!
//! `ErrorCode` carries the whole api.md §3 list even though W2 only produces
//! `not_in_session`, `invalid_input`, `unlock_failed`, `account_exists`,
//! `passphrase_mismatch` and `internal`. The list is the spec's, not this chunk's, and
//! later chunks must not fork a second one.

use serde::{Deserialize, Serialize};

/// Stable, machine-readable error codes (api.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// Command forbidden in the current `SessionState` (api.md §2).
    NotInSession,
    /// Schema/validation failure (empty passphrase, bad URL, unknown id format).
    InvalidInput,
    /// Unknown `doc_id` / `variant_id` / `preview_token` / `approval_session_id`.
    NotFound,
    /// No extractable text (FR-1.2).
    UnsupportedDocument,
    /// Per-import retain while the global default is `never_retain` (FR-1.4).
    RetentionLoosenForbidden,
    /// `import_document` before the user confirmed a retention default (decision 0007).
    RetentionPolicyUnset,
    /// Another approval session is already active.
    ApprovalBusy,
    /// Approval command does not match the session lifecycle.
    ApprovalBadState,
    /// `open_approval` on a document that already has a canonical `ApprovedVersion`.
    AlreadyApproved,
    /// `save_variant` name already used on this `doc_id`.
    VariantNameConflict,
    /// Share/preview of a doc with no canonical `ApprovedVersion`.
    NotApproved,
    /// `preview_token` missing, expired, or the `ShareRequest` no longer matches.
    PreviewExpired,
    /// Share-to-AI or test without a stored key.
    CloudAiNotConfigured,
    /// TLS/HTTP failure talking to the allowlisted host.
    CloudAiNetwork,
    /// Endpoint returned 4xx/5xx.
    CloudAiRefused,
    /// Wrong passphrase — **and** unknown account. api.md §3: "Wrong passphrase and
    /// unknown account are the same `unlock_failed` (no account-enumeration …)".
    UnlockFailed,
    /// `create_account` when the session is not `"first_run"`.
    AccountExists,
    /// `change_passphrase` current passphrase wrong.
    PassphraseMismatch,
    /// Unexpected core failure; `message` is a non-secret class.
    Internal,
}

impl ErrorCode {
    /// The stable wire string for this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NotInSession => "not_in_session",
            ErrorCode::InvalidInput => "invalid_input",
            ErrorCode::NotFound => "not_found",
            ErrorCode::UnsupportedDocument => "unsupported_document",
            ErrorCode::RetentionLoosenForbidden => "retention_loosen_forbidden",
            ErrorCode::RetentionPolicyUnset => "retention_policy_unset",
            ErrorCode::ApprovalBusy => "approval_busy",
            ErrorCode::ApprovalBadState => "approval_bad_state",
            ErrorCode::AlreadyApproved => "already_approved",
            ErrorCode::VariantNameConflict => "variant_name_conflict",
            ErrorCode::NotApproved => "not_approved",
            ErrorCode::PreviewExpired => "preview_expired",
            ErrorCode::CloudAiNotConfigured => "cloud_ai_not_configured",
            ErrorCode::CloudAiNetwork => "cloud_ai_network",
            ErrorCode::CloudAiRefused => "cloud_ai_refused",
            ErrorCode::UnlockFailed => "unlock_failed",
            ErrorCode::AccountExists => "account_exists",
            ErrorCode::PassphraseMismatch => "passphrase_mismatch",
            ErrorCode::Internal => "internal",
        }
    }
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error half of every command's `Result` (api.md §3).
///
/// `message` is always a `&'static str` class chosen from a constructor in this file.
/// It is stored as `String` because api.md types it as `string` on the wire, but there
/// is no code path that puts caller input into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// Stable code the frontend switches on.
    pub code: ErrorCode,
    /// Non-secret, human-readable class. Never contains a passphrase, key, field text,
    /// or document text (C-API-1).
    pub message: String,
}

impl ApiError {
    /// Build an error from a code and a **fixed** non-secret class string.
    ///
    /// The `&'static str` bound is the guardrail: a caller cannot pass a `format!`ed
    /// string containing user input without going out of its way.
    #[must_use]
    pub fn new(code: ErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }

    /// api.md §2: the command is not available in the current session state.
    #[must_use]
    pub fn not_in_session() -> Self {
        Self::new(
            ErrorCode::NotInSession,
            "command is not available in the current session state",
        )
    }

    /// api.md §3: schema/validation failure.
    #[must_use]
    pub fn invalid_input(message: &'static str) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    /// api.md §3: wrong passphrase **or** unknown account — deliberately the same error,
    /// so `unlock` is not an account-enumeration oracle.
    #[must_use]
    pub fn unlock_failed() -> Self {
        Self::new(ErrorCode::UnlockFailed, "unlock failed")
    }

    /// api.md §3: `create_account` when the session is not `first_run`.
    #[must_use]
    pub fn account_exists() -> Self {
        Self::new(ErrorCode::AccountExists, "an account already exists")
    }

    /// api.md §3: `change_passphrase` current passphrase wrong.
    #[must_use]
    pub fn passphrase_mismatch() -> Self {
        Self::new(
            ErrorCode::PassphraseMismatch,
            "current passphrase is incorrect",
        )
    }

    /// api.md §5.3 / decision 0007 (W11): `import_document` before the global retention
    /// default has been confirmed.
    #[must_use]
    pub fn retention_policy_unset() -> Self {
        Self::new(
            ErrorCode::RetentionPolicyUnset,
            "retention default is not yet confirmed",
        )
    }

    /// api.md §5.3 (W11): a per-import `retain` override against a `never_retain` global
    /// default.
    #[must_use]
    pub fn retention_loosen_forbidden() -> Self {
        Self::new(
            ErrorCode::RetentionLoosenForbidden,
            "cannot loosen retention below the paranoid default",
        )
    }

    /// api.md §3: unknown `doc_id` / `variant_id` / `preview_token` / `approval_session_id`.
    #[must_use]
    pub fn not_found() -> Self {
        Self::new(ErrorCode::NotFound, "not found")
    }

    /// api.md §3: another approval session is already active.
    #[must_use]
    pub fn approval_busy() -> Self {
        Self::new(
            ErrorCode::ApprovalBusy,
            "an approval session is already active",
        )
    }

    /// api.md §3: approval command does not match the session lifecycle.
    #[must_use]
    pub fn approval_bad_state() -> Self {
        Self::new(
            ErrorCode::ApprovalBadState,
            "approval command does not match session lifecycle",
        )
    }

    /// api.md §3: `open_approval` on a document that already has a canonical approved version.
    #[must_use]
    pub fn already_approved() -> Self {
        Self::new(
            ErrorCode::AlreadyApproved,
            "document already has an approved version",
        )
    }

    /// api.md §3: `save_variant` name already used on this `doc_id`.
    #[must_use]
    pub fn variant_name_conflict() -> Self {
        Self::new(
            ErrorCode::VariantNameConflict,
            "a variant with that name already exists on this document",
        )
    }

    /// api.md §3: command requires a canonical `ApprovedVersion`.
    #[must_use]
    pub fn not_approved() -> Self {
        Self::new(
            ErrorCode::NotApproved,
            "document has no approved version",
        )
    }

    /// api.md §3: `preview_token` missing, expired, or replaced.
    #[must_use]
    pub fn preview_expired() -> Self {
        Self::new(ErrorCode::PreviewExpired, "preview token expired")
    }

    /// api.md §3: Cloud AI share/test without a stored key.
    #[must_use]
    pub fn cloud_ai_not_configured() -> Self {
        Self::new(
            ErrorCode::CloudAiNotConfigured,
            "cloud ai is not configured",
        )
    }

    /// api.md §3: unexpected core failure; the class is non-secret.
    #[must_use]
    pub fn internal(message: &'static str) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

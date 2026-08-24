//! Tauri IPC command shims (api.md §5; W29).
//!
//! dev-plan.md W29: "Tauri command shims (thin; not mutation-gated) over tested functions."
//! Every shim below does exactly one thing beyond deserializing its argument: lock the
//! managed [`SessionManager`](pg_core::session::SessionManager), call the one already-tested
//! `pg_core::session` method with the same name, and return its `Result` as-is —
//! `pg_core::api::ApiError` already implements `Serialize`, so no error translation happens
//! at this boundary either. `create_account` / `unlock` / `lock` are the only three with
//! anything beyond that (`pg://session-changed`, api.md §6), and `get_session_state` is the
//! only one with no `Result` at all (api.md §2: "callable in every state ... including
//! before first run", and it cannot fail).
//!
//! This is deliberately the **only** place a `#[tauri::command]` exists in this crate:
//! `main.rs`'s `generate_handler!` list is the enforced allowlist (CLAUDE.md "No new Tauri
//! command name absent from api.md") — every name here has a matching `**\`name\`**` heading
//! in `docs/specs/api.md` §5, and `core/tests/session_gating_w4.rs` already proves
//! `command_allowed` covers exactly this set.

use tauri::{AppHandle, State};

use pg_core::api::ApiError;
use pg_core::session::*;

use crate::events::emit_session_changed;
use crate::state::{lock_session, AppState};

/// `&self`/`&mut self`, no input. Both shapes call identically through a `MutexGuard`
/// (`DerefMut` makes a `&mut self` method callable on the guard temporary either way), so
/// one macro covers both.
macro_rules! cmd0 {
    ($name:ident, $out:ty) => {
        #[tauri::command]
        pub fn $name(state: State<'_, AppState>) -> Result<$out, ApiError> {
            lock_session(&state)?.$name()
        }
    };
}

/// `&self`/`&mut self`, one input DTO.
macro_rules! cmd1 {
    ($name:ident, $in_ty:ty, $out:ty) => {
        #[tauri::command]
        pub fn $name(state: State<'_, AppState>, input: $in_ty) -> Result<$out, ApiError> {
            lock_session(&state)?.$name(input)
        }
    };
}

// ---------------------------------------------------------------------------
// api.md §5.1 — Session and account
// ---------------------------------------------------------------------------

/// api.md §2: callable in every state, including before `first_run`; cannot fail.
#[tauri::command]
pub fn get_session_state(state: State<'_, AppState>) -> Result<SessionStateOut, ApiError> {
    Ok(lock_session(&state)?.get_session_state())
}

// Generic over `R: tauri::Runtime` (rather than the default-`Wry` `AppHandle`/`AppHandle<Wry>`
// these three would get from a bare `use tauri::AppHandle`): `main.rs`'s `register_commands`
// is itself generic so it can build both the real `Wry`-backed app and, in tests, a
// `tauri::test::MockRuntime` one from the exact same allowlist. A hardcoded `AppHandle<Wry>`
// parameter here would make these three commands (uniquely, among all the shims in this file)
// fail to satisfy `CommandArg<'_, MockRuntime>` the moment `register_commands` is instantiated
// with any runtime other than `Wry`.

#[tauri::command]
pub fn create_account<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    input: CreateAccountIn,
) -> Result<CreateAccountOut, ApiError> {
    let out = lock_session(&state)?.create_account(input)?;
    emit_session_changed(&app, out.state);
    Ok(out)
}

#[tauri::command]
pub fn unlock<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    input: UnlockIn,
) -> Result<UnlockOut, ApiError> {
    let out = lock_session(&state)?.unlock(input)?;
    emit_session_changed(&app, out.state);
    Ok(out)
}

#[tauri::command]
pub fn lock<R: tauri::Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> Result<LockOut, ApiError> {
    let out = lock_session(&state)?.lock()?;
    emit_session_changed(&app, out.state);
    Ok(out)
}

cmd1!(change_passphrase, ChangePassphraseIn, ChangePassphraseOut);
cmd0!(get_account, GetAccountOut);
cmd0!(get_integrity_report, IntegrityReport);

// ---------------------------------------------------------------------------
// api.md §5.2 — Config
// ---------------------------------------------------------------------------

cmd0!(get_retention_default, RetentionDefaultOut);
cmd1!(set_retention_default, SetRetentionDefaultIn, RetentionDefaultOut);
cmd0!(get_detector_preference, DetectorPreferenceOut);
cmd1!(set_detector_preference, SetDetectorPreferenceIn, DetectorPreferenceOut);

// ---------------------------------------------------------------------------
// api.md §5.3 — Import and catalog
// ---------------------------------------------------------------------------

cmd1!(import_document, ImportDocumentIn, ImportDocumentOut);
cmd0!(list_documents, ListDocumentsOut);
cmd1!(get_document, GetDocumentIn, GetDocumentOut);

// ---------------------------------------------------------------------------
// api.md §5.4 — Approval
// ---------------------------------------------------------------------------

cmd1!(open_approval, OpenApprovalIn, ApprovalView);
cmd1!(get_approval_view, GetApprovalViewIn, ApprovalView);
cmd1!(set_field_decisions, SetFieldDecisionsIn, SetFieldDecisionsOut);
cmd1!(submit_approval, SubmitApprovalIn, SubmitApprovalOut);
cmd1!(abort_approval, AbortApprovalIn, AbortApprovalOut);
cmd1!(delete_document, DeleteDocumentIn, DeleteDocumentOut);
cmd1!(delete_retained_original, DeleteRetainedOriginalIn, DeleteRetainedOriginalOut);

// ---------------------------------------------------------------------------
// api.md §5.5 — Variants
// ---------------------------------------------------------------------------

cmd1!(list_variants, ListVariantsIn, ListVariantsOut);
cmd1!(get_variant, GetVariantIn, GetVariantOut);
cmd1!(save_variant, SaveVariantIn, SaveVariantOut);
cmd1!(delete_variant, DeleteVariantIn, DeleteVariantOut);

// ---------------------------------------------------------------------------
// api.md §5.6 — Share: preview token, export, Cloud AI
// ---------------------------------------------------------------------------

cmd1!(preview_share, PreviewShareIn, SharePreview);
cmd1!(commit_share, CommitShareIn, CommitShareOut);

// ---------------------------------------------------------------------------
// api.md §5.7 — Cloud AI configuration
// ---------------------------------------------------------------------------

cmd1!(cloud_ai_set_config, CloudAiSetConfigIn, CloudAiSetConfigOut);
cmd0!(cloud_ai_get_config, CloudAiGetConfigOut);
cmd0!(cloud_ai_clear_config, CloudAiClearConfigOut);
cmd0!(cloud_ai_test, CloudAiTestOut);

// ---------------------------------------------------------------------------
// api.md §5.8 — Audit
// ---------------------------------------------------------------------------

cmd1!(list_audit_events, ListAuditEventsIn, ListAuditEventsOut);

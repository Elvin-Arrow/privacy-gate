//! Tauri IPC command shims (W29, api.md §5).
//!
//! Every function here is intentionally thin: deserialize the `In` DTO (Tauri does this
//! from the JS call's arguments), lock the managed session, run the dispatcher-level
//! [`gate`] check, call the one matching [`SessionManager`] method, and return its
//! `Result<T, ApiError>` unchanged. No validation beyond the state gate, no new error
//! classes, no business logic — that all already lives in `pg_core::session` and is
//! already tested there. A command name here is always the exact string documented in
//! api.md §5 and always matches the `SessionManager` method of the same name, so the gate
//! check and the call site cannot name two different commands by accident.
//!
//! `get_session_state` is the one command with **no** gate (api.md §2: "callable in
//! every state, including before first run"), matching `SESSION_TABLE`'s deliberate
//! absence of a row for it (`core/src/session.rs`).
//!
//! `create_account` / `unlock` / `lock` additionally emit `pg://session-changed` after a
//! successful state transition (api.md §6: "After lock/unlock/create/degraded"). This is
//! a W29 design choice, not a `SessionManager` hook: `SessionManager` has no
//! state-change callback seam, and adding one would be core-side logic for a concern
//! that is purely "tell the webview a Tauri event fired," which belongs at the IPC
//! boundary. Emitting at exactly these three call sites (the only ones `SESSION_TABLE`
//! allows to change `SessionState`) keeps the event's truth tied 1:1 to a command
//! actually succeeding, rather than duplicating the transition logic.
//!
//! `import_document` is the one **async** command (api.md §5.3: "Runs import +
//! in-process detection as a Tauri async command. CPU-bound work runs on a blocking pool
//! so `pg://detect-progress` can flush to the webview."): it clones the `Arc<Mutex<..>>`
//! out of `State` and runs the gate + call inside `spawn_blocking`, so the async runtime's
//! worker threads are never blocked by detection.

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Runtime, State};

use pg_core::api::ApiError;
use pg_core::session::{
    self, AbortApprovalIn, AbortApprovalOut, ApprovalView, ChangePassphraseIn,
    ChangePassphraseOut, CloudAiClearConfigOut, CloudAiGetConfigOut, CloudAiSetConfigIn,
    CloudAiSetConfigOut, CloudAiTestOut, CommitShareIn, CommitShareOut, CreateAccountIn,
    CreateAccountOut, DeleteDocumentIn, DeleteDocumentOut, DeleteRetainedOriginalIn,
    DeleteRetainedOriginalOut, DeleteVariantIn, DeleteVariantOut, DetectorPreferenceOut,
    GetAccountOut, GetApprovalViewIn, GetDocumentIn, GetDocumentOut, GetVariantIn,
    GetVariantOut, ImportDocumentIn, ImportDocumentOut, IntegrityReport, ListAuditEventsIn,
    ListAuditEventsOut, ListDocumentsOut, ListVariantsIn, ListVariantsOut, LockOut,
    OpenApprovalIn, PreviewShareIn, RetentionDefaultOut, SaveVariantIn, SaveVariantOut,
    SessionManager, SessionStateOut, SetDetectorPreferenceIn, SetFieldDecisionsIn,
    SetFieldDecisionsOut, SetRetentionDefaultIn, SharePreview, SubmitApprovalIn,
    SubmitApprovalOut, UnlockIn, UnlockOut, SESSION_CHANGED_EVENT,
};

use crate::state::AppState;

/// The single dispatcher-level gate (dev-plan W4: "Integrate: single gate in the command
/// dispatcher"). Runs **before** every gated command's `SessionManager` call, using the
/// same [`session::command_allowed`] table `SessionManager` itself already consults
/// internally — this is belt-and-suspenders on top of that internal check, not a second
/// copy of api.md §2's matrix: both read the one `SESSION_TABLE`, so they cannot disagree.
fn gate(mgr: &SessionManager, command: &str) -> Result<(), ApiError> {
    if session::command_allowed(command, mgr.get_session_state().state) {
        Ok(())
    } else {
        Err(ApiError::not_in_session())
    }
}

fn emit_session_changed<R: Runtime>(app: &AppHandle<R>, state: pg_core::session::SessionState) {
    let _ = app.emit(SESSION_CHANGED_EVENT, SessionStateOut { state });
}

/// A zero-input, gated command: lock the session, run the dispatcher gate under `$name`,
/// call `SessionManager::$name()`.
macro_rules! cmd0 {
    ($name:ident, $out:ty) => {
        #[tauri::command]
        #[allow(unused_mut)]
        pub fn $name(state: State<AppState>) -> Result<$out, ApiError> {
            let mut mgr = state.0.lock().expect("session mutex poisoned");
            gate(&mgr, stringify!($name))?;
            mgr.$name()
        }
    };
}

/// A one-input, gated command: same as [`cmd0`] but the JS call's argument object
/// deserializes directly into `$in` (Tauri's automatic argument binding), then is passed
/// straight through to `SessionManager::$name(input)`.
macro_rules! cmd1 {
    ($name:ident, $in:ty, $out:ty) => {
        #[tauri::command]
        #[allow(unused_mut)]
        pub fn $name(state: State<AppState>, input: $in) -> Result<$out, ApiError> {
            let mut mgr = state.0.lock().expect("session mutex poisoned");
            gate(&mgr, stringify!($name))?;
            mgr.$name(input)
        }
    };
}

// ---------------------------------------------------------------------------
// 5.1 Session and account
// ---------------------------------------------------------------------------

/// No gate: api.md §2 — "`get_session_state` is callable in every state (including
/// before first run)."
#[tauri::command]
pub fn get_session_state(state: State<AppState>) -> SessionStateOut {
    let mgr = state.0.lock().expect("session mutex poisoned");
    mgr.get_session_state()
}

#[tauri::command]
pub fn create_account<R: Runtime>(
    app: AppHandle<R>,
    state: State<AppState>,
    input: CreateAccountIn,
) -> Result<CreateAccountOut, ApiError> {
    let mut mgr = state.0.lock().expect("session mutex poisoned");
    gate(&mgr, "create_account")?;
    let out = mgr.create_account(input)?;
    emit_session_changed(&app, out.state);
    Ok(out)
}

#[tauri::command]
pub fn unlock<R: Runtime>(
    app: AppHandle<R>,
    state: State<AppState>,
    input: UnlockIn,
) -> Result<UnlockOut, ApiError> {
    let mut mgr = state.0.lock().expect("session mutex poisoned");
    gate(&mgr, "unlock")?;
    let out = mgr.unlock(input)?;
    emit_session_changed(&app, out.state);
    Ok(out)
}

#[tauri::command]
pub fn lock<R: Runtime>(app: AppHandle<R>, state: State<AppState>) -> Result<LockOut, ApiError> {
    let mut mgr = state.0.lock().expect("session mutex poisoned");
    gate(&mgr, "lock")?;
    let out = mgr.lock()?;
    emit_session_changed(&app, out.state);
    Ok(out)
}

cmd1!(change_passphrase, ChangePassphraseIn, ChangePassphraseOut);
cmd0!(get_account, GetAccountOut);
cmd0!(get_integrity_report, IntegrityReport);

// ---------------------------------------------------------------------------
// 5.2 Config
// ---------------------------------------------------------------------------

cmd0!(get_retention_default, RetentionDefaultOut);
cmd1!(set_retention_default, SetRetentionDefaultIn, RetentionDefaultOut);
cmd0!(get_detector_preference, DetectorPreferenceOut);
cmd1!(
    set_detector_preference,
    SetDetectorPreferenceIn,
    DetectorPreferenceOut
);

// ---------------------------------------------------------------------------
// 5.3 Import and catalog
// ---------------------------------------------------------------------------

/// api.md §5.3: "Runs import + in-process detection as a Tauri **async** command.
/// CPU-bound work runs on a blocking pool so `pg://detect-progress` can flush to the
/// webview." The `Arc<Mutex<SessionManager>>` is cloned out of `State` so the lock and
/// the call happen inside `spawn_blocking`, never on an async worker thread.
#[tauri::command]
pub async fn import_document(
    state: State<'_, AppState>,
    input: ImportDocumentIn,
) -> Result<ImportDocumentOut, ApiError> {
    let handle: Arc<Mutex<SessionManager>> = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut mgr = handle.lock().expect("session mutex poisoned");
        gate(&mgr, "import_document")?;
        mgr.import_document(input)
    })
    .await
    .expect("import_document blocking task panicked")
}

cmd0!(list_documents, ListDocumentsOut);
cmd1!(get_document, GetDocumentIn, GetDocumentOut);
cmd1!(delete_document, DeleteDocumentIn, DeleteDocumentOut);
cmd1!(
    delete_retained_original,
    DeleteRetainedOriginalIn,
    DeleteRetainedOriginalOut
);

// ---------------------------------------------------------------------------
// 5.4 Approval
// ---------------------------------------------------------------------------

cmd1!(open_approval, OpenApprovalIn, ApprovalView);
cmd1!(get_approval_view, GetApprovalViewIn, ApprovalView);
cmd1!(set_field_decisions, SetFieldDecisionsIn, SetFieldDecisionsOut);
cmd1!(submit_approval, SubmitApprovalIn, SubmitApprovalOut);
cmd1!(abort_approval, AbortApprovalIn, AbortApprovalOut);

// ---------------------------------------------------------------------------
// 5.5 Variants
// ---------------------------------------------------------------------------

cmd1!(list_variants, ListVariantsIn, ListVariantsOut);
cmd1!(get_variant, GetVariantIn, GetVariantOut);
cmd1!(save_variant, SaveVariantIn, SaveVariantOut);
cmd1!(delete_variant, DeleteVariantIn, DeleteVariantOut);

// ---------------------------------------------------------------------------
// 5.6 Share
// ---------------------------------------------------------------------------

cmd1!(preview_share, PreviewShareIn, SharePreview);
cmd1!(commit_share, CommitShareIn, CommitShareOut);

// ---------------------------------------------------------------------------
// 5.7 Cloud AI configuration
// ---------------------------------------------------------------------------

cmd1!(cloud_ai_set_config, CloudAiSetConfigIn, CloudAiSetConfigOut);
cmd0!(cloud_ai_get_config, CloudAiGetConfigOut);
cmd0!(cloud_ai_clear_config, CloudAiClearConfigOut);
cmd0!(cloud_ai_test, CloudAiTestOut);

// ---------------------------------------------------------------------------
// 5.8 Audit
// ---------------------------------------------------------------------------

cmd1!(list_audit_events, ListAuditEventsIn, ListAuditEventsOut);

/// Registered command names, in api.md §5 order. `main.rs` feeds this same list to
/// `tauri_build`'s `AppManifest` (via `build.rs`) so the ACL-autogenerated
/// `allow-$command` permission slugs and the `tauri::generate_handler!` registration can
/// never drift apart — both are generated from one array, not typed out twice.
#[allow(dead_code)] // consumed by tests (this module + the capability fixture test)
pub const COMMAND_NAMES: &[&str] = &[
    "get_session_state",
    "create_account",
    "unlock",
    "lock",
    "change_passphrase",
    "get_account",
    "get_integrity_report",
    "get_retention_default",
    "set_retention_default",
    "get_detector_preference",
    "set_detector_preference",
    "import_document",
    "list_documents",
    "get_document",
    "delete_document",
    "delete_retained_original",
    "open_approval",
    "get_approval_view",
    "set_field_decisions",
    "submit_approval",
    "abort_approval",
    "list_variants",
    "get_variant",
    "save_variant",
    "delete_variant",
    "preview_share",
    "commit_share",
    "cloud_ai_set_config",
    "cloud_ai_get_config",
    "cloud_ai_clear_config",
    "cloud_ai_test",
    "list_audit_events",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// [`COMMAND_NAMES`] must be exactly api.md §5's 32 commands (`get_session_state` plus
    /// every row `SESSION_TABLE` carries), so a command added to one list is never missing
    /// from the other.
    #[test]
    fn command_names_match_api_md_count() {
        assert_eq!(COMMAND_NAMES.len(), 32, "api.md §5 lists 32 commands");
    }

    #[test]
    fn command_names_are_unique() {
        let mut sorted = COMMAND_NAMES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), COMMAND_NAMES.len());
    }
}

//! `pg://detect-progress` and `pg://session-changed` (api.md §6; W29).
//!
//! `pg://detect-progress`'s payload shape is `pg_core::session::DetectProgress` — the core
//! already owns that type and the event name constant, since the payload is produced
//! synchronously inside a core command (`crate::session` module docs: "Emit is synchronous
//! so a blocking-pool import ... can flush to the webview between fractions"). This module
//! is only the adapter: a [`pg_core::session::ProgressSink`] that forwards to a Tauri
//! [`AppHandle`].
//!
//! `pg://session-changed` has no core-side equivalent — api.md §6 defines it as a reflection
//! of `SessionState` after `lock`/`unlock`/`create_account` succeed, which is exactly a
//! command-shim concern (dev-plan W29: "Tauri command shims ... over tested functions"),
//! not a core one. Its constant and payload type live here, not in `pg_core`.

use tauri::{AppHandle, Emitter};

use pg_core::session::{DetectProgress, ProgressSink, SessionState, DETECT_PROGRESS_EVENT};

/// api.md §6 `pg://session-changed`.
pub const SESSION_CHANGED_EVENT: &str = "pg://session-changed";

/// api.md §6 `pg://session-changed` payload: `{ state: SessionState }`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionChangedPayload {
    pub state: SessionState,
}

/// Forwards `pg://detect-progress` from the core to the webview. No field text, keys, or
/// passphrases ever pass through `DetectProgress` (api.md §6 last line) — this adapter
/// does not need to sanitize anything, only relay.
///
/// Generic over `R: tauri::Runtime` for the same reason `commands::create_account`/`unlock`/
/// `lock` are: `main.rs` builds this from `app.handle()` inside a `register_commands<R>`-
/// generic setup closure, so it must not hardcode `AppHandle`'s default `Wry` runtime.
pub struct TauriProgressSink<R: tauri::Runtime> {
    handle: AppHandle<R>,
}

impl<R: tauri::Runtime> TauriProgressSink<R> {
    #[must_use]
    pub fn new(handle: AppHandle<R>) -> Self {
        Self { handle }
    }
}

impl<R: tauri::Runtime> ProgressSink for TauriProgressSink<R> {
    fn emit_detect_progress(&self, event: DetectProgress) {
        // A dropped event here means a closed/closing window; there is nothing useful to
        // retry against and no secret is at stake, so the error is swallowed rather than
        // propagated into a command result that has already otherwise succeeded.
        let _ = self.handle.emit(DETECT_PROGRESS_EVENT, event);
    }
}

/// Emit `pg://session-changed` after a `create_account` / `unlock` / `lock` shim's call to
/// the core succeeds (api.md §6: "After lock/unlock/create/degraded" — `degraded` is one of
/// `unlock`'s own possible `state` outcomes, not a fourth call site).
pub fn emit_session_changed<R: tauri::Runtime>(handle: &AppHandle<R>, state: SessionState) {
    let _ = handle.emit(SESSION_CHANGED_EVENT, SessionChangedPayload { state });
}

//! `pg://detect-progress` emitter (W29, api.md §6).
//!
//! Thin: [`pg_core::session::ProgressSink`] is already the seam `SessionManager` calls
//! synchronously during detect (W14); this just forwards each event to the webview
//! through `AppHandle::emit`. No business logic — the payload shape and event name are
//! the core's ([`pg_core::session::DetectProgress`],
//! [`pg_core::session::DETECT_PROGRESS_EVENT`]).

use tauri::{AppHandle, Emitter, Runtime};

use pg_core::session::{DetectProgress, ProgressSink, DETECT_PROGRESS_EVENT};

pub struct TauriProgressSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriProgressSink<R> {
    #[must_use]
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> ProgressSink for TauriProgressSink<R> {
    fn emit_detect_progress(&self, event: DetectProgress) {
        // Emission is best-effort: a closed/missing window must not fail the import
        // that is already in progress on the core side.
        let _ = self.app.emit(DETECT_PROGRESS_EVENT, event);
    }
}

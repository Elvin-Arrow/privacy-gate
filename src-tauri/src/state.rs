//! Tauri-managed state (W29).
//!
//! `SessionManager` is "not internally synchronized" by design (its own doc comment:
//! "`&mut self` on the mutating commands makes the state machine's transitions exclusive
//! by construction. W29 wraps one of these in a Tauri managed `Mutex`."). This is that
//! wrapper. It is `Arc<Mutex<..>>` rather than a bare `Mutex<..>` so `import_document`
//! (the one command that must run on a blocking pool per api.md §5.3) can clone the
//! handle into a `spawn_blocking` closure without borrowing the `tauri::State` across an
//! `.await`.

use std::sync::{Arc, Mutex};

use pg_core::session::SessionManager;

/// The single process-wide session. One document/approval/preview session at a time
/// (design §2.3) is already a `SessionManager` invariant; this just makes it `Send`-safe
/// to reach from every IPC command handler.
pub struct AppState(pub Arc<Mutex<SessionManager>>);

impl AppState {
    #[must_use]
    pub fn new(manager: SessionManager) -> Self {
        Self(Arc::new(Mutex::new(manager)))
    }
}

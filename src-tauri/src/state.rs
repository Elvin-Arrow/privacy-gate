//! Production wiring for one [`pg_core::session::SessionManager`] (W29).
//!
//! Every backend here is exactly what the core's own tests already exercise
//! (`core/tests/*.rs` wire the same traits over `SqlCipherVault` and
//! `keystore::select_backend`) — this module's only job is picking the real paths and
//! managed-state plumbing a Tauri app needs that a test harness does not: an app-data
//! directory from the OS, and a `Mutex` so IPC command handlers (which run on Tauri's
//! async runtime, potentially concurrently) get exclusive access to one session.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pg_core::account::AccountStore;
use pg_core::audit::AuditStore;
use pg_core::catalog::DocumentStore;
use pg_core::cloud_ai::CloudAiStore;
use pg_core::config::ConfigStore;
use pg_core::keystore::select_backend;
use pg_core::session::{ProgressSink, SessionManager};
use pg_core::vault::{SqlCipherVault, VaultBackend};

/// architecture.md §4.1: `vault.db` directly under the platform app-data directory (Tauri's
/// `app_data_dir()` already namespaces that by `identifier` from `tauri.conf.json`, so no
/// extra `privacy-gate/` subfolder is added here).
const VAULT_FILE_NAME: &str = "vault.db";

/// Tauri-managed state: one [`SessionManager`] behind a [`Mutex`] (module docs: "`&mut
/// self` on the mutating commands makes the state machine's transitions exclusive by
/// construction" — this is the "W29 wraps one of these in a Tauri managed `Mutex`" the
/// core's own doc comment already anticipates).
pub struct AppState(pub Mutex<SessionManager>);

/// Build the production [`SessionManager`]: a real [`SqlCipherVault`] at
/// `app_data_dir/vault.db`, [`select_backend`]'s real-vs-fallback keystore probe
/// (architecture §3.2), and `progress` wired in for `pg://detect-progress`.
///
/// Deliberately does **not** call `.with_detector(..)`: that builder exists so tests can
/// pin [`pg_core::detector::StubDetector`] (AC-1..AC-4); production leaves the detector
/// unset so `import_document` uses W15c's real per-detect `auto`/`bundled_only` selection
/// between `pg-hybrid-v1` and the optional Ollama backend.
///
/// # Panics
/// If `app_data_dir` cannot be created. There is no reduced-functionality fallback for "no
/// writable app-data directory" — the vault and keystore fallback file both need one.
#[must_use]
pub fn build_session_manager(app_data_dir: &Path, progress: Arc<dyn ProgressSink>) -> SessionManager {
    std::fs::create_dir_all(app_data_dir).expect("create app-data directory");

    let vault = Arc::new(SqlCipherVault::new(app_data_dir.join(VAULT_FILE_NAME)));
    let keystore = select_backend(app_data_dir);
    let accounts: Arc<dyn AccountStore> = vault.clone();
    let backend: Arc<dyn VaultBackend> = vault.clone();
    let audit: Arc<dyn AuditStore> = vault.clone();
    let config: Arc<dyn ConfigStore> = vault.clone();
    let documents: Arc<dyn DocumentStore> = vault.clone();
    let cloud_ai: Arc<dyn CloudAiStore> = vault.clone();

    SessionManager::new_full(keystore, accounts, backend, audit, config)
        .with_documents(documents)
        .with_cloud_ai(cloud_ai)
        .with_progress_sink(progress)
}

/// Lock the managed session, mapping mutex poisoning (a prior command panicked while
/// holding it) to a fixed, non-secret `internal` [`pg_core::api::ApiError`] rather than
/// panicking again on every subsequent command — a poisoned lock should degrade the app to
/// "commands fail" (recoverable by restarting), not wedge every future command call
/// permanently at the panic call site.
///
/// # Errors
/// [`pg_core::api::ApiError::internal`] if the mutex is poisoned.
pub fn lock_session<'a>(
    state: &'a tauri::State<'a, AppState>,
) -> Result<std::sync::MutexGuard<'a, SessionManager>, pg_core::api::ApiError> {
    state
        .0
        .lock()
        .map_err(|_| pg_core::api::ApiError::internal("session state poisoned"))
}

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod progress;
mod state;

#[cfg(test)]
mod capability_fixture;
#[cfg(test)]
mod ipc_roundtrip_tests;

use std::sync::Arc;

use tauri::Manager;

use pg_core::session::SessionManager;
use pg_core::vault::{SqlCipherVault, VaultBackend};

use crate::progress::TauriProgressSink;
use crate::state::AppState;

/// architecture.md §4.1 ("Locations"): the SQLCipher database lives in the platform
/// app-data directory as `vault.db`, not next to the install image. `AppHandle::path()`
/// resolves the OS-correct directory; this is the one implementation choice architecture
/// leaves open ("exact path is implementation").
const VAULT_FILE_NAME: &str = "vault.db";

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("resolve app data dir");
            std::fs::create_dir_all(&app_data_dir).expect("create app data dir");

            let keystore = pg_core::keystore::select_backend(&app_data_dir);

            let vault = Arc::new(SqlCipherVault::new(app_data_dir.join(VAULT_FILE_NAME)));
            let accounts: Arc<dyn pg_core::account::AccountStore> = vault.clone();
            let backend: Arc<dyn VaultBackend> = vault.clone();
            let audit: Arc<dyn pg_core::audit::AuditStore> = vault.clone();
            let config: Arc<dyn pg_core::config::ConfigStore> = vault.clone();
            let documents: Arc<dyn pg_core::catalog::DocumentStore> = vault.clone();
            let plugin_secrets: Arc<dyn pg_core::cloud_ai::PluginSecretStore> = vault;

            let progress_sink = Arc::new(TauriProgressSink::new(app.handle().clone()));

            // No `.with_detector(..)` override here: production import uses W15c's
            // per-detect selection between `pg-hybrid-v1` and the optional Ollama backend
            // (architecture §10.1). `with_detector(StubDetector)` is test-only.
            let manager = SessionManager::new_full(keystore, accounts, backend, audit, config)
                .with_documents(documents)
                .with_plugin_secrets(plugin_secrets)
                .with_progress_sink(progress_sink);

            app.manage(AppState::new(manager));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_session_state,
            commands::create_account,
            commands::unlock,
            commands::lock,
            commands::change_passphrase,
            commands::get_account,
            commands::get_integrity_report,
            commands::get_retention_default,
            commands::set_retention_default,
            commands::get_detector_preference,
            commands::set_detector_preference,
            commands::import_document,
            commands::list_documents,
            commands::get_document,
            commands::delete_document,
            commands::delete_retained_original,
            commands::open_approval,
            commands::get_approval_view,
            commands::set_field_decisions,
            commands::submit_approval,
            commands::abort_approval,
            commands::list_variants,
            commands::get_variant,
            commands::save_variant,
            commands::delete_variant,
            commands::preview_share,
            commands::commit_share,
            commands::cloud_ai_set_config,
            commands::cloud_ai_get_config,
            commands::cloud_ai_clear_config,
            commands::cloud_ai_test,
            commands::list_audit_events,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

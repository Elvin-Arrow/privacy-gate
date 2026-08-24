// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;
mod state;

use std::sync::Arc;

use tauri::Manager;

use state::AppState;

/// The invoke allowlist (dev-plan W29 / CLAUDE.md "No new Tauri command name absent from
/// `api.md`"). Factored out of `main()` so `#[cfg(test)]` builds the exact same allowlist
/// against [`tauri::test::mock_builder`] instead of a second, hand-maintained copy that
/// could silently drift from the real one.
fn register_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
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
        commands::open_approval,
        commands::get_approval_view,
        commands::set_field_decisions,
        commands::submit_approval,
        commands::abort_approval,
        commands::delete_document,
        commands::delete_retained_original,
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
}

fn main() {
    // Pinned explicitly: `register_commands<R>` and the `AppHandle<R>`-generic shims it
    // wires up no longer force `R = Wry` on their own (that's the point — they also need
    // to work with `tauri::test::MockRuntime`), so the concrete runtime for the real app
    // has to be anchored somewhere, and here is the one place it belongs.
    let builder: tauri::Builder<tauri::Wry> = tauri::Builder::default();
    register_commands(
        builder
            .plugin(tauri_plugin_dialog::init())
            .plugin(tauri_plugin_fs::init())
            .setup(|app| {
                let app_data_dir = app
                    .path()
                    .app_data_dir()
                    .expect("resolve app-data directory");
                let progress = Arc::new(events::TauriProgressSink::new(app.handle().clone()));
                let manager = state::build_session_manager(&app_data_dir, progress);
                app.manage(AppState(std::sync::Mutex::new(manager)));
                Ok(())
            }),
    )
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    //! dev-plan W29 "Tests first": a capability fixture proving read/HTTP/shell stay
    //! denied, and an IPC round trip proving `register_commands`' allowlist actually
    //! reaches an already-tested `pg_core::session::SessionManager` method through the
    //! full Tauri invoke path (not just a direct Rust call, the way `commands.rs`'s own
    //! macros would be exercised by a unit test) — `tauri::test::mock_builder` runs no
    //! real webview/webkit2gtk, so this needs no display server.

    use std::sync::Arc;

    use serde_json::json;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::Manager;

    use super::*;
    use pg_core::session::SessionState;

    /// Builds a mock app wired exactly like production (`register_commands`) but with
    /// `AppState` pointed at a throwaway vault directory instead of a real OS app-data
    /// path, so the test never touches this machine's real Privacy Gate vault.
    fn test_app() -> (tauri::App<tauri::test::MockRuntime>, tempfile::TempDir) {
        let app = register_commands(mock_builder())
            .build(mock_context(noop_assets()))
            .expect("build mock app");
        let dir = tempfile::tempdir().expect("tempdir");
        let progress = Arc::new(events::TauriProgressSink::new(app.handle().clone()));
        let manager = state::build_session_manager(dir.path(), progress);
        app.manage(AppState(std::sync::Mutex::new(manager)));
        (app, dir)
    }

    fn invoke(
        webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
        cmd: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        get_ipc_response(
            webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: InvokeBody::Json(json!({ "input": input })),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|b| b.deserialize::<serde_json::Value>().unwrap())
        .map_err(|e| e.to_string())
    }

    /// dev-plan W29: "shims round-trip a command already tested in-process" — `create_account`
    /// then `unlock` through the real IPC dispatch, asserting the same `state` transitions
    /// that `core/tests/*` already prove for `SessionManager::create_account`/`unlock`
    /// directly, showing the shim layer plumbs input/output/state faithfully rather than
    /// re-proving the core behavior itself.
    #[test]
    fn lock_unlock_round_trips_through_ipc() {
        let (app, _dir) = test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("build mock webview");

        let created = invoke(
            &webview,
            "create_account",
            json!({
                "display_name": "Test User",
                "passphrase": "correct horse battery staple",
            }),
        )
        .expect("create_account should succeed");
        assert_eq!(created["state"], json!(SessionState::Unlocked));

        let locked = invoke(&webview, "lock", json!(null)).expect("lock should succeed");
        assert_eq!(locked["state"], json!(SessionState::Locked));

        let unlocked = invoke(
            &webview,
            "unlock",
            json!({ "passphrase": "correct horse battery staple" }),
        )
        .expect("unlock should succeed");
        assert_eq!(unlocked["state"], json!(SessionState::Unlocked));

        let wrong = invoke(
            &webview,
            "unlock",
            json!({ "passphrase": "not the passphrase" }),
        );
        assert!(wrong.is_err(), "wrong passphrase must not unlock");
    }

    /// dev-plan W29: "capability fixture denies read/HTTP/shell". The webview never gets a
    /// real ACL check in this mock harness (`mock_context` uses `Resolved::default()`), so
    /// this asserts the fixture file itself — the thing `tauri::generate_context!()` loads
    /// in the real app — carries the required denies, per ui.md §3.2 ("Deny read, readDir,
    /// remove, exists, and watch") and architecture C-ARCH-2 (no HTTP/shell plugin surface
    /// reachable from the webview).
    #[test]
    fn capability_fixture_denies_read_http_shell() {
        let raw = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"),
        )
        .expect("read capabilities/default.json");
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("parse capabilities json");

        let permissions: Vec<&str> = doc["permissions"]
            .as_array()
            .expect("permissions array")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let deny: Vec<&str> = doc["deny"]
            .as_array()
            .expect("deny array")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        for must_deny in [
            "fs:allow-read",
            "fs:allow-read-dir",
            "fs:allow-remove",
            "fs:allow-exists",
            "fs:allow-watch",
            "dialog:allow-open",
            "http:default",
            "shell:allow-execute",
            "shell:allow-kill",
            "shell:allow-open",
        ] {
            assert!(
                deny.contains(&must_deny),
                "capabilities/default.json must deny {must_deny}"
            );
            assert!(
                !permissions.contains(&must_deny),
                "capabilities/default.json must not grant {must_deny}"
            );
        }

        // No permission string anywhere starts with a broad `http:` or `shell:` grant —
        // guards against a future edit adding e.g. "http:default" to `permissions` under a
        // different key than the ones enumerated above.
        for granted in &permissions {
            assert!(
                !granted.starts_with("http:") && !granted.starts_with("shell:"),
                "unexpected HTTP/shell permission granted to the webview: {granted}"
            );
        }
    }
}

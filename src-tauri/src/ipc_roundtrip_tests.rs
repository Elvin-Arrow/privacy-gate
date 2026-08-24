//! dev-plan W29 "Tests first: … shims round-trip a command already tested in-process."
//!
//! Uses `tauri::test::mock_builder`/`mock_context` (the test harness the installed Tauri
//! 2.11.5 ships, `tauri::test`, feature `test`) to register the **real** command
//! functions from `crate::commands` through `tauri::generate_handler!` — the same macro
//! `main.rs` uses — and invoke them over the mocked IPC path (`get_ipc_response`), not by
//! calling the Rust functions directly. This proves the plumbing (deserialization,
//! `State` extraction, the dispatcher gate, error serialization) without re-testing
//! `SessionManager`'s own business logic, which `core/tests/*` already covers.

use std::sync::{Arc, Mutex};

use serde_json::json;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::Manager;

use pg_core::account::InMemoryAccountStore;
use pg_core::keystore::InMemoryKeystore;
use pg_core::session::SessionManager;

use crate::commands;
use crate::state::AppState;

fn invoke_request(cmd: &str, body: serde_json::Value) -> InvokeRequest {
    // Matches `tauri::test`'s own doctest: the "local" origin URL scheme differs by
    // platform (`http://tauri.localhost` on Windows/Android, `tauri://localhost`
    // elsewhere) — using the wrong one makes the mock resolve every command as a remote
    // origin and fail the ACL's `windows: "main"` match for reasons unrelated to this
    // chunk's dispatcher gate.
    let url = if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: url.parse().unwrap(),
        body: InvokeBody::from(body),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    }
}

fn fresh_app() -> tauri::App<tauri::test::MockRuntime> {
    let manager = SessionManager::new(
        Arc::new(InMemoryKeystore::new()),
        Arc::new(InMemoryAccountStore::default()),
    );

    // `tauri::generate_context!()` (not `mock_context`) so the resolved capability ACL is
    // the **real** `capabilities/default.json` this chunk wrote — `mock_context` has no
    // capabilities at all, which would make every command "not allowed" for a reason
    // unrelated to the dispatcher gate this test exists to prove.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            commands::get_session_state,
            commands::create_account,
            commands::unlock,
            commands::lock,
        ])
        .build(tauri::generate_context!())
        .expect("build mock app");

    app.manage(AppState::new(manager));
    app
}

#[test]
fn get_session_state_reports_first_run_before_any_account() {
    let app = fresh_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("build mock webview");

    let response = get_ipc_response(&webview, invoke_request("get_session_state", json!({})))
        .expect("get_session_state should succeed")
        .deserialize::<serde_json::Value>()
        .unwrap();

    assert_eq!(response, json!({ "state": "first_run" }));
}

#[test]
fn create_account_then_lock_then_unlock_round_trips_through_ipc() {
    let app = fresh_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("build mock webview");

    // create_account: first_run -> unlocked.
    let created = get_ipc_response(
        &webview,
        invoke_request(
            "create_account",
            json!({ "input": { "display_name": "Alex", "passphrase": "correct horse battery" } }),
        ),
    )
    .expect("create_account should succeed")
    .deserialize::<serde_json::Value>()
    .unwrap();
    assert_eq!(created["state"], json!("unlocked"));

    // get_session_state now reports unlocked.
    let state = get_ipc_response(&webview, invoke_request("get_session_state", json!({})))
        .expect("get_session_state should succeed")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(state, json!({ "state": "unlocked" }));

    // lock: unlocked -> locked.
    let locked = get_ipc_response(&webview, invoke_request("lock", json!({})))
        .expect("lock should succeed")
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(locked, json!({ "state": "locked" }));

    // unlock with the wrong passphrase fails with the same ApiError shape a direct
    // SessionManager call would produce (api.md §3).
    let bad = get_ipc_response(
        &webview,
        invoke_request("unlock", json!({ "input": { "passphrase": "wrong passphrase here" } })),
    );
    let err = bad.expect_err("wrong passphrase must fail");
    let err_value: serde_json::Value = serde_json::from_str(&err.to_string())
        .unwrap_or_else(|_| json!({ "raw": err.to_string() }));
    // `InvokeError`'s Display is the JSON-encoded ApiError; just assert the code surfaces.
    assert!(
        err_value.to_string().contains("unlock_failed") || err.to_string().contains("unlock_failed"),
        "expected unlock_failed in error, got: {err}"
    );

    // unlock with the right passphrase: locked -> unlocked.
    let unlocked = get_ipc_response(
        &webview,
        invoke_request(
            "unlock",
            json!({ "input": { "passphrase": "correct horse battery" } }),
        ),
    )
    .expect("unlock should succeed")
    .deserialize::<serde_json::Value>()
    .unwrap();
    assert_eq!(unlocked["state"], json!("unlocked"));
}

/// dev-plan W29 "Tests first: capability fixture denies read/HTTP/shell; shims
/// round-trip a command already tested in-process" — this is the dispatcher-gate half:
/// prove the **Tauri-layer** gate is real (not decorative) by calling a registered
/// command shim in a state `SESSION_TABLE` forbids and confirming `not_in_session` comes
/// back through the actual IPC path, not just from `SessionManager`'s own internal check.
#[test]
fn calling_create_account_when_already_unlocked_is_rejected_by_the_tauri_gate() {
    let app = fresh_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("build mock webview");

    get_ipc_response(
        &webview,
        invoke_request(
            "create_account",
            json!({ "input": { "display_name": "Alex", "passphrase": "correct horse battery" } }),
        ),
    )
    .expect("first create_account should succeed");

    // Session is now "unlocked"; SESSION_TABLE only allows create_account in "first_run".
    let second = get_ipc_response(
        &webview,
        invoke_request(
            "create_account",
            json!({ "input": { "display_name": "Bo", "passphrase": "another passphrase" } }),
        ),
    );
    let err = second.expect_err("create_account while unlocked must be rejected");
    assert!(
        err.to_string().contains("not_in_session"),
        "expected not_in_session, got: {err}"
    );
}

/// Sanity check that the harness's mutex is actually shared (not a fresh `AppState` per
/// call), so the round-trip tests above are exercising one continuous session, matching
/// how `main.rs` manages exactly one `AppState` for the process lifetime.
#[test]
fn app_state_is_a_single_shared_mutex() {
    let app = fresh_app();
    let a = app.state::<AppState>().0.clone();
    let b = app.state::<AppState>().0.clone();
    assert!(Arc::ptr_eq(&a, &b));
    // Touch the mutex once so this test also fails loudly if locking ever panics.
    let _guard: std::sync::MutexGuard<'_, SessionManager> = a.lock().unwrap();
    drop(_guard);
    let _ = Mutex::new(()); // keep `Mutex` import used across cfg permutations
}

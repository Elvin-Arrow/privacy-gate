# [0041] W29 — Tauri IPC, CSP, events

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Wire `src-tauri/` up to `pg-core`'s already-tested `SessionManager`: 32 thin `#[tauri::command]`
shims over api.md §5, a capability ACL that structurally denies filesystem read/HTTP/shell/dialog
open, the CSP byte-for-byte from ui.md §3.1, and `pg://detect-progress` / `pg://session-changed`
event forwarding. Not mutation-gated (dev-plan §3), but still needs real tests: a capability/CSP
fixture and an IPC round-trip proof.

## Implementation

- `src-tauri/src/state.rs` (new): `AppState(Arc<Mutex<SessionManager>>)`, the "W29 wraps one of
  these in a Tauri managed `Mutex`" seam `SessionManager`'s own doc comment anticipated. `Arc`
  rather than a bare `Mutex` so `import_document` can clone the handle into `spawn_blocking`
  without borrowing `State` across an `.await`.
- `src-tauri/src/progress.rs` (new): `TauriProgressSink<R: Runtime>` implements
  `pg_core::session::ProgressSink` by forwarding to `AppHandle::emit` under the core's own
  `DETECT_PROGRESS_EVENT` constant — no new event-name string, no payload reshaping.
- `src-tauri/src/commands.rs` (new): one function per api.md §5 command. A single dispatcher gate
  (`fn gate`) calls `pg_core::session::command_allowed(name, mgr.get_session_state().state)` before
  every gated command's `SessionManager` call — belt-and-suspenders on top of the identical check
  `SessionManager` already runs internally, both reading the one `SESSION_TABLE` so they cannot
  disagree. Two macros (`cmd0`/`cmd1`) cover the 27 commands that are "lock, gate, call, return" (no
  input or one input DTO); `get_session_state` has no gate (api.md §2: callable in every state);
  `create_account`/`unlock`/`lock` are hand-written to also emit `pg://session-changed`;
  `import_document` is hand-written as the one **async** command. `COMMAND_NAMES` (all 32, api.md §5
  order) is the single source both `main.rs`'s `generate_handler!` list and the capability fixture
  test check themselves against, plus `build.rs`'s independent `APP_COMMANDS` array (can't share a
  `const` across the build-script/crate boundary, so a fixture test asserts the two lists agree in
  length/content by hand-checking `capabilities/default.json` against `COMMAND_NAMES`).
- `src-tauri/src/main.rs`: registers `tauri_plugin_dialog`/`tauri_plugin_fs`, resolves
  `app.path().app_data_dir()` (architecture §4.1: "exact path is implementation"), wires the real
  production backends — `pg_core::keystore::select_backend` (W7's OS/Linux-fallback probe, not
  `InMemoryKeystore`), `SqlCipherVault` coerced to all five vault-backed traits (same one-connection
  pattern `core/tests/audit_list_w28.rs` already uses), and `TauriProgressSink` — into one
  `SessionManager::new_full(..).with_documents(..).with_plugin_secrets(..).with_progress_sink(..)`.
  Deliberately **no** `.with_detector(..)` override: production import uses W15c's per-detect
  `pg-hybrid-v1`/Ollama selection; `with_detector(StubDetector)` stays test-only.
- `src-tauri/build.rs`: declares `tauri_build::AppManifest::new().commands(APP_COMMANDS)` — see
  "Ambiguity: does the ACL even check app commands?" below for why this line is load-bearing, not
  boilerplate.
- `src-tauri/capabilities/default.json`: rewritten to the ui.md §3.2 grant/deny list —
  `allow-$command` (hyphen-slugified) for all 32 commands, `dialog:allow-save` (never `allow-open`,
  explicitly denied too), `fs:allow-write-file` + `fs:allow-write-text-file` (never `allow-read`,
  `allow-read-dir`, `allow-read-file`, `allow-remove`, `allow-exists`, `allow-watch` — all six
  explicitly denied), `http:default` and all `shell:*` denied, nothing from `http:`/`shell:` ever
  granted.
- `src-tauri/tauri.conf.json`: CSP was already byte-identical to ui.md §3.1 from W0; a fixture test
  now pins that so a future edit can't drift it silently.
- `core/src/session.rs`: added `pub const SESSION_CHANGED_EVENT: &str = "pg://session-changed";`
  alongside the existing `DETECT_PROGRESS_EVENT` — the one non-shim addition to `pg-core` in this
  chunk, a constant (not logic) so the Tauri layer and any future in-process test share one string
  instead of two independently typed literals.

## Tests

- `src-tauri/src/capability_fixture.rs` (new, `#[cfg(test)]`): parses `capabilities/default.json`
  and `tauri.conf.json` via `include_str!` + `serde_json` and asserts, field-for-field: CSP matches
  ui.md §3.1 byte-for-byte and has no `https:` in `connect-src`; every one of `COMMAND_NAMES`' 32
  commands has its `allow-*` permission granted; `core:event:allow-listen` is granted;
  `dialog:allow-save` is granted and `dialog:allow-open` is neither granted nor missing from `deny`;
  `fs:allow-write-file`/`fs:allow-write-text-file` are granted and the six read/remove/exists/watch
  identifiers are neither granted nor missing from `deny`; no `http:`/`shell:` permission is ever
  granted and both are denied. This is the dev-plan's explicitly named "capability fixture denies
  read/HTTP/shell" test — it fails on any accidental ACL widening, not just on review.
- `src-tauri/src/ipc_roundtrip_tests.rs` (new, `#[cfg(test)]`): uses `tauri::test::mock_builder` +
  `tauri::generate_context!()` (not `mock_context`, which carries no capabilities at all — using it
  would make every command fail as "not allowed" for a reason unrelated to the dispatcher gate this
  file exists to test) to register the real `commands::*` functions via the same
  `tauri::generate_handler!` macro `main.rs` uses, then drive them over `get_ipc_response` with a
  `SessionManager::new(InMemoryKeystore, InMemoryAccountStore)`-backed `AppState`. Covers:
  `get_session_state` before any account; `create_account` → `get_session_state` → `lock` →
  `unlock` (wrong then right passphrase) round-tripping through the actual IPC path end to end; and
  — the dispatcher-gate proof dev-plan asks for by name — calling `create_account` a second time
  while already unlocked and confirming `not_in_session` comes back through the Tauri layer, not
  just from a direct `SessionManager` call.
- `cargo test --workspace` (via `--jobs 2`; see ambiguity below): 416 `pg-core` tests unchanged, 0
  regressions, plus 13 new `privacy-gate` tests, all green.

## Ambiguities resolved

- **Does the Tauri 2 ACL even check app-defined (non-plugin) commands?** Not obviously yes: reading
  `tauri-2.11.5/src/webview/mod.rs`'s invoke handling showed the ACL is skipped entirely for local,
  non-plugin commands unless `has_app_acl_manifest` is true — and that flag is only true if
  `build.rs` declares an `AppManifest` with a non-empty `commands` list. Without that declaration,
  `capabilities/default.json` would have been decorative for all 32 app commands (Tauri would allow
  them regardless of the file's contents), which is exactly the "capability ACL silently fails to
  grant/deny as intended" risk this chunk was warned about. Resolved by adding `build.rs`'s
  `AppManifest::new().commands(APP_COMMANDS)`, which autogenerates `allow-$command`/`deny-$command`
  permissions (slug: `_` → `-`) that `capabilities/default.json` then grants by name — confirmed by
  reading `tauri-build-2.6.3/src/acl.rs`'s `autogenerate_command_permissions` for the exact slug
  format, and proven live by `ipc_roundtrip_tests.rs` initially failing with "not allowed" before
  `build.rs` existed, then passing after.
- **`fs:allow-write` vs `fs:allow-write-file`:** the W0-era capability file granted `fs:allow-write`,
  which (per `tauri-plugin-fs-2.5.1`'s autogenerated permission files) only enables the low-level
  `write` command (writes to an already-open file descriptor by `rid`) — not the `writeFile`/
  `writeTextFile` JS API ui.md §3.2 names explicitly, which need `fs:allow-write-file` /
  `fs:allow-write-text-file`. Switched to the latter two; `fs:allow-write-file`'s own permission
  definition also grants the `open` command (needed to obtain a write handle at all), which is safe
  here only because `read`/`read_file` stay denied — `open` alone cannot exfiltrate content back to
  the webview.
- **Dispatcher-gate placement:** confirmed `SessionManager` has no state-change callback/hook seam,
  so `pg://session-changed` is emitted at exactly the three Tauri-shim call sites that can change
  `SessionState` per `SESSION_TABLE` (`create_account`, `unlock`, `lock`) rather than threading a
  sink through the core, per the brief's suggested resolution. The dispatcher gate itself
  (`fn gate` in `commands.rs`) is the "single gate in the command dispatcher" dev-plan W4 asked W29
  to add — it re-reads the same `SESSION_TABLE`/`command_allowed` `SessionManager` already consults
  internally, so the two checks are structurally the same check run twice, not two definitions of
  the gating table.
- **Tauri test-harness availability:** `tauri::test` (feature `test`) exists in the installed
  2.11.5 and includes `mock_builder`/`get_ipc_response`/`MockRuntime`, so the full "real command
  registration, real IPC path, real ACL" round-trip was reachable — no fallback to testing the gate
  as a decoupled plain-Rust function was needed. The one non-obvious detail: `get_ipc_response`'s
  "local origin" check depends on the request URL matching the platform's IPC scheme
  (`tauri://localhost` on Linux/macOS, `http://tauri.localhost` on Windows/Android) — using the
  wrong one on Linux made every command resolve as a remote origin and fail ACL matching for
  reasons unrelated to the dispatcher gate, which the harness now branches on via `cfg!(windows,
  target_os = "android")` the same way `tauri::test`'s own doctest does.
- **App-data-dir path:** `AppHandle::path().app_data_dir()` (Tauri 2's `Manager::path()` /
  `PathResolver`), matching architecture §4.1's "platform app-data directory (exact path is
  implementation)"; `vault.db` is created under it, `create_dir_all`'d first since a fresh install
  has no such directory yet.
- **`cargo test --workspace` OOM:** the default full-parallelism build linked `tauri-runtime-wry`
  (real GTK/WebKit2GTK, pulled in because `tauri-plugin-dialog`/`tauri-plugin-fs`'s default features
  include the real runtime even for the `test`-feature build) and got OOM-killed (`ld` SIGKILL) in
  the 7.75 GiB dev container under full job parallelism. `cargo test --workspace --jobs 2` links
  successfully; noted here rather than changed silently in `Makefile`/CI since it's a container
  resource constraint, not a code issue — future chunks touching `src-tauri` should expect the same
  and pass `--jobs 2` (or fewer) locally if `cargo test --workspace` OOMs.

## Traceability

- api.md §2 (session model), §3 (error model), §4 (DTOs), §5 (all 32 commands), §6 (events), §8
  (capability allowlist).
- architecture.md §12 (IPC/OS capabilities, C-ARCH-2), §2.3 (trust boundary table), §4.1 (app-data
  location).
- ui.md §3.1 (CSP), §3.2 (Tauri capabilities).
- dev-plan.md W29 ("Tauri command shims (thin; not mutation-gated) … Capabilities per api.md §8 and
  ui.md §3 … Dialog save only … `plugin-fs` write only").

Next: W30 — UI: first run, lock, unlock (`src/`, real `invoke` against these commands).

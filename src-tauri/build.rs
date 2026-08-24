// W29: declare the app's own command ACL manifest so the capability file's `allow-*`
// permissions are actually enforced (Tauri 2 only ACL-checks app-defined commands once an
// `AppManifest` with a non-empty command list exists — see
// `docs/dev-log/0041-w29-tauri-ipc.md` for how this was confirmed against the installed
// tauri/tauri-build source). Each entry here autogenerates `allow-$command` /
// `deny-$command` permissions (command name with `_` slugified to `-`), which
// `capabilities/default.json` then grants by name.
//
// This list must exactly match `src-tauri/src/commands.rs`'s `COMMAND_NAMES` — the
// registered `tauri::generate_handler!` set — and api.md §5's 32 commands. A command
// present in one but not the other is either unreachable (registered but denied) or
// silently unrestricted (denied here but callable anyway), so keep them in lockstep.

const APP_COMMANDS: &[&str] = &[
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

fn main() {
    let attributes = tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(APP_COMMANDS),
    );
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

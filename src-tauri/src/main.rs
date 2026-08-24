// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .setup(|_app| {
            // App initialization happens here
            // Commands and handlers will be registered in later chunks (W2+)
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

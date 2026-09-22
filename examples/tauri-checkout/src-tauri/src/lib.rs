//! The example application: a Tauri v2 app whose only job is to consume
//! `tauri-plugin-vpay-checkout`.
//!
//! There is no `#[tauri::command]` here and no application state. The whole
//! payment surface is the plugin's two commands, `plugin:vpay-checkout|show`
//! and `plugin:vpay-checkout|dismiss`, which the front end reaches through
//! `@vaam-apps/vpay-tauri-checkout`. This file exists to register the plugin
//! and to be compiled — on desktop by `cargo build`, on Android as a
//! `cdylib` loaded by the generated Gradle project, and on iOS as a
//! `staticlib` linked into the generated Xcode project. Those three
//! compilations are the evidence this example was written for.

/// Builds and runs the application.
///
/// `#[cfg_attr(mobile, tauri::mobile_entry_point)]` is what makes this the
/// symbol the generated Android and iOS hosts call; on desktop `main.rs`
/// calls it directly.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_vpay_checkout::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

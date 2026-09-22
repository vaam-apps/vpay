//! Desktop entry point.
//!
//! Everything is in the library so that the mobile entry point
//! (`#[tauri::mobile_entry_point]` in `lib.rs`) and this one run the same
//! code. The `cfg_attr` is what `cargo tauri init` generates: it stops a
//! Windows release build opening a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    vpay_example_tauri_checkout_lib::run()
}

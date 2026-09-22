//! Build script for the example application.
//!
//! `tauri_build::build()` is what reads `tauri.conf.json`, resolves every
//! plugin's `permissions/` directory (including
//! `tauri-plugin-vpay-checkout`'s, through the `links =
//! "tauri-plugin-vpay-checkout"` key in its manifest), checks this app's
//! `capabilities/*.json` against them, and writes `gen/schemas/`. A
//! capability naming a permission no linked plugin defines is a build
//! failure here rather than a runtime one — which is exactly what makes
//! this example evidence that `vpay-checkout:default` resolves.
fn main() {
    tauri_build::build()
}

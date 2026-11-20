//! Build script: generates the Tauri context and the app ACL manifest.
//!
//! Listing the commands in an [`AppManifest`] makes Tauri generate `allow-<command>` /
//! `deny-<command>` permissions for them and enforce the ACL on app commands, so every command
//! must be granted explicitly in `capabilities/`.

use tauri_build::{AppManifest, Attributes};

/// Every `#[tauri::command]` (src/commands and src/menu.rs); keep in sync with `generate_handler!` in `lib.rs`.
const COMMANDS: &[&str] = &[
    "app_info",
    "update_check",
    "update_install",
    "app_relaunch",
    "menu_subscribe",
];

fn main() {
    let attributes = Attributes::new().app_manifest(AppManifest::new().commands(COMMANDS));

    if let Err(error) = tauri_build::try_build(attributes) {
        // Build scripts report failures by panicking; `{error:#}` keeps the whole cause chain.
        panic!("tauri build script failed: {error:#}");
    }
}

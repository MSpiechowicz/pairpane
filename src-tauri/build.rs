//! Build script: generates the Tauri context and the app ACL manifest.
//!
//! Listing the commands in an [`AppManifest`] makes Tauri generate `allow-<command>` /
//! `deny-<command>` permissions for them and enforce the ACL on app commands, so every command
//! must be granted explicitly in `capabilities/`.

use tauri_build::{AppManifest, Attributes};

/// Every `#[tauri::command]` (src/commands and src/menu.rs); keep in sync with `generate_handler!` in `lib.rs`.
const COMMANDS: &[&str] = &[
    "app_info",
    "app_quit",
    "update_check",
    "update_install",
    "app_relaunch",
    "menu_subscribe",
    "pane_open",
    "archive_open",
    "archive_password",
    "remote_hosts",
    "remote_connect",
    "remote_open",
    "remote_disconnect",
    "pane_folder_size",
    "shell_run",
    "shell_stop",
    "terminal_open",
    "find_start",
    "find_cancel",
    "find_forget",
    "find_open",
    "compare_start",
    "compare_cancel",
    "compare_forget",
    "volumes_list",
    "state_load",
    "state_save",
    "ops_subscribe",
    "ops_start",
    "ops_remote",
    "ops_extract",
    "ops_pack",
    "ops_add_to_archive",
    "ops_control",
    "ops_answer",
    "ops_rename",
    "ops_trash",
    "ops_untrash",
    "ops_delete",
    "ops_mkdir",
    "ops_mkfile",
    "ops_link",
    "ops_task",
    "ops_rename_undo",
    "ops_sync_undo",
    "ops_sync",
    "rename_preview",
    "ops_properties",
    "view_open",
    "archive_view_open",
    "archive_edit_open",
    "view_close",
    "view_bytes",
    "view_text",
    "view_search",
    "view_search_cancel",
    "diff_next_difference",
    "view_markdown",
    "view_archive",
    "view_image_decode",
    "view_media",
    "thumb_get",
    "columns_dimensions",
    "columns_checksum",
    "columns_git",
    "edit_open",
    "edit_save",
    "edit_external",
    "settings_open",
    "settings_editor",
    "editor_set_dirty",
    "editor_close_ack",
    "app_close_confirmed",
    "devices_open",
    "devices_pair_begin",
    "devices_pair_cancel",
    "devices_join",
    "devices_discovered",
    "devices_status",
    "devices_rename",
    "devices_unpair",
];

fn main() {
    let attributes = Attributes::new().app_manifest(AppManifest::new().commands(COMMANDS));

    if let Err(error) = tauri_build::try_build(attributes) {
        // Build scripts report failures by panicking; `{error:#}` keeps the whole cause chain.
        panic!("tauri build script failed: {error:#}");
    }
}

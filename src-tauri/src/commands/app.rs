//! App-level commands: build information and quitting.

use pairpane_core::model::AppInfo;
use tauri::AppHandle;

/// Product name and version.
#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    let package = app.package_info();

    AppInfo::new(package.name.clone(), package.version.to_string())
}

/// Quits the app, also while its window is hidden in the tray. It goes through the exit request like the tray's Quit, so
/// the window is shown and unsaved text in the editor is asked about first (`close.rs`).
#[tauri::command]
pub fn app_quit(app: AppHandle) {
    app.exit(0);
}

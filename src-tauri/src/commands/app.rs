//! App-level commands: build information.

use pairpane_core::model::AppInfo;
use tauri::AppHandle;

/// Product name and version.
#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    let package = app.package_info();

    AppInfo::new(package.name.clone(), package.version.to_string())
}

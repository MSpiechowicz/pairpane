//! Pairpane: the Tauri shell. It hosts the UI and exposes `pairpane_core` through IPC commands;
//! all domain logic lives in the core crate.

mod commands;
mod logging;
mod window;

use std::process::ExitCode;

/// Builds and runs the app until the last window closes.
pub fn run() -> ExitCode {
    let result = tauri::Builder::default()
        // Must be registered first so a second launch exits before anything else starts.
        // Its argv is ignored: nothing from another process is forwarded to the webview.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::focus_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // Rust-side only: no JS opener permission is granted, so its injected link handler is off.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(logging::plugin())
        .setup(|app| {
            window::create_main_window(app)?;

            let version = app.package_info().version.to_string();
            tracing::info!(%version, "Pairpane started");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app::app_info])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Pairpane failed to run: {error}");
            ExitCode::FAILURE
        }
    }
}

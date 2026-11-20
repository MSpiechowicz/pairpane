//! Pairpane: the Tauri shell. It hosts the UI and exposes `pairpane_core` through IPC commands;
//! all domain logic lives in the core crate.

mod commands;
mod error;
mod logging;
mod menu;
mod updates;
mod window;

use std::process::ExitCode;

use tauri::Manager;

/// Builds and runs the app until the last window closes.
pub fn run() -> ExitCode {
    let builder = tauri::Builder::default()
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
        // Updates are driven from Rust only (commands/updates.rs); no JS updater permission is granted.
        .plugin(tauri_plugin_updater::Builder::new().build());

    let result = menu::attach(builder)
        .setup(|app| {
            app.manage(updates::Updates::from_config(app.config()));
            app.manage(menu::MenuEvents::default());
            app.manage(commands::panes::Panes::default());

            window::create_main_window(app)?;

            let version = app.package_info().version.to_string();
            tracing::info!(%version, "Pairpane started");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::updates::update_check,
            commands::updates::update_install,
            commands::updates::app_relaunch,
            menu::menu_subscribe,
            commands::panes::pane_open,
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Pairpane failed to run: {error}");
            ExitCode::FAILURE
        }
    }
}

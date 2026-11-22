//! Pairpane: the Tauri shell. It hosts the UI and exposes `pairpane_core` through IPC commands;
//! all domain logic lives in the core crate.

// The shell never needs `unsafe` (the workspace only denies it, for one module of the core crate).
#![forbid(unsafe_code)]

mod close;
mod commands;
mod error;
mod logging;
mod menu;
mod updates;
mod window;

use std::process::ExitCode;

use tauri::{Manager, RunEvent};

/// Builds and runs the app until the last window closes; the searches and the transfers still running are cancelled, and given up to two seconds to clean up, as it exits, and the viewer's sessions are ended (which stops the HEIC helper that runs) and its temporary archive files are swept.
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
        // Rust-side only: it shows the question before an external editor program is started (commands/edit). No dialog
        // permission is granted, so the page cannot call it, and `confirm` and `alert` of the page do nothing.
        .plugin(tauri_plugin_dialog::init())
        // Updates are driven from Rust only (commands/updates.rs); no JS updater permission is granted.
        .plugin(tauri_plugin_updater::Builder::new().build());

    let result = menu::attach(builder)
        // A close with unsaved text in the editor waits for the page's answer (close.rs).
        .on_window_event(close::on_window_event)
        .setup(|app| {
            app.manage(updates::Updates::from_config(app.config()));
            app.manage(menu::MenuEvents::default());
            app.manage(commands::panes::Panes::default());
            app.manage(commands::volumes::Volumes::default());
            app.manage(commands::ops::Ops::default());
            app.manage(commands::items::Items::default());
            app.manage(commands::renames::Renames::default());
            app.manage(commands::view::Viewer::default());
            app.manage(commands::archives::Archives::default());
            app.manage(commands::find::Finder::default());
            app.manage(commands::compare::Comparer::default());
            // Files an earlier run left in the temporary folder (an entry opened in the viewer is deleted at once).
            tauri::async_runtime::spawn_blocking(commands::view::archive_entry::sweep);
            // The same for the staging folders of an unpacking that a crash left (a RAR unpacked by `unrar`).
            tauri::async_runtime::spawn_blocking(pairpane_core::archive::rar::stage::sweep);
            app.manage(close::CloseGuard::default());
            // Next to the window-state plugin's file. Without a config folder nothing is saved.
            let state_file = app
                .path()
                .app_config_dir()
                .ok()
                .map(|dir| dir.join(commands::state::STATE_FILE));
            app.manage(commands::state::StateStore::new(state_file));
            // The user's settings file, in the same folder. Without a config folder there is no external editor.
            let settings_file = app
                .path()
                .app_config_dir()
                .ok()
                .map(|dir| dir.join(pairpane_core::settings::SETTINGS_FILE));
            app.manage(commands::edit::Editor::new(settings_file));

            window::create_main_window(app)?;

            let version = app.package_info().version.to_string();
            tracing::info!(%version, "Pairpane started");

            Ok(())
        })
        // The media of the viewer (audio and video) is served from the files it holds open, by token (commands/view/serve.rs).
        .register_asynchronous_uri_scheme_protocol(commands::view::serve::SCHEME, |context, request, responder| {
            commands::view::serve::handle(context.app_handle(), &request, responder);
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::updates::update_check,
            commands::updates::update_install,
            commands::updates::app_relaunch,
            menu::menu_subscribe,
            commands::panes::pane_open,
            commands::panes::archive::archive_open,
            commands::archives::archive_password,
            commands::sizes::pane_folder_size,
            commands::find::find_start,
            commands::find::find_cancel,
            commands::find::find_forget,
            commands::find::list::find_open,
            commands::compare::compare_start,
            commands::compare::compare_cancel,
            commands::compare::compare_forget,
            commands::volumes::volumes_list,
            commands::state::state_load,
            commands::state::state_save,
            commands::ops::ops_subscribe,
            commands::ops::ops_start,
            commands::ops::extract::ops_extract,
            commands::ops::pack::ops_pack,
            commands::ops::add::ops_add_to_archive,
            commands::ops::ops_control,
            commands::ops::ops_answer,
            commands::ops::ops_rename,
            commands::ops::tasks::ops_task,
            commands::ops::ops_rename_undo,
            commands::compare::ops_sync,
            commands::renames::rename_preview,
            commands::items::ops_trash,
            commands::items::ops_untrash,
            commands::items::ops_delete,
            commands::items::ops_mkdir,
            commands::items::ops_mkfile,
            commands::items::ops_link,
            commands::items::props::ops_properties,
            commands::view::view_open,
            commands::view::archive_entry::archive_view_open,
            commands::view::view_close,
            commands::view::read::view_bytes,
            commands::view::read::view_text,
            commands::view::read::view_markdown,
            commands::view::read::view_archive,
            commands::view::read::view_image_decode,
            commands::view::search::view_search,
            commands::view::search::view_search_cancel,
            commands::view::media::view_media,
            commands::edit::edit_open,
            commands::edit::edit_save,
            commands::edit::edit_external,
            commands::edit::settings_open,
            close::editor_set_dirty,
            close::editor_close_ack,
            close::app_close_confirmed,
        ])
        .build(tauri::generate_context!())
        .map(|app| {
            app.run(|app, event| {
                // An exit with unsaved text in the editor waits for the page's answer, unless it is a restart.
                close::on_run_event(app, &event);

                // Cancel the jobs, silently, and give each a moment to remove its partial file (the plan's decision 8).
                if let RunEvent::Exit = event {
                    // First: a search that runs stops its walk (it may be on a slow disk) before anything waits.
                    if let Some(finder) = app.try_state::<commands::find::Finder>() {
                        finder.shut_down();
                    }
                    if let Some(comparer) = app.try_state::<commands::compare::Comparer>() {
                        comparer.shut_down();
                    }
                    // An archive read that nobody cancels (a file opened in the viewer, a password check, the
                    // checks before an extraction) stops, so none starts `unrar` again while the staging folders go.
                    if let Some(archives) = app.try_state::<commands::archives::Archives>() {
                        archives.shut_down();
                    }
                    if let Some(ops) = app.try_state::<commands::ops::Ops>() {
                        ops.shutdown();
                    }
                    // A HEIC decode that runs stops its helper process when its session ends.
                    if let Some(viewer) = app.try_state::<commands::view::Viewer>() {
                        viewer.close_all();
                    }
                    commands::view::archive_entry::sweep();
                    pairpane_core::archive::rar::stage::sweep_on_exit();
                }
            });
        });

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Pairpane failed to run: {error}");
            ExitCode::FAILURE
        }
    }
}

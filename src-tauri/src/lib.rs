//! Pairpane: the Tauri shell. It hosts the UI and exposes `pairpane_core` through IPC commands;
//! all domain logic lives in the core crate.

// The shell never needs `unsafe` (the workspace only denies it, for one module of the core crate).
#![forbid(unsafe_code)]

mod close;
mod commands;
mod error;
mod logging;
mod menu;
mod tray;
mod updates;
mod window;

use std::process::ExitCode;
use std::sync::Arc;

use commands::devices::{LinkService, device_name, host_name, paired_on_disk};
use tauri::{Manager, RunEvent, Wry};

/// Manages the device link. It starts now only when devices are paired already (they must be answered); a fresh install
/// does not listen until the Devices screen is opened.
fn manage_devices(app: &mut tauri::App) {
    // Without a data folder the link never starts.
    let dir = app.path().app_data_dir().ok();
    let paired = dir.as_deref().is_some_and(paired_on_disk);
    let name = device_name(host_name().as_deref());

    // When the last device is gone, nothing keeps the app alive in the tray: the window comes back (on the main thread,
    // where the tray is).
    let on_empty = app.handle().clone();
    app.manage(LinkService::new(
        dir,
        name,
        Arc::new(move || {
            let app = on_empty.clone();
            if let Err(error) = on_empty.run_on_main_thread(move || tray::reveal_if_hidden(&app)) {
                tracing::warn!(%error, "cannot show the window");
            }
        }),
    ));

    if !paired {
        return;
    }

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let devices = handle.state::<LinkService>();
        if let Err(error) = devices.ensure_started().await {
            tracing::warn!(kind = error.kind(), "the device link did not start");
        }
    });
}

/// Builds and runs the app until the last window closes; the searches and the transfers still running are cancelled, and given up to two seconds to clean up, as it exits, and the viewer's sessions are ended (which stops the HEIC helper that runs) and its temporary archive files are swept.
pub fn run() -> ExitCode {
    let builder = tauri::Builder::default()
        // Must be registered first so a second launch exits before anything else starts.
        // Its argv is ignored: nothing from another process is forwarded to the webview.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Also from the tray, where the window is hidden.
            tray::reveal(app);
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
            app.manage(commands::thumbs::Thumbs::default());
            app.manage(commands::columns::Columns::default());
            app.manage(commands::archives::Archives::default());
            // Starts the runtime of the SFTP sites; without one the app runs and the remote commands refuse.
            app.manage(commands::remote::Remotes::start());
            app.manage(commands::find::Finder::default());
            app.manage(commands::compare::Comparer::default());
            app.manage(commands::shell::Shells::default());
            // Files an earlier run left in the temporary folder (an entry opened in the viewer is deleted at once).
            tauri::async_runtime::spawn_blocking(commands::view::archive_entry::sweep);
            // The same for the staging folders of an unpacking that a crash left (a RAR unpacked by `unrar`).
            tauri::async_runtime::spawn_blocking(pairpane_core::archive::rar::stage::sweep);
            app.manage(close::CloseGuard::default());
            app.manage(tray::Tray::<Wry>::default());
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

            manage_devices(app);
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
            commands::app::app_quit,
            commands::updates::update_check,
            commands::updates::update_install,
            commands::updates::app_relaunch,
            menu::menu_subscribe,
            commands::panes::pane_open,
            commands::panes::archive::archive_open,
            commands::archives::archive_password,
            commands::remote::remote_hosts,
            commands::remote::remote_connect,
            commands::panes::remote::remote_open,
            commands::remote::remote_disconnect,
            commands::sizes::pane_folder_size,
            commands::shell::shell_run,
            commands::shell::shell_stop,
            commands::shell::terminal_open,
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
            commands::ops::ops_sync_undo,
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
            commands::view::archive_entry::archive_edit_open,
            commands::view::view_close,
            commands::view::read::view_bytes,
            commands::view::read::view_text,
            commands::view::read::view_markdown,
            commands::view::read::view_archive,
            commands::view::read::view_image_decode,
            commands::view::search::view_search,
            commands::view::search::view_search_cancel,
            commands::view::diff::diff_next_difference,
            commands::view::media::view_media,
            commands::thumbs::thumb_get,
            commands::columns::columns_dimensions,
            commands::columns::columns_checksum,
            commands::columns::columns_git,
            commands::edit::edit_open,
            commands::edit::edit_save,
            commands::edit::edit_external,
            commands::edit::settings_open,
            commands::edit::setting::settings_editor,
            commands::devices::devices_open,
            commands::devices::devices_pair_begin,
            commands::devices::devices_pair_cancel,
            commands::devices::devices_join,
            commands::devices::devices_discovered,
            commands::devices::devices_status,
            commands::devices::devices_rename,
            commands::devices::devices_unpair,
            close::editor_set_dirty,
            close::editor_close_ack,
            close::app_close_confirmed,
        ])
        .build(tauri::generate_context!())
        .map(|app| {
            app.run(|app, event| {
                // An exit with unsaved text in the editor waits for the page's answer, unless it is a restart.
                close::on_run_event(app, &event);
                // A window that is hidden in the tray keeps the app running; the dock shows it again.
                tray::on_run_event(app, &event);

                // Cancel the jobs, silently, and give each a moment to remove its partial file (the plan's decision 8).
                if let RunEvent::Exit = event {
                    // First: a search that runs stops its walk (it may be on a slow disk) before anything waits.
                    if let Some(finder) = app.try_state::<commands::find::Finder>() {
                        finder.shut_down();
                    }
                    if let Some(comparer) = app.try_state::<commands::compare::Comparer>() {
                        comparer.shut_down();
                    }
                    // A command of the command line ends with what it started: none outlives the app.
                    if let Some(shells) = app.try_state::<commands::shell::Shells>() {
                        shells.shut_down();
                    }
                    // An archive read that nobody cancels (a file opened in the viewer, a password check, the
                    // checks before an extraction) stops, so none starts `unrar` again while the staging folders go.
                    if let Some(archives) = app.try_state::<commands::archives::Archives>() {
                        archives.shut_down();
                    }
                    if let Some(ops) = app.try_state::<commands::ops::Ops>() {
                        ops.shutdown();
                    }
                    // The connections to servers say goodbye together and the runtime that serves them stops, in about a second at the most.
                    if let Some(remotes) = app.try_state::<commands::remote::Remotes>() {
                        remotes.shut_down();
                    }
                    // A thumbnail that is being made stops its helper process, and none starts after this.
                    if let Some(thumbs) = app.try_state::<commands::thumbs::Thumbs>() {
                        thumbs.shut_down();
                    }
                    // A checksum that is being computed stops between its chunks, and none starts after this.
                    if let Some(columns) = app.try_state::<commands::columns::Columns>() {
                        columns.shut_down();
                    }
                    // A HEIC decode that runs stops its helper process when its session ends.
                    if let Some(viewer) = app.try_state::<commands::view::Viewer>() {
                        viewer.close_all();
                    }
                    // The device link closes its connections, but is not waited for longer than that.
                    if let Some(devices) = app.try_state::<LinkService>() {
                        // The timer is made inside the runtime: a timeout made outside it panics.
                        tauri::async_runtime::block_on(async {
                            tokio::time::timeout(commands::devices::SHUTDOWN_WAIT, devices.shutdown())
                                .await
                                .ok();
                        });
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

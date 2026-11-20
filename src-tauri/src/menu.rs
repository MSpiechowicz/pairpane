//! The native app menu: "Pairpane > Check for Updates…" on macOS, "Help > Check for Updates…" on Windows and
//! Linux.
//!
//! Menu clicks reach the UI over a channel the UI registers once with `menu_subscribe`, so the webview needs no
//! event permission: it still only calls the app's own commands.

use std::sync::{Mutex, PoisonError};

use pairpane_core::model::MenuCommand;
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuEvent, MenuItem, Submenu};
use tauri::{AppHandle, Manager, Runtime, State};

/// Menu item id of "Check for Updates…".
const CHECK_FOR_UPDATES: &str = "check-for-updates";

/// The UI's menu channel (the latest one it registered).
#[derive(Default)]
pub struct MenuEvents {
    channel: Mutex<Option<Channel<MenuCommand>>>,
}

impl MenuEvents {
    fn send(&self, command: MenuCommand) {
        let channel = self.channel.lock().unwrap_or_else(PoisonError::into_inner);

        match channel.as_ref() {
            // A closed channel only means the page went away; it registers again when it loads.
            Some(channel) => {
                channel.send(command).ok();
            }
            None => tracing::warn!(?command, "menu command before the UI subscribed; ignored"),
        }
    }
}

/// Registers the channel menu commands are sent to. The UI calls it once when it loads; a later call replaces
/// the earlier channel (e.g. after a reload).
#[tauri::command]
pub fn menu_subscribe(events: State<'_, MenuEvents>, on_command: Channel<MenuCommand>) {
    *events.channel.lock().unwrap_or_else(PoisonError::into_inner) = Some(on_command);
}

/// Builds the app menu.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let check = MenuItem::with_id(app, CHECK_FOR_UPDATES, "Check for Updates…", true, None::<&str>)?;

    if cfg!(target_os = "macos") {
        // macOS needs the standard menus (Edit for copy/paste, Window, ...): keep them and add the item to the
        // app menu right under "About" (the separator that follows "About" then follows it).
        let menu = Menu::default(app)?;

        if let Some(app_menu) = menu.items()?.first().and_then(|item| item.as_submenu().cloned()) {
            app_menu.insert(&check, 1)?;
        }

        return Ok(menu);
    }

    let help = Submenu::with_items(app, "Help", true, &[&check])?;
    Menu::with_items(app, &[&help])
}

/// Forwards a menu click to the UI.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == CHECK_FOR_UPDATES {
        app.state::<MenuEvents>().send(MenuCommand::CheckForUpdates);
    }
}

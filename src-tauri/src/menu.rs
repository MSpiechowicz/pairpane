//! The native app menu, macOS only: "Pairpane > Check for Updates…". Windows and Linux get no native menu bar:
//! the in-app top bar holds the menu there (its Settings menu has "Check for Updates…"), so it is never shown
//! twice.
//!
//! Menu clicks reach the UI over a channel the UI registers once with `menu_subscribe`, so the webview needs no
//! event permission: it still only calls the app's own commands.

use std::sync::{Mutex, PoisonError};

use pairpane_core::model::MenuCommand;
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::{AppHandle, Builder, Manager, Runtime, State};

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

/// Adds the native menu on macOS; elsewhere the builder is returned unchanged, so the window has no menu bar.
pub fn attach<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    if cfg!(target_os = "macos") {
        builder.menu(build).on_menu_event(on_event)
    } else {
        builder
    }
}

/// Builds the macOS app menu: the standard menus (Edit for copy/paste, Window, ...) plus "Check for Updates…" in
/// the app menu right under "About" (the separator that follows "About" then follows it).
fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let check = MenuItem::with_id(app, CHECK_FOR_UPDATES, "Check for Updates…", true, None::<&str>)?;
    let menu = Menu::default(app)?;

    if let Some(app_menu) = menu.items()?.first().and_then(|item| item.as_submenu().cloned()) {
        app_menu.insert(&check, 1)?;
    }

    Ok(menu)
}

/// Forwards a menu click to the UI.
fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == CHECK_FOR_UPDATES {
        app.state::<MenuEvents>().send(MenuCommand::CheckForUpdates);
    }
}

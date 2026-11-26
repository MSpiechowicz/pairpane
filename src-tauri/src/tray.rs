//! The tray icon that keeps Pairpane running when its window is closed while a device is paired, so the device can still
//! pair, and be reached, from the others. It is made the first time the window is hidden, never at launch, and only
//! on the main thread: the Linux tray library loads the system's `libappindicator` when it is first used and panics
//! when it is missing, so the build is guarded and a system without one has no tray (`TrayHealth::Unavailable`), which
//! makes closing the window quit as before.
//!
//! The window is hidden only when the link runs and at least one device is paired and the tray is there. Hiding ends any
//! pairing code that is offered. "Show Pairpane" (or a click on the icon, a second launch, or the dock on macOS) shows
//! the window again, and "Quit" exits through the same question as any quit (`close.rs`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, Runtime, Window};

use crate::commands::devices::LinkService;
use crate::window::{MAIN_WINDOW, focus_main_window};

const TRAY_ID: &str = "main";
const SHOW_ID: &str = "tray-show";
const QUIT_ID: &str = "tray-quit";

/// Whether the tray icon can be made on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayHealth {
    /// Not tried yet.
    Untried,
    Ready,
    /// The system has no tray (or the library for it is missing): the app quits when the window closes.
    Unavailable,
}

/// What an exit request that nobody asked for is to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    Prevent,
    Allow,
}

/// Whether closing the window hides it: a device is paired and the link runs (`keeps_alive`), and the tray is not known
/// to be missing.
fn should_hide(keeps_alive: bool, health: TrayHealth) -> bool {
    keeps_alive && health != TrayHealth::Unavailable
}

/// What an exit request does. Only one with no code (every window is gone) may be prevented, and only while the main
/// window is still there and would be hidden; a quit with a code (the tray's, the system's, a restart) always goes on.
fn exit_action(code: Option<i32>, window_alive: bool, hide: bool) -> ExitAction {
    if code.is_none() && window_alive && hide {
        ExitAction::Prevent
    } else {
        ExitAction::Allow
    }
}

/// Whether the window must be shown to be seen: it is hidden.
fn must_reveal(window_visible: bool) -> bool {
    !window_visible
}

/// The tray icon, once it is made, and whether it could be.
pub struct Tray<R: Runtime> {
    icon: Mutex<Option<TrayIcon<R>>>,
    health: Mutex<TrayHealth>,
}

impl<R: Runtime> Default for Tray<R> {
    fn default() -> Self {
        Self {
            icon: Mutex::new(None),
            health: Mutex::new(TrayHealth::Untried),
        }
    }
}

impl<R: Runtime> Tray<R> {
    fn health(&self) -> TrayHealth {
        *lock(&self.health)
    }

    /// Makes the icon when that has not been tried. Only the main thread may call it.
    fn ensure(&self, app: &AppHandle<R>) -> TrayHealth {
        let mut health = lock(&self.health);
        if *health != TrayHealth::Untried {
            return *health;
        }

        // The library may panic (not return an error) when the system's tray library is missing.
        match catch_unwind(AssertUnwindSafe(|| build(app))) {
            Ok(Ok(icon)) => {
                *lock(&self.icon) = Some(icon);
                *health = TrayHealth::Ready;
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "the tray icon cannot be made; closing the window quits");
                *health = TrayHealth::Unavailable;
            }
            Err(_) => {
                tracing::warn!("the system has no tray library; closing the window quits");
                *health = TrayHealth::Unavailable;
            }
        }

        *health
    }

    fn show_icon(&self, visible: bool) {
        if let Some(icon) = lock(&self.icon).as_ref()
            && let Err(error) = icon.set_visible(visible)
        {
            tracing::warn!(%error, "cannot change the tray icon");
        }
    }
}

fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<TrayIcon<R>> {
    let show = MenuItem::with_id(app, SHOW_ID, "Show Pairpane", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Pairpane")
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(on_icon_event);

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)
}

fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        SHOW_ID => reveal(app),
        // Through the exit request, so unsaved text in the editor is asked about (`close.rs`).
        QUIT_ID => app.exit(0),
        _ => {}
    }
}

fn on_icon_event<R: Runtime>(icon: &TrayIcon<R>, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        reveal(icon.app_handle());
    }
}

/// Shows and focuses the main window and takes the icon away: the window is the way to the app again.
pub fn reveal<R: Runtime>(app: &AppHandle<R>) {
    focus_main_window(app);

    if let Some(tray) = app.try_state::<Tray<R>>() {
        tray.show_icon(false);
    }
}

/// Shows the main window when it is hidden (in the tray), before something is asked in it.
pub fn reveal_if_hidden<R: Runtime>(app: &AppHandle<R>) {
    let visible = app
        .get_webview_window(MAIN_WINDOW)
        .is_some_and(|window| window.is_visible().unwrap_or(true));

    if must_reveal(visible) {
        reveal(app);
    }
}

/// Hides the window instead of closing it when the app is to stay in the tray; `true` when it did, and the close is
/// to be stopped. It runs on the main thread (the window event handler does), where the icon may be made.
pub fn hide_instead_of_close<R: Runtime>(window: &Window<R>) -> bool {
    let app = window.app_handle();
    if window.label() != MAIN_WINDOW {
        return false;
    }

    let (Some(service), Some(tray)) = (app.try_state::<LinkService>(), app.try_state::<Tray<R>>()) else {
        return false;
    };
    if !should_hide(service.keeps_app_alive(), tray.health()) {
        return false;
    }
    if tray.ensure(app) == TrayHealth::Unavailable {
        return false;
    }

    // A code is for a screen the user is looking at; nobody sees it from the tray.
    service.cancel_code();
    if let Err(error) = window.hide() {
        tracing::warn!(%error, "cannot hide the main window");

        return false;
    }
    tray.show_icon(true);

    true
}

/// Keeps the app running while the window is hidden, and shows the window when the system asks to (the dock icon on macOS).
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    match event {
        RunEvent::ExitRequested { code, api, .. } => {
            let window_alive = app.get_webview_window(MAIN_WINDOW).is_some();
            let hide = match (app.try_state::<LinkService>(), app.try_state::<Tray<R>>()) {
                (Some(service), Some(tray)) => should_hide(service.keeps_app_alive(), tray.health()),
                _ => false,
            };

            if exit_action(*code, window_alive, hide) == ExitAction::Prevent {
                api.prevent_exit();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => reveal(app),
        _ => {}
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;

//! Closing the window or quitting the app while the page has something to ask first: unsaved text in the editor or the
//! file comparison, or (when the user's preference says so) copies and moves that still run.
//!
//! The page tells the shell whether a close must be asked about (`editor_set_dirty`, named after its first use). When
//! the window is asked to close (or the app to exit) while it must, the shell stops it and sends the page
//! `MenuCommand::CloseRequested` over the menu channel; the page asks its questions (Save / Don't save / Cancel, Quit
//! anyway?) and, unless the user cancels, calls `app_close_confirmed`, which carries out what was stopped. Without a page that can answer (no channel, a closed one) the close goes ahead.
//!
//! A close that was stopped must never make the app impossible to close: a page that cannot answer (hung, or its
//! dialog lost) would leave it so. The page acknowledges a question the moment it receives it (`editor_close_ack`).
//! One that is not acknowledged within [`PATIENCE`] (a hung page) is given up, and the next close or exit goes ahead. One
//! that is acknowledged waits for the user as long as it takes: it ends only by the page's answer (`report(true)` for a
//! cancel, `app_close_confirmed`) or by a reset, so a user who reads the dialog for a minute is not cut off.
//!
//! Residual, accepted: a hostile page can still keep the window open by acknowledging and then answering every close with
//! "cancel" (`editor_set_dirty(true)`). The page is the app's own code and is not a trust boundary here; the system's
//! force quit remains.
//!
//! A restart after an update is not stopped: Tauri ignores `prevent_exit` for it, so the page asks before it calls
//! `app_relaunch`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use pairpane_core::model::MenuCommand;
use tauri::{AppHandle, Manager, RESTART_EXIT_CODE, RunEvent, Runtime, State, Window, WindowEvent};

use crate::menu::MenuEvents;

/// What was asked of the app and is waiting for the user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Close the window with this label.
    Window(String),
    /// Exit with this code.
    Exit(i32),
}

/// How long a stopped request waits for the page to acknowledge the question before the next one is let through.
const PATIENCE: Duration = Duration::from_secs(5);

/// A request that waits for the user: when the page was first asked, and whether it has acknowledged the question.
#[derive(Debug)]
struct Waiting {
    request: Request,
    asked: Instant,
    acknowledged: bool,
}

type Pending = Option<Waiting>;

/// Whether the editor buffer is dirty, and the request that waits for the user.
#[derive(Default)]
pub struct CloseGuard {
    dirty: AtomicBool,
    pending: Mutex<Pending>,
}

/// Whether `request` is to wait for the user: only while the buffer is dirty, and never a restart (which cannot be
/// stopped).
fn must_ask(dirty: bool, request: &Request) -> bool {
    dirty && *request != Request::Exit(RESTART_EXIT_CODE)
}

/// Whether `waiting` is to be given up at `now`: the page has not acknowledged the question for longer than [`PATIENCE`].
/// An acknowledged question is never given up.
fn given_up(waiting: &Waiting, now: Instant) -> bool {
    !waiting.acknowledged && now.saturating_duration_since(waiting.asked) > PATIENCE
}

impl CloseGuard {
    pub fn set_dirty(&self, dirty: bool) {
        self.dirty.store(dirty, Ordering::SeqCst);
    }

    /// The page has received the question of a stopped request and is putting it to the user: it is not given up for
    /// being slow any more. Does nothing when no request waits.
    pub fn acknowledge(&self) {
        if let Some(waiting) = self.pending().as_mut() {
            waiting.acknowledged = true;
        }
    }

    /// What the page reports of its buffer. A buffer that is still dirty is the page's answer "cancel" to a question it was
    /// asked (a Save or a Don't save is followed by `app_close_confirmed`, and a save reports a clean buffer), so what
    /// waited on it is forgotten: the next close is asked again, and is as new as the first.
    pub fn report(&self, dirty: bool) {
        if dirty {
            *self.pending() = None;
        }

        self.set_dirty(dirty);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    /// Forgets the buffer and what waited on it.
    pub fn reset(&self) {
        self.set_dirty(false);
        *self.pending() = None;
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `request` is to be stopped, at `now`. It is when the buffer is dirty and `ask` (which tells the page)
    /// says the page got the question; the request is kept for [`CloseGuard::confirm`]. Otherwise it goes ahead, and so
    /// does one that comes when the page was asked more than [`PATIENCE`] ago and has not acknowledged the question.
    pub fn intercept(&self, request: Request, now: Instant, ask: impl FnOnce() -> bool) -> bool {
        if !must_ask(self.is_dirty(), &request) {
            return false;
        }

        {
            let mut pending = self.pending();
            if pending.as_ref().is_some_and(|waiting| given_up(waiting, now)) {
                *pending = None;

                return false;
            }

            // Kept before the page is asked, so that an answer that comes at once finds it. A question that is asked
            // again keeps the time and the acknowledgement of the first, so that clicking on neither makes a hung page
            // wait for ever nor takes back what the page has acknowledged.
            let (asked, acknowledged) = pending
                .as_ref()
                .map_or((now, false), |waiting| (waiting.asked, waiting.acknowledged));
            *pending = Some(Waiting {
                request,
                asked,
                acknowledged,
            });
        }

        if ask() {
            return true;
        }

        *self.pending() = None;

        false
    }

    /// The request the user said to go ahead with, now that its buffer is saved or given up (so it is not dirty any
    /// more and the request is not stopped again); `None` when nothing waits.
    pub fn confirm(&self) -> Option<Request> {
        let waiting = self.pending().take()?;
        self.set_dirty(false);

        Some(waiting.request)
    }
}

/// Tells the shell whether the editor buffer has unsaved text. The page also sends `true` when the user cancels the
/// question of a close, so the next close is asked again as a new one.
#[tauri::command]
pub fn editor_set_dirty(close: State<'_, CloseGuard>, dirty: bool) {
    close.report(dirty);
}

/// The page received `MenuCommand::CloseRequested` and is asking the user: the close that waits is no longer given up for
/// being slow (see [`PATIENCE`]).
#[tauri::command]
pub fn editor_close_ack(close: State<'_, CloseGuard>) {
    close.acknowledge();
}

/// Goes ahead with the close or the exit that was stopped for unsaved text; does nothing when none waits.
#[tauri::command]
pub fn app_close_confirmed(app: AppHandle, close: State<'_, CloseGuard>) {
    match close.confirm() {
        Some(Request::Window(label)) => {
            if let Some(window) = app.get_webview_window(&label) {
                window.close().ok();
            }
        }
        Some(Request::Exit(code)) => app.exit(code),
        None => {}
    }
}

/// Asks the page before the window closes with unsaved text.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };

    // While a device is paired the window goes to the tray instead of closing (`tray.rs`); nothing is asked then.
    if crate::tray::hide_instead_of_close(window) {
        api.prevent_close();

        return;
    }

    let app = window.app_handle();
    let (Some(close), Some(events)) = (app.try_state::<CloseGuard>(), app.try_state::<MenuEvents>()) else {
        return;
    };

    let request = Request::Window(window.label().to_owned());
    if close.intercept(request, Instant::now(), || events.try_send(MenuCommand::CloseRequested)) {
        api.prevent_close();
    }
}

/// Asks the page before the app exits (a quit from the system) with unsaved text.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    let RunEvent::ExitRequested { code, api, .. } = event else {
        return;
    };

    let (Some(close), Some(events)) = (app.try_state::<CloseGuard>(), app.try_state::<MenuEvents>()) else {
        return;
    };

    let request = Request::Exit(code.unwrap_or(0));
    let ask = || {
        // A quit from the tray comes while the window is hidden, and the question is asked in it.
        crate::tray::reveal_if_hidden(app);

        events.try_send(MenuCommand::CloseRequested)
    };
    if close.intercept(request, Instant::now(), ask) {
        api.prevent_exit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(label: &str) -> Request {
        Request::Window(label.to_owned())
    }

    /// Whether `request` is stopped now, the page getting the question.
    fn stop(guard: &CloseGuard, request: Request) -> bool {
        guard.intercept(request, Instant::now(), || true)
    }

    #[test]
    fn a_clean_buffer_never_stops_anything() {
        let guard = CloseGuard::default();

        assert!(!guard.intercept(window("main"), Instant::now(), || panic!("nobody is asked")));
        assert!(!guard.intercept(Request::Exit(0), Instant::now(), || panic!("nobody is asked")));
        assert_eq!(guard.confirm(), None);
    }

    #[test]
    fn a_dirty_buffer_stops_a_close_and_an_exit_and_keeps_the_request() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);

        assert!(stop(&guard, window("main")));
        assert_eq!(guard.confirm(), Some(window("main")));

        guard.set_dirty(true);
        assert!(stop(&guard, Request::Exit(0)));
        assert_eq!(guard.confirm(), Some(Request::Exit(0)));
    }

    #[test]
    fn a_restart_is_never_stopped_because_tauri_would_ignore_it() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);

        assert!(
            !guard.intercept(Request::Exit(RESTART_EXIT_CODE), Instant::now(), || panic!(
                "nobody is asked"
            ))
        );
        assert!(guard.is_dirty());
    }

    #[test]
    fn without_a_page_to_answer_the_close_goes_ahead() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);

        assert!(!guard.intercept(window("main"), Instant::now(), || false));
        assert_eq!(guard.confirm(), None, "nothing is left waiting");
    }

    #[test]
    fn the_answer_may_come_while_the_page_is_still_being_asked() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);

        let stopped = guard.intercept(window("main"), Instant::now(), || {
            assert_eq!(guard.confirm(), Some(window("main")));

            true
        });

        assert!(stopped);
    }

    #[test]
    fn confirming_gives_the_request_once_and_lets_it_through() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        assert!(stop(&guard, window("main")));

        assert_eq!(guard.confirm(), Some(window("main")));

        // The window is closed again: the buffer is not dirty now, so this time nothing stops it.
        assert!(!guard.is_dirty());
        assert!(!guard.intercept(window("main"), Instant::now(), || panic!("nobody is asked")));
        assert_eq!(guard.confirm(), None);
    }

    #[test]
    fn confirming_with_nothing_waiting_changes_nothing() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);

        assert_eq!(guard.confirm(), None);

        assert!(guard.is_dirty());
    }

    #[test]
    fn a_request_that_the_page_never_acknowledges_is_given_up_so_the_app_can_be_closed() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        let asked = Instant::now();
        assert!(guard.intercept(window("main"), asked, || true));

        // Within the patience the page is asked again.
        let soon = asked + PATIENCE;
        assert!(guard.intercept(window("main"), soon, || true));

        // After it, the next close goes ahead, without asking, and nothing is left waiting.
        let late = asked + PATIENCE + Duration::from_secs(1);
        assert!(!guard.intercept(window("main"), late, || panic!("nobody is asked")));
        assert_eq!(guard.confirm(), None);
        assert!(guard.is_dirty());

        // The same for an exit.
        assert!(guard.intercept(Request::Exit(0), late, || true));
        let later = late + PATIENCE + Duration::from_secs(1);
        assert!(!guard.intercept(Request::Exit(0), later, || panic!("nobody is asked")));
    }

    #[test]
    fn an_acknowledged_question_waits_for_the_user_however_long_it_takes() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        let asked = Instant::now();
        assert!(guard.intercept(window("main"), asked, || true));
        guard.acknowledge();

        // The user reads the dialog for an hour and closes again: the page is asked, nobody is let through.
        let much_later = asked + Duration::from_secs(3600);
        assert!(guard.intercept(window("main"), much_later, || true));
        assert!(guard.intercept(Request::Exit(0), much_later, || true), "an exit too");

        // It ends with the answer.
        assert_eq!(guard.confirm(), Some(Request::Exit(0)));
    }

    #[test]
    fn asking_again_neither_extends_the_patience_nor_takes_back_an_acknowledgement() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        let asked = Instant::now();
        assert!(guard.intercept(window("main"), asked, || true));

        // Asked again within the patience: the time is still the first one's.
        assert!(guard.intercept(window("main"), asked + PATIENCE, || true));
        let late = asked + PATIENCE + Duration::from_secs(1);
        assert!(!guard.intercept(window("main"), late, || panic!("nobody is asked")));

        // An acknowledgement survives the question being asked again.
        assert!(guard.intercept(window("main"), late, || true));
        guard.acknowledge();
        assert!(guard.intercept(window("main"), late, || true));
        assert!(guard.intercept(window("main"), late + 10 * PATIENCE, || true));
    }

    #[test]
    fn an_acknowledgement_with_nothing_waiting_is_forgotten() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);

        guard.acknowledge();

        // The next question is a new one: not acknowledged, so it is given up after the patience.
        let asked = Instant::now();
        assert!(guard.intercept(window("main"), asked, || true));
        assert!(
            !guard.intercept(window("main"), asked + PATIENCE + Duration::from_secs(1), || {
                panic!("nobody is asked")
            })
        );
    }

    #[test]
    fn a_cancelled_question_is_forgotten_and_the_next_close_is_a_new_one() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        let asked = Instant::now();
        assert!(guard.intercept(window("main"), asked, || true));
        guard.acknowledge();

        // The user cancels: the page says it is still dirty.
        guard.report(true);

        let much_later = asked + 10 * PATIENCE;
        assert!(guard.intercept(window("main"), much_later, || true), "asked again");
        assert_eq!(guard.confirm(), Some(window("main")));
    }

    #[test]
    fn a_reset_forgets_an_acknowledged_question_too() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        assert!(stop(&guard, window("main")));
        guard.acknowledge();

        guard.reset();

        assert_eq!(guard.confirm(), None);
    }

    #[test]
    fn a_clean_report_keeps_the_request_for_the_answer_that_follows() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        assert!(stop(&guard, window("main")));

        // Save: the buffer is clean, then the page confirms.
        guard.report(false);

        assert_eq!(guard.confirm(), Some(window("main")));
    }

    #[test]
    fn a_reset_forgets_the_buffer_and_the_request() {
        let guard = CloseGuard::default();
        guard.set_dirty(true);
        assert!(stop(&guard, window("main")));

        guard.reset();

        assert!(!guard.is_dirty());
        assert_eq!(guard.confirm(), None);
    }
}

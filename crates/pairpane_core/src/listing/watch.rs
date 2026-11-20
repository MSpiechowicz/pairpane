//! Change notifications for a listed folder (not its subfolders), debounced so a burst of changes (a build, an
//! extraction) reports once.
//!
//! Access events are ignored: reading a folder opens it, so a pane that re-lists on every event would list its
//! own folder forever.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use notify::{Error, ErrorKind, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Changes closer together than this are reported once, after the last one.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// A folder that never pauses (a growing download, a log) is still reported this long after its first change.
pub const MAX_WAIT: Duration = Duration::from_secs(2);

/// Watches one folder while it lives; dropping it stops the watching (and its debounce thread).
pub struct FolderWatch {
    _watcher: RecommendedWatcher,
}

/// Calls `on_change` (on the watch's own thread) after the folder at `path` changes. A watcher error later on
/// is logged and ends nothing: the pane still works, only without live updates.
pub fn watch(path: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<FolderWatch> {
    let (changed, changes) = mpsc::channel::<()>();

    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| match result {
        Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
            changed.send(()).ok();
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(reason = reason(&error), "folder watch failed"),
    })?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;

    // Ends when the watcher (holding the sender) is dropped.
    thread::Builder::new()
        .name("folder-watch".to_owned())
        .spawn(move || {
            while changes.recv().is_ok() {
                // Wait until the changes pause (or for at most MAX_WAIT), then report once.
                let deadline = Instant::now() + MAX_WAIT;
                loop {
                    let wait = DEBOUNCE.min(deadline.saturating_duration_since(Instant::now()));
                    if wait.is_zero() {
                        break;
                    }
                    match changes.recv_timeout(wait) {
                        Ok(()) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_change();
            }
        })
        .map_err(Error::io)?;

    Ok(FolderWatch { _watcher: watcher })
}

/// A fixed description of a watch error for the log: the error's own text can name paths, which are never logged.
pub fn reason(error: &Error) -> &'static str {
    match &error.kind {
        ErrorKind::PathNotFound => "the folder no longer exists",
        ErrorKind::WatchNotFound => "the watch is gone",
        ErrorKind::MaxFilesWatch => "the system limit of watched folders is reached",
        ErrorKind::InvalidConfig(_) => "invalid watcher configuration",
        ErrorKind::Io(_) => "an I/O error",
        ErrorKind::Generic(_) => "a watcher error",
    }
}

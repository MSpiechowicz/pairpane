//! Pane commands: list a folder into a pane and keep it current.
//!
//! `pane_open` is async and returns at once; the watch is set up and the listing runs on a blocking thread,
//! streaming [`ListEvent`]s over the pane's channel, so a slow disk never blocks the window (the pane shows a
//! spinner meanwhile). Opening another folder in the same pane cancels the earlier listing and stops watching the
//! earlier folder; a late, slow call never replaces a newer one (each open has a generation).

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use pairpane_core::listing::{self, FolderWatch, Outcome, platform};
use pairpane_core::model::{IpcError, IpcErrorCode, ListEvent, PaneId};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use crate::error::{CommandError, CommandResult};

/// What runs for one pane: its latest open (by generation), whose listing the next open cancels, and the watch
/// on its folder.
struct Slot {
    generation: u64,
    cancel: Arc<AtomicBool>,
    watch: Option<FolderWatch>,
}

type Slots = Arc<Mutex<HashMap<PaneId, Slot>>>;

/// The panes' listings and watches.
#[derive(Default)]
pub struct Panes {
    slots: Slots,
    generations: AtomicU64,
}

/// Lists a folder into `pane` and watches it: `path` (absolute), or the home folder when it is `null`; with
/// `child`, the folder of that name inside it. Everything after the arguments are checked, including a folder
/// that cannot be read, arrives on `on_event`.
#[tauri::command]
pub async fn pane_open(
    panes: State<'_, Panes>,
    pane: PaneId,
    path: Option<String>,
    child: Option<String>,
    show_hidden: bool,
    on_event: Channel<ListEvent>,
) -> CommandResult<()> {
    let folder = resolve(path, child.as_deref())?;
    let generation = panes.generations.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));

    let earlier = lock(&panes.slots).insert(
        pane,
        Slot {
            generation,
            cancel: Arc::clone(&cancel),
            watch: None,
        },
    );
    // Dropped outside the lock: stopping a watch may wait on the OS.
    if let Some(earlier) = earlier {
        earlier.cancel.store(true, Ordering::Relaxed);
        drop(earlier);
    }

    let slots = Arc::clone(&panes.slots);
    async_runtime::spawn_blocking(move || {
        // Watched from before the listing starts, so a change made while it runs is not missed. Setting up a
        // watch touches the folder, so it happens here, never on the command's thread.
        let changes = on_event.clone();
        let watch = listing::watch(&folder, move || {
            changes.send(ListEvent::Changed).ok();
        })
        .inspect_err(|error| {
            tracing::warn!(
                reason = listing::watch_error_reason(error),
                "cannot watch the folder; the pane will not update by itself"
            );
        })
        .ok();
        set_watch(&slots, pane, generation, watch);

        let failed = list(&folder, show_hidden, &cancel, &on_event);
        if failed {
            // A folder that cannot be listed is not watched (a file changing would re-list it forever).
            set_watch(&slots, pane, generation, None);
        }
    });

    Ok(())
}

/// Attaches `watch` to the pane if `generation` is still its latest open; otherwise it is dropped.
fn set_watch(slots: &Slots, pane: PaneId, generation: u64, watch: Option<FolderWatch>) {
    let replaced = match lock(slots).get_mut(&pane) {
        Some(slot) if slot.generation == generation => std::mem::replace(&mut slot.watch, watch),
        _ => watch,
    };
    drop(replaced);
}

/// Lists `folder` onto `on_event`; `true` when it could not be listed.
fn list(folder: &std::path::Path, show_hidden: bool, cancel: &AtomicBool, on_event: &Channel<ListEvent>) -> bool {
    let started = Instant::now();
    let mut first_batch = None;
    let mut total = 0;
    // A closed channel only means the page went away; the next `pane_open` starts over.
    let send = |event: ListEvent| {
        match &event {
            ListEvent::Start { total: count, .. } => total = *count,
            ListEvent::Batch { .. } => {
                first_batch.get_or_insert_with(|| started.elapsed());
            }
            _ => {}
        }
        on_event.send(event).ok();
    };

    match listing::list(folder, show_hidden, cancel, send) {
        Ok(Outcome::Done) => {
            tracing::debug!(
                total,
                first_batch_ms = first_batch.map(|time| time.as_secs_f64() * 1000.0),
                all_ms = started.elapsed().as_secs_f64() * 1000.0,
                "folder listed"
            );
            false
        }
        Ok(Outcome::Cancelled) => false,
        Err(error) => {
            on_event
                .send(ListEvent::Error {
                    error: list_error(&error),
                })
                .ok();
            true
        }
    }
}

fn lock(slots: &Slots) -> std::sync::MutexGuard<'_, HashMap<PaneId, Slot>> {
    slots.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The folder to list: `path` or the home folder, plus `child` when given, within the platform's path rules.
fn resolve(path: Option<String>, child: Option<&str>) -> CommandResult<PathBuf> {
    let base = match path {
        Some(path) => PathBuf::from(path),
        None => std::env::home_dir()
            .ok_or_else(|| CommandError::new(IpcErrorCode::NotFound, "cannot find the home folder"))?,
    };

    let folder = match child {
        None => base,
        Some(child) if platform::is_plain_name(child) => base.join(child),
        Some(_) => {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "a folder name must be a single plain name",
            ));
        }
    };

    platform::check_folder(&folder).map_err(|reason| CommandError::new(IpcErrorCode::InvalidInput, reason))?;

    Ok(folder)
}

/// What the pane says when its folder cannot be listed.
fn list_error(error: &io::Error) -> IpcError {
    let (code, message) = match error.kind() {
        io::ErrorKind::NotFound => (IpcErrorCode::NotFound, "This folder does not exist.".to_owned()),
        io::ErrorKind::PermissionDenied => (
            IpcErrorCode::Io,
            "You do not have permission to open this folder.".to_owned(),
        ),
        io::ErrorKind::NotADirectory => (IpcErrorCode::InvalidInput, "This is not a folder.".to_owned()),
        io::ErrorKind::InvalidInput => (IpcErrorCode::InvalidInput, error.to_string()),
        _ => (IpcErrorCode::Io, format!("Cannot open this folder: {error}")),
    };

    IpcError::new(code, message)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn a_child_must_be_a_plain_name() {
        let root = if cfg!(windows) { "C:\\data" } else { "/data" };

        let joined = resolve(Some(root.to_owned()), Some("photos")).expect("plain name");
        assert_eq!(joined, Path::new(root).join("photos"));

        for bad in ["..", ".", "a/b", "a/", "/etc", ""] {
            let error = resolve(Some(root.to_owned()), Some(bad)).expect_err(bad);
            assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad}");
        }
        #[cfg(windows)]
        for bad in ["a\\b", "C:", "C:\\x"] {
            assert!(resolve(Some(root.to_owned()), Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn relative_climbing_and_network_paths_are_refused_and_no_path_means_home() {
        let root = if cfg!(windows) { "C:\\data" } else { "/data" };
        let climbing = format!("{root}{}..{}x", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
        for bad in ["relative/dir", climbing.as_str()] {
            let error = resolve(Some(bad.to_owned()), None).expect_err(bad);
            assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad}");
        }
        #[cfg(windows)]
        for bad in [r"\\attacker\x", "//attacker/x", r"\\?\UNC\a\b", r"\\.\pipe\x"] {
            assert!(resolve(Some(bad.to_owned()), None).is_err(), "{bad}");
        }

        if let Some(home) = std::env::home_dir() {
            assert_eq!(resolve(None, None).expect("home"), home);
        }
    }

    #[test]
    fn listing_errors_tell_the_user_what_happened() {
        let not_found = list_error(&io::Error::from(io::ErrorKind::NotFound));
        let denied = list_error(&io::Error::from(io::ErrorKind::PermissionDenied));

        assert_eq!(not_found.code, IpcErrorCode::NotFound);
        assert_eq!(denied.code, IpcErrorCode::Io);
        assert!(denied.message.contains("permission"));
    }
}

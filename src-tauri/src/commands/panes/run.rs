//! The blocking part of `pane_open`: the checks, the watch and the listing of one open.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use pairpane_core::listing::{self, Outcome};
use pairpane_core::model::{IpcError, ListEvent, PaneId};
use pairpane_core::vfs::{LocalFolder, VfsFolder};
use tauri::ipc::Channel;

use super::{SlotKey, Slots, Watch, claim_sizing, list_error, lock};
use crate::commands::limit::Permit;

/// One `pane_open`, ready to run on a blocking thread.
pub(super) struct Open {
    pub(super) slots: Slots,
    pub(super) pane: PaneId,
    pub(super) generation: u64,
    pub(super) folder: PathBuf,
    pub(super) show_hidden: bool,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) on_event: Channel<ListEvent>,
    /// The pane's watch was kept from the same folder's earlier open.
    pub(super) watched: bool,
}

impl Open {
    /// Checks the path, watches the folder and lists it, holding `permit` until done.
    pub(super) fn run(self, permit: Permit) {
        let _permit = permit;
        let Self {
            slots,
            pane,
            generation,
            folder,
            show_hidden,
            cancel,
            on_event,
            watched,
        } = self;

        // First of all: a watch or a read would make Windows connect to a network host if a link on the path leads to
        // one. Only the path is read, here and not on the command's thread: a hung folder holds a counted thread.
        // From here on the folder is read at the path without links that this returned, so the system resolves
        // none of its own; `folder`, as the user wrote it, is only what the pane shows and compares.
        let local = match local_folder(&folder) {
            Ok(local) => local,
            Err(error) => {
                on_event.send(ListEvent::Error { error }).ok();
                set_watch(&slots, pane, generation, None);
                return;
            }
        };

        // Watched from before the listing starts, so a change made while it runs is not missed. Setting up a
        // watch touches the folder, so it happens here, never on the command's thread.
        if !watched {
            set_watch(&slots, pane, generation, start_watch(&local, &on_event));
        }

        let started = || claim_sizing(&slots, pane, generation, &SlotKey::Folder(folder));
        let failed = list(&local, show_hidden, &cancel, &on_event, started);
        if failed {
            // A folder that cannot be listed is not watched (a file changing would re-list it forever).
            set_watch(&slots, pane, generation, None);
        }
    }
}

/// The folder to list for `folder`: read where it leads once every link on it is resolved and checked (see
/// [`LocalFolder::resolve`]), or why it must not be opened (a link on its path leads to a network or device path, or
/// loops, or cannot be read). On other systems than Windows it is read at `folder` itself.
pub(super) fn local_folder(folder: &Path) -> Result<LocalFolder, IpcError> {
    LocalFolder::resolve(folder.to_owned()).map_err(|error| list_error(&error))
}

/// Watches `folder`, sending its changes to `on_event` (or to the channel of a later reload); `None` when the
/// folder cannot be watched, or is not a place that is.
fn start_watch(folder: &dyn VfsFolder, on_event: &Channel<ListEvent>) -> Option<Watch> {
    let sink = Arc::new(Mutex::new(on_event.clone()));
    let changes = Arc::clone(&sink);
    let watch = folder
        .watch(Box::new(move || {
            changes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(ListEvent::Changed)
                .ok();
        }))?
        .inspect_err(|error| {
            tracing::warn!(
                reason = listing::watch_error_reason(error),
                "cannot watch the folder; the pane will not update by itself"
            );
        })
        .ok()?;

    Some(Watch { _watch: watch, sink })
}

/// Attaches `watch` to the pane if `generation` is still its latest open; otherwise it is dropped.
fn set_watch(slots: &Slots, pane: PaneId, generation: u64, watch: Option<Watch>) {
    let replaced = match lock(slots).get_mut(&pane) {
        Some(slot) if slot.generation == generation => std::mem::replace(&mut slot.watch, watch),
        _ => watch,
    };
    drop(replaced);
}

/// Lists `folder` onto `on_event`, calling `on_start` as the first pass ends (just before `start` is sent); `true`
/// when it could not be listed.
fn list(
    folder: &dyn VfsFolder,
    show_hidden: bool,
    cancel: &AtomicBool,
    on_event: &Channel<ListEvent>,
    on_start: impl FnOnce(),
) -> bool {
    let started = Instant::now();
    let mut first_batch = None;
    let mut total = 0;
    let mut on_start = Some(on_start);
    // A closed channel only means the page went away; the next `pane_open` starts over.
    let mut send = |event: ListEvent| {
        match &event {
            ListEvent::Start { total: count, .. } => {
                total = *count;
                if let Some(on_start) = on_start.take() {
                    on_start();
                }
            }
            ListEvent::Batch { .. } => {
                first_batch.get_or_insert_with(|| started.elapsed());
            }
            _ => {}
        }
        on_event.send(event).ok();
    };

    match folder.list(show_hidden, cancel, &mut send) {
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

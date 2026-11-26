//! Pane commands: list a folder into a pane and keep it current.
//!
//! `pane_open` is async and returns at once; the watch is set up and the listing runs on a blocking thread,
//! streaming [`ListEvent`]s over the pane's channel, so a slow disk never blocks the window (the pane shows a
//! spinner meanwhile). Opening another folder in the same pane cancels the earlier listing and stops watching the
//! earlier folder; a late, slow call never replaces a newer one (each open has a generation).
//!
//! A listing cancelled on a hung mount stays stuck, so a pane may have only [`LISTING_THREADS`] outstanding; beyond
//! that `pane_open` is refused.
//!
//! The pane's size jobs (`sizes.rs`) belong to the latest folder that listed: they move on only once another
//! folder's listing has started, so an open that fails leaves the pane, and its jobs, as they were.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pairpane_core::listing::{FolderWatch, platform};
use pairpane_core::model::{FindId, IpcError, IpcErrorCode, ListEvent, PaneId};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use self::run::Open;
use super::limit::{Permit, ThreadCap};
use super::sizes::Sizing;
use crate::error::{CommandError, CommandResult};

/// What runs for one pane: its latest open (by generation), whose listing the next open cancels, the watch
/// on its folder, and its size jobs.
struct Slot {
    generation: u64,
    key: SlotKey,
    cancel: Arc<AtomicBool>,
    /// Outlives opens: it follows the folder that last listed, not the one last asked for.
    sizing: Sizing,
    watch: Option<Watch>,
}

/// What a pane's slot shows, which decides what an open of the pane keeps from the earlier one: the same key keeps the
/// folder's watch, and only a key with a folder to measure keeps the pane's size jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SlotKey {
    /// A folder on disk, as the pane shows it.
    Folder(PathBuf),
    /// A folder inside the archive file at this path: the folder that holds the archive is not watched.
    Archive(PathBuf),
    /// The results of a search: no folder, so nothing is watched and nothing can be measured.
    Results(FindId),
}

impl SlotKey {
    /// The folder whose subfolders the pane may measure while it shows this; `None` when it has none.
    pub(super) fn measured(&self) -> Option<&Path> {
        match self {
            Self::Folder(path) | Self::Archive(path) => Some(path),
            Self::Results(_) => None,
        }
    }
}

/// A folder watch and the channel its changes go to: the latest open's, so a reload of the same folder keeps the
/// watch (a new one would report recent changes again on macOS, and churn the OS on every reload).
struct Watch {
    _watch: FolderWatch,
    sink: Arc<Mutex<Channel<ListEvent>>>,
}

type Slots = Arc<Mutex<HashMap<PaneId, Slot>>>;

/// What an open of a pane holds from the moment it is the latest: its place of the listing cap, its generation, the flag
/// the next open raises and whether the pane's watch was kept.
pub(super) struct Claim {
    pub(super) permit: Permit,
    pub(super) generation: u64,
    pub(super) cancel: Arc<AtomicBool>,
    watched: bool,
}

/// The panes' listings and watches.
#[derive(Default)]
pub struct Panes {
    slots: Slots,
    generations: AtomicU64,
    /// The listing threads each pane has outstanding, which outlive the slots they ran for.
    left_listings: ThreadCap,
    right_listings: ThreadCap,
}

/// Listing threads a pane may have at once. A superseded listing stops within a moment (it checks the cancel flag
/// for every entry), so quick navigation stays far below this; only folders that do not respond pile up.
const LISTING_THREADS: usize = 4;

impl Panes {
    /// Makes a new open of `key` the latest of `pane`, after a place of the pane's listing cap is taken (refused as busy
    /// before anything changes, so the pane keeps showing its folder). The slot of the open it replaces is dropped here,
    /// outside the lock, and its listing is cancelled.
    pub(super) fn claim(&self, pane: PaneId, key: SlotKey, on_event: &Channel<ListEvent>) -> CommandResult<Claim> {
        let permit = self
            .listings(pane)
            .acquire(LISTING_THREADS)
            .ok_or_else(|| CommandError::new(IpcErrorCode::Busy, "a folder is not responding; try again shortly"))?;
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));

        let (earlier, watched) = replace_slot(&mut lock(&self.slots), pane, generation, key, &cancel, on_event);
        // Dropped outside the lock: stopping a watch may wait on the OS.
        drop(earlier);

        Ok(Claim {
            permit,
            generation,
            cancel,
            watched,
        })
    }

    /// What to call once the listing of `key`, opened as `pane`'s `generation`, starts (see [`claim_sizing`]).
    pub(super) fn listed(&self, pane: PaneId, generation: u64, key: SlotKey) -> impl FnOnce() + Send + use<> {
        let slots = Arc::clone(&self.slots);

        move || claim_sizing(&slots, pane, generation, &key)
    }

    fn listings(&self, pane: PaneId) -> &ThreadCap {
        match pane {
            PaneId::Left => &self.left_listings,
            PaneId::Right => &self.right_listings,
        }
    }

    /// Runs `job` on the size state of `pane`; `None` when the pane was never opened.
    pub(super) fn with_sizing<R>(&self, pane: PaneId, job: impl FnOnce(&mut Sizing) -> R) -> Option<R> {
        lock(&self.slots).get_mut(&pane).map(|slot| job(&mut slot.sizing))
    }
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
    let Claim {
        permit,
        generation,
        cancel,
        watched,
    } = panes.claim(pane, SlotKey::Folder(folder.clone()), &on_event)?;

    let open = Open {
        slots: Arc::clone(&panes.slots),
        pane,
        generation,
        folder,
        show_hidden,
        cancel,
        on_event,
        watched,
    };
    // The permit is held until the listing ends, however it ends.
    async_runtime::spawn_blocking(move || open.run(permit));

    Ok(())
}

/// Makes `key` the pane's latest open and returns the slot it replaced (to drop outside the lock; its listing
/// is cancelled) and whether the pane's watch was kept. The same folder again (a reload after a change, hidden
/// files toggled) keeps its watch, retargeted to `on_event`. The size jobs always stay: they move to the folder of `key` only
/// when it lists (see [`claim_sizing`]).
fn replace_slot(
    slots: &mut HashMap<PaneId, Slot>,
    pane: PaneId,
    generation: u64,
    key: SlotKey,
    cancel: &Arc<AtomicBool>,
    on_event: &Channel<ListEvent>,
) -> (Option<Slot>, bool) {
    let mut earlier = slots.remove(&pane);

    let mut kept = None;
    let mut sizing = Sizing::default();
    if let Some(slot) = earlier.as_mut() {
        sizing = std::mem::take(&mut slot.sizing);
        if slot.key == key {
            kept = slot.watch.take();
        }
    }
    if let Some(kept) = &kept {
        *kept.sink.lock().unwrap_or_else(PoisonError::into_inner) = on_event.clone();
    }

    let watched = kept.is_some();
    slots.insert(
        pane,
        Slot {
            generation,
            key,
            cancel: Arc::clone(cancel),
            sizing,
            watch: kept,
        },
    );

    if let Some(earlier) = &earlier {
        earlier.cancel.store(true, Ordering::Relaxed);
    }

    (earlier, watched)
}

/// Moves the pane's size jobs to the folder of `key`, whose listing has started, if `generation` is still the pane's
/// latest open: a late listing never takes them from a newer open. A key with no folder stops them.
fn claim_sizing(slots: &Slots, pane: PaneId, generation: u64, key: &SlotKey) {
    if let Some(slot) = lock(slots).get_mut(&pane)
        && slot.generation == generation
    {
        slot.sizing.move_to(key.measured());
    }
}

fn lock(slots: &Slots) -> std::sync::MutexGuard<'_, HashMap<PaneId, Slot>> {
    slots.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The folder to list: `path` or the home folder, plus `child` when given, within the platform's path rules.
pub(super) fn resolve(path: Option<String>, child: Option<&str>) -> CommandResult<PathBuf> {
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
    folder_error(error, "open this folder", "Cannot open this folder")
}

/// A folder error for the user: `denied` finishes "You do not have permission to …", `failed` leads any other error.
pub(super) fn folder_error(error: &io::Error, denied: &str, failed: &str) -> IpcError {
    let (code, message) = match error.kind() {
        io::ErrorKind::NotFound => (IpcErrorCode::NotFound, "This folder does not exist.".to_owned()),
        io::ErrorKind::PermissionDenied => (IpcErrorCode::Io, format!("You do not have permission to {denied}.")),
        io::ErrorKind::NotADirectory => (IpcErrorCode::InvalidInput, "This is not a folder.".to_owned()),
        io::ErrorKind::InvalidInput => (IpcErrorCode::InvalidInput, error.to_string()),
        _ => (IpcErrorCode::Io, format!("{failed}: {error}")),
    };

    IpcError::new(code, message)
}

pub mod archive;
mod run;

#[cfg(test)]
mod path_tests;
#[cfg(test)]
mod tests;

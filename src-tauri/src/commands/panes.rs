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
use pairpane_core::model::{IpcError, IpcErrorCode, ListEvent, PaneId};
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
    folder: PathBuf,
    cancel: Arc<AtomicBool>,
    /// Outlives opens: it follows the folder that last listed, not the one last asked for.
    sizing: Sizing,
    watch: Option<Watch>,
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
struct Claim {
    permit: Permit,
    generation: u64,
    cancel: Arc<AtomicBool>,
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
    /// Makes a new open of `folder` the latest of `pane`, after a place of the pane's listing cap is taken (refused as busy
    /// before anything changes, so the pane keeps showing its folder). The slot of the open it replaces is dropped here,
    /// outside the lock, and its listing is cancelled.
    fn claim(&self, pane: PaneId, folder: &Path, on_event: &Channel<ListEvent>) -> CommandResult<Claim> {
        let permit = self
            .listings(pane)
            .acquire(LISTING_THREADS)
            .ok_or_else(|| CommandError::new(IpcErrorCode::Busy, "a folder is not responding; try again shortly"))?;
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));

        let (earlier, watched) = replace_slot(&mut lock(&self.slots), pane, generation, folder, &cancel, on_event);
        // Dropped outside the lock: stopping a watch may wait on the OS.
        drop(earlier);

        Ok(Claim {
            permit,
            generation,
            cancel,
            watched,
        })
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
    } = panes.claim(pane, &folder, &on_event)?;

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

/// Makes `folder` the pane's latest open and returns the slot it replaced (to drop outside the lock; its listing
/// is cancelled) and whether the pane's watch was kept. The same folder again (a reload after a change, hidden
/// files toggled) keeps its watch, retargeted to `on_event`. The size jobs always stay: they move to `folder` only
/// when it lists (see [`claim_sizing`]).
fn replace_slot(
    slots: &mut HashMap<PaneId, Slot>,
    pane: PaneId,
    generation: u64,
    folder: &Path,
    cancel: &Arc<AtomicBool>,
    on_event: &Channel<ListEvent>,
) -> (Option<Slot>, bool) {
    let mut earlier = slots.remove(&pane);

    let mut kept = None;
    let mut sizing = Sizing::default();
    if let Some(slot) = earlier.as_mut() {
        sizing = std::mem::take(&mut slot.sizing);
        if slot.folder == folder {
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
            folder: folder.to_owned(),
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

/// Moves the pane's size jobs to `folder`, whose listing has started, if `generation` is still the pane's latest
/// open: a late listing never takes them from a newer open.
fn claim_sizing(slots: &Slots, pane: PaneId, generation: u64, folder: &Path) {
    if let Some(slot) = lock(slots).get_mut(&pane)
        && slot.generation == generation
    {
        slot.sizing.move_to(folder);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static str {
        if cfg!(windows) { "C:\\data" } else { "/data" }
    }

    /// Opens `folder` in the left pane of `slots` and returns what it replaced.
    fn open(slots: &Slots, generation: u64, folder: &Path) -> Option<Slot> {
        let (earlier, _) = replace_slot(
            &mut lock(slots),
            PaneId::Left,
            generation,
            folder,
            &Arc::new(AtomicBool::new(false)),
            &Channel::new(|_| Ok(())),
        );

        earlier
    }

    /// The listing of `folder` opened as `generation` has started.
    fn start(slots: &Slots, generation: u64, folder: &Path) {
        claim_sizing(slots, PaneId::Left, generation, folder);
    }

    /// Starts a size job for `name` inside `path` in the left pane.
    fn size_job(slots: &Slots, path: &Path, name: &str) -> CommandResult<Arc<AtomicBool>> {
        let slot = lock(slots)
            .get_mut(&PaneId::Left)
            .map(|slot| slot.sizing.begin(path.to_string_lossy().into_owned(), name));

        slot.expect("open pane").map(|job| job.stop)
    }

    /// A pane whose folder (generation 1) has listed and has a size job running in it.
    fn measuring() -> (Slots, PathBuf, Arc<AtomicBool>) {
        let slots = Slots::default();
        let folder = PathBuf::from(root());
        open(&slots, 1, &folder);
        start(&slots, 1, &folder);
        let stop = size_job(&slots, &folder, "photos").expect("job");

        (slots, folder, stop)
    }

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

    #[test]
    fn reopening_the_same_folder_keeps_its_size_job_and_listing_another_stops_it() {
        let (slots, folder, stop) = measuring();
        let folder = folder.as_path();

        let earlier = open(&slots, 2, folder).expect("replaced");
        start(&slots, 2, folder);
        assert!(earlier.cancel.load(Ordering::Relaxed), "the earlier listing stops");
        assert!(!stop.load(Ordering::Relaxed), "a reload leaves the size job running");

        let photos = folder.join("photos");
        open(&slots, 3, &photos);
        start(&slots, 3, &photos);
        assert!(stop.load(Ordering::Relaxed), "a listed folder stops the job");
        assert!(size_job(&slots, folder, "photos").is_err());
        assert!(size_job(&slots, &photos, "2024").is_ok());
    }

    #[test]
    fn a_failed_open_leaves_the_size_job_and_target_of_the_shown_folder() {
        let (slots, folder, stop) = measuring();
        let folder = folder.as_path();

        // The open of `locked` fails: its listing never starts.
        let locked = folder.join("locked");
        let earlier = open(&slots, 2, &locked).expect("replaced");

        assert!(earlier.cancel.load(Ordering::Relaxed), "the earlier listing stops");
        assert!(!stop.load(Ordering::Relaxed), "the shown folder's job keeps running");
        assert!(
            size_job(&slots, folder, "other").is_ok(),
            "its folders can still be measured"
        );
        assert!(
            size_job(&slots, &locked, "x").is_err(),
            "the folder that failed is not the target"
        );
    }

    #[test]
    fn a_late_listing_never_takes_the_size_jobs_from_a_newer_open() {
        let (slots, folder, stop) = measuring();
        let folder = folder.as_path();

        let photos = folder.join("photos");
        let other = folder.join("other");
        open(&slots, 2, &photos);
        open(&slots, 3, &other);
        start(&slots, 2, &photos);

        assert!(
            !stop.load(Ordering::Relaxed),
            "the superseded open does not move the jobs"
        );

        start(&slots, 3, &other);
        assert!(stop.load(Ordering::Relaxed), "the latest open does");
        assert!(size_job(&slots, folder, "photos").is_err());
        assert!(size_job(&slots, &other, "x").is_ok());
    }

    #[test]
    fn each_pane_has_its_own_listing_cap_and_a_finished_listing_frees_a_place() {
        let panes = Panes::default();

        let mut running: Vec<_> = (0..LISTING_THREADS)
            .map(|_| panes.listings(PaneId::Left).acquire(LISTING_THREADS).expect("place"))
            .collect();
        assert!(
            panes.listings(PaneId::Left).acquire(LISTING_THREADS).is_none(),
            "the left pane is full"
        );
        assert!(
            panes.listings(PaneId::Right).acquire(LISTING_THREADS).is_some(),
            "the right pane is not"
        );

        running.pop();
        assert!(panes.listings(PaneId::Left).acquire(LISTING_THREADS).is_some());
    }
}

pub mod archive;
mod run;

#[cfg(test)]
mod path_tests;

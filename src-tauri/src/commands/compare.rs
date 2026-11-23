//! Compare and synchronize folders: `compare_start` scans two folders on a blocking thread of its own and streams its
//! [`CompareEvent`]s over the channel it is given; `compare_cancel` stops it, `compare_forget` drops it, and `ops_sync`
//! (`ops/sync.rs`) queues the one job that copies the items the page chose from the scan.
//!
//! The comparer keeps nothing of what a scan found (the page holds the rows): only a flag to stop it. At most
//! [`SCAN_THREADS`] scans run at once (a third is refused as busy; a stopped scan keeps its place until its walk returns, as a
//! walk on a hung mount may not) and at most [`STORED`] are kept: starting one beyond that drops the oldest that has ended,
//! never one that still runs. A scan stops when the page's channel is gone, and all stop as the app exits.
//!
//! A side may be a folder inside an archive (never both: an archive is only copied from, and two would have nothing to
//! write); its index is read on the scan thread, with the scan's own stop flag. Content is never compared with an archive.
//!
//! Errors never carry a path: a side that cannot be compared is reported with the core's fixed text.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::compare::{self, Side};
use pairpane_core::model::{
    CompareEvent, CompareId, CompareOptions, CompareRoot, IpcError, IpcErrorCode, JobId, SyncItem, SyncRemoval,
};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use super::archives::{Archives, Target, archive_error, check_inner};
use super::limit::{Permit, ThreadCap};
use super::ops::Ops;
use super::panes::{folder_error, resolve};
use crate::error::{CommandError, CommandResult};

#[cfg(test)]
pub(super) mod tests;

/// Scans that may run at once.
const SCAN_THREADS: usize = 2;

/// Scans kept, running or ended.
const STORED: usize = 4;

/// One side of a request, checked for syntax: a folder, or a folder inside an archive file.
#[derive(Debug, Clone)]
pub(super) enum Root {
    Folder(std::path::PathBuf),
    Archive { target: Target, dir: String },
}

impl Root {
    /// Checks the side the page sent. Touches no disk.
    pub(super) fn parse(root: CompareRoot) -> CommandResult<Self> {
        match root {
            CompareRoot::Folder { path } => Ok(Self::Folder(resolve(Some(path), None)?)),
            CompareRoot::Archive { archive, dir } => {
                check_inner(&dir)?;

                Ok(Self::Archive {
                    target: Target::parse(archive)?,
                    dir,
                })
            }
        }
    }

    pub(super) fn is_archive(&self) -> bool {
        matches!(self, Self::Archive { .. })
    }
}

/// One scan: its stop flag, and whether its walk has returned.
#[derive(Debug, Default)]
struct Scan {
    cancel: AtomicBool,
    finished: AtomicBool,
}

impl Scan {
    fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Runs the comparison of `left` and `right`, sending every event to `on_event` and ending with one `done` or `error`.
    /// Stops when the channel is gone.
    fn run(
        &self,
        archives: &Archives,
        (left, right): (Root, Root),
        options: CompareOptions,
        on_event: &Channel<CompareEvent>,
    ) {
        let mut emit = |event: CompareEvent| {
            if on_event.send(event).is_err() {
                self.stop();
            }
        };

        // A panic in the scan must still end it for the page.
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let (left, right) = (self.side(archives, left)?, self.side(archives, right)?);

            compare::run(&left, &right, options, &self.cancel, &mut emit).map_err(|error| compare_error(&error))
        }));

        match outcome {
            Ok(Ok(())) => {}
            // Stopped while an archive was read: the scan ends as cancelled, with nothing found.
            Ok(Err(_)) if self.cancel.load(Ordering::Relaxed) => emit(CompareEvent::Done {
                rows: 0,
                skipped: 0,
                truncated: false,
                cancelled: true,
            }),
            Ok(Err(error)) => emit(CompareEvent::Error { error }),
            Err(_) => emit(CompareEvent::Error {
                error: IpcError::new(IpcErrorCode::Internal, "The comparison failed."),
            }),
        }

        self.finished.store(true, Ordering::Release);
    }

    /// The side the core compares: a folder as it is, an archive read into its index now (kept in the cache of the archives,
    /// so a pane that shows it does not read it again). The file is closed again before the walk starts.
    fn side(&self, archives: &Archives, root: Root) -> Result<Side, IpcError> {
        match root {
            Root::Folder(path) => Ok(Side::Folder(path)),
            Root::Archive { target, dir } => {
                let opened = archives.open(&target).map_err(|error| archive_error(&error))?;
                let index = archives
                    .index(&target, &opened, &self.cancel, None)
                    .map_err(|error| archives.report(&target, &error))?;

                Ok(Side::Archive { index, dir })
            }
        }
    }
}

/// The scans of the app.
#[derive(Default)]
pub struct Comparer {
    scans: Mutex<HashMap<CompareId, Arc<Scan>>>,
    ids: AtomicU64,
    running: ThreadCap,
}

impl Comparer {
    /// Checks the request and starts the scan, returning its id; its events follow on `on_event`.
    fn start(
        &self,
        archives: &Archives,
        (left, right): (CompareRoot, CompareRoot),
        options: CompareOptions,
        on_event: Channel<CompareEvent>,
    ) -> CommandResult<CompareId> {
        let (left, right) = (Root::parse(left)?, Root::parse(right)?);
        let archived = usize::from(left.is_archive()) + usize::from(right.is_archive());

        if archived == 2 {
            return Err(refused("Two archives cannot be compared."));
        }
        if archived == 1 && options.content {
            return Err(refused("Content cannot be compared with an archive."));
        }

        let permit = self.running.acquire(SCAN_THREADS).ok_or_else(busy)?;
        let read = (archived == 1).then(|| archives.begin()).transpose()?;

        let id = CompareId(self.ids.fetch_add(1, Ordering::Relaxed) + 1);
        let scan = Arc::new(Scan::default());
        self.store(id, Arc::clone(&scan))?;

        spawn((permit, read), scan, archives.clone(), (left, right), options, on_event);

        Ok(id)
    }

    /// Keeps `scan`, dropping the oldest scan that has ended when [`STORED`] are kept already; refused when all of them run.
    fn store(&self, id: CompareId, scan: Arc<Scan>) -> CommandResult<()> {
        let mut scans = self.lock();

        let mut evicted = None;
        if scans.len() >= STORED {
            let oldest = scans
                .iter()
                .filter(|(_, kept)| kept.finished())
                .map(|(id, _)| *id)
                .min()
                .ok_or_else(busy)?;
            evicted = scans.remove(&oldest);
        }
        scans.insert(id, scan);

        drop(scans);
        drop(evicted);

        Ok(())
    }

    fn cancel(&self, id: CompareId) -> CommandResult<()> {
        self.get(id)?.stop();

        Ok(())
    }

    /// Stops the scan `id` and forgets it.
    fn forget(&self, id: CompareId) -> CommandResult<()> {
        let scan = self.lock().remove(&id).ok_or_else(unknown)?;

        scan.stop();

        Ok(())
    }

    fn get(&self, id: CompareId) -> CommandResult<Arc<Scan>> {
        self.lock().get(&id).cloned().ok_or_else(unknown)
    }

    /// Stops every scan, as the app exits.
    pub fn shut_down(&self) {
        for scan in self.lock().values() {
            scan.stop();
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<CompareId, Arc<Scan>>> {
        self.scans.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs the scan on a blocking thread that holds its places until its walk returns, however it ends.
fn spawn(
    permits: (Permit, Option<Permit>),
    scan: Arc<Scan>,
    archives: Archives,
    roots: (Root, Root),
    options: CompareOptions,
    on_event: Channel<CompareEvent>,
) {
    async_runtime::spawn_blocking(move || {
        let _permits = permits;

        scan.run(&archives, roots, options, &on_event);
    });
}

fn busy() -> CommandError {
    CommandError::new(
        IpcErrorCode::Busy,
        "two comparisons are running; stop one or wait for it",
    )
}

fn unknown() -> CommandError {
    CommandError::new(IpcErrorCode::NotFound, "this comparison is no longer kept")
}

pub(super) fn refused(text: &str) -> CommandError {
    CommandError::new(IpcErrorCode::InvalidInput, text)
}

/// What the page says when the folders cannot be compared.
fn compare_error(error: &std::io::Error) -> IpcError {
    folder_error(error, "read these folders", "Cannot compare these folders")
}

/// Starts the comparison of `left` and `right` and answers with its id at once; the events follow on `on_event`: `rows`
/// batches, `progress`, then one `done`, or one `error` when the sides cannot be compared (the same folder, one inside the
/// other, a folder that cannot be read). A root that cannot be used, two archives, or content with an archive rejects the
/// command with `invalidInput`; a third comparison at once, `busy`.
#[tauri::command]
pub async fn compare_start(
    comparer: State<'_, Comparer>,
    archives: State<'_, Archives>,
    left: CompareRoot,
    right: CompareRoot,
    options: CompareOptions,
    on_event: Channel<CompareEvent>,
) -> CommandResult<CompareId> {
    comparer.start(&archives, (left, right), options, on_event)
}

/// Stops the comparison `id` (its `done` says `cancelled`). An unknown id is `notFound`.
#[tauri::command]
pub async fn compare_cancel(comparer: State<'_, Comparer>, id: CompareId) -> CommandResult<()> {
    comparer.cancel(id)
}

/// Stops the comparison `id` and drops it. An unknown id is `notFound`.
#[tauri::command]
pub async fn compare_forget(comparer: State<'_, Comparer>, id: CompareId) -> CommandResult<()> {
    comparer.forget(id)
}

/// Queues the one job that copies `items` between `left` and `right`, each the way its direction says, then removes
/// `removals` (a mirror: to the Trash, or for good when `permanent`), and returns its id at once. An item that is not as the
/// scan saw it by then is skipped and counted. An archive may only be copied from, and only one side may be one.
#[tauri::command]
pub async fn ops_sync(
    ops: State<'_, Ops>,
    archives: State<'_, Archives>,
    left: CompareRoot,
    right: CompareRoot,
    items: Vec<SyncItem>,
    removals: Vec<SyncRemoval>,
    permanent: bool,
) -> CommandResult<JobId> {
    ops.sync(&archives, (left, right), items, (removals, permanent)).await
}

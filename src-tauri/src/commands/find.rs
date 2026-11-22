//! Find files: `find_start` runs a search of a folder tree on a blocking thread of its own and streams its [`FindEvent`]s
//! over the channel it is given; `find_cancel` stops it, `find_forget` drops it, and `find_open` (`find/list.rs`) lists
//! its results into a pane.
//!
//! The finder keeps what a search found (until it is forgotten or evicted), so the page can list the results of a search
//! it is done with without searching again. At most [`SEARCH_THREADS`] searches run at once (a third is refused as busy;
//! a stopped search keeps its place until its walk returns, as a walk on a hung mount may not) and at most [`STORED`]
//! are kept: starting one beyond that drops the oldest that has ended, never one that still runs. A search stops when
//! the page's channel is gone, and all stop as the app exits.
//!
//! Errors never carry a path: a pattern that cannot run is refused with the core's fixed text.

use std::collections::HashMap;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::find::{self, Compiled};
use pairpane_core::model::{FindEvent, FindHit, FindId, FindQuery, IpcError, IpcErrorCode};
use pairpane_core::view::is_refusal;
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use super::limit::{Permit, ThreadCap};
use super::panes::{folder_error, resolve};
use crate::error::{CommandError, CommandResult};

pub mod list;

#[cfg(test)]
mod tests;

/// Searches that may run at once.
const SEARCH_THREADS: usize = 2;

/// Searches kept, running or ended.
const STORED: usize = 4;

/// One search: the folder it looks in, its cancel flag, what it found, and whether its walk has returned.
#[derive(Debug)]
pub(super) struct Search {
    root: PathBuf,
    cancel: AtomicBool,
    finished: AtomicBool,
    hits: Mutex<Vec<FindHit>>,
}

impl Search {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            hits: Mutex::new(Vec::new()),
        }
    }

    fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// The results of the filesystem found so far; a hit inside an archive has no entry of a folder to list.
    fn snapshot(&self) -> Vec<FindHit> {
        let hits = self.hits.lock().unwrap_or_else(PoisonError::into_inner);

        hits.iter().filter(|hit| hit.inner.is_none()).cloned().collect()
    }

    fn record(&self, event: &FindEvent) {
        if let FindEvent::Found { hits } | FindEvent::Group { hits, .. } = event {
            let mut found = self.hits.lock().unwrap_or_else(PoisonError::into_inner);
            found.extend(hits.iter().cloned());
        }
    }

    /// Runs `compiled` in the root, sending every event to `on_event` (after it is recorded, so a listing made once the
    /// page has seen a result holds it) and ending with one `done` or `error`. Stops when the channel is gone.
    fn run(&self, compiled: &Compiled, on_event: &Channel<FindEvent>) {
        let mut emit = |event: FindEvent| {
            self.record(&event);
            if on_event.send(event).is_err() {
                self.cancel.store(true, Ordering::Relaxed);
            }
        };

        // A panic in the search must still end it for the page.
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            find::run(&self.root, compiled, &self.cancel, &mut emit)
        }));

        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => emit(FindEvent::Error {
                error: search_error(&error),
            }),
            Err(_) => emit(FindEvent::Error {
                error: IpcError::new(IpcErrorCode::Internal, "The search failed."),
            }),
        }

        self.finished.store(true, Ordering::Release);
    }
}

/// The searches of the app.
#[derive(Default)]
pub struct Finder {
    searches: Mutex<HashMap<FindId, Arc<Search>>>,
    ids: AtomicU64,
    running: ThreadCap,
}

impl Finder {
    /// Starts the search of `query` and returns its id; its events follow on `on_event`.
    fn start(&self, query: &FindQuery, on_event: Channel<FindEvent>) -> CommandResult<FindId> {
        let root = resolve(Some(query.root.clone()), None)?;
        let compiled = find::compile(query).map_err(invalid)?;
        let permit = self.running.acquire(SEARCH_THREADS).ok_or_else(busy)?;

        let id = FindId(self.ids.fetch_add(1, Ordering::Relaxed) + 1);
        let search = Arc::new(Search::new(root));
        self.store(id, Arc::clone(&search))?;

        spawn(permit, search, compiled, on_event);

        Ok(id)
    }

    /// Keeps `search`, dropping the oldest search that has ended when [`STORED`] are kept already; refused when all of
    /// them run.
    fn store(&self, id: FindId, search: Arc<Search>) -> CommandResult<()> {
        let mut searches = self.lock();

        let mut evicted = None;
        if searches.len() >= STORED {
            let oldest = searches
                .iter()
                .filter(|(_, kept)| kept.finished())
                .map(|(id, _)| *id)
                .min()
                .ok_or_else(busy)?;
            evicted = searches.remove(&oldest);
        }
        searches.insert(id, search);

        drop(searches);
        drop(evicted);

        Ok(())
    }

    /// The search `id`, ended or running.
    fn get(&self, id: FindId) -> CommandResult<Arc<Search>> {
        self.lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| CommandError::new(IpcErrorCode::NotFound, "this search is no longer kept"))
    }

    fn cancel(&self, id: FindId) -> CommandResult<()> {
        self.get(id)?.cancel.store(true, Ordering::Relaxed);

        Ok(())
    }

    /// Stops the search `id` and forgets its results.
    fn forget(&self, id: FindId) -> CommandResult<()> {
        let search = self
            .lock()
            .remove(&id)
            .ok_or_else(|| CommandError::new(IpcErrorCode::NotFound, "this search is no longer kept"))?;

        search.cancel.store(true, Ordering::Relaxed);

        Ok(())
    }

    /// Stops every search, as the app exits.
    pub fn shut_down(&self) {
        for search in self.lock().values() {
            search.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<FindId, Arc<Search>>> {
        self.searches.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs the search on a blocking thread that holds `permit` until its walk returns, however it ends.
fn spawn(permit: Permit, search: Arc<Search>, compiled: Compiled, on_event: Channel<FindEvent>) {
    async_runtime::spawn_blocking(move || {
        let _permit = permit;

        search.run(&compiled, &on_event);
    });
}

fn busy() -> CommandError {
    CommandError::new(IpcErrorCode::Busy, "two searches are running; stop one or wait for it")
}

/// A search that cannot run: the core's fixed text, or a generic one (nothing from the system, which may hold a path).
fn invalid(error: io::Error) -> CommandError {
    let text = if is_refusal(&error) {
        error.to_string()
    } else {
        "the search cannot run".to_owned()
    };

    CommandError::new(IpcErrorCode::InvalidInput, text)
}

/// What the page says when the folder cannot be searched.
fn search_error(error: &io::Error) -> IpcError {
    folder_error(error, "search this folder", "Cannot search this folder")
}

/// Starts a search of the folder `query.root` (absolute) and answers with its id at once; the events follow on `on_event`:
/// `found` batches, `progress`, `group` (duplicates), then one `done`, or one `error` when the folder cannot be
/// searched. A root or a pattern that cannot run rejects the command with `invalidInput`; a third search at once, `busy`.
#[tauri::command]
pub async fn find_start(
    finder: State<'_, Finder>,
    query: FindQuery,
    on_event: Channel<FindEvent>,
) -> CommandResult<FindId> {
    finder.start(&query, on_event)
}

/// Stops the search `id` (its `done` says `cancelled`); its results stay until it is forgotten. An unknown id is `notFound`.
#[tauri::command]
pub async fn find_cancel(finder: State<'_, Finder>, id: FindId) -> CommandResult<()> {
    finder.cancel(id)
}

/// Stops the search `id` and drops its results. An unknown id is `notFound`.
#[tauri::command]
pub async fn find_forget(finder: State<'_, Finder>, id: FindId) -> CommandResult<()> {
    finder.forget(id)
}

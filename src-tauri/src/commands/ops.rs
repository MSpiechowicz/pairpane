//! Copy and move jobs: the commands are thin adapters over `pairpane_core::ops::Engine`.
//!
//! Every path is checked for syntax here, on the command's thread, and never touched: `ops_start` hands the engine
//! paths that passed `check_folder` and plain names, and the engine resolves them on its own worker (see
//! `pairpane_core::ops`). The one exception is `ops_rename`, which is a single quick rename and so runs on a blocking
//! thread, at most [`RENAME_THREADS`] at once, because a rename on a hung mount never returns and must not use up the
//! thread pool (see `limit.rs`).
//!
//! The engine's events go to one channel the UI registers with `ops_subscribe` (the latest one, as `menu_subscribe`
//! does), so jobs keep running while the page reloads and a reloaded page learns about them from the snapshot.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pairpane_core::listing::platform;
use pairpane_core::model::{Answer, ControlAction, IpcError, IpcErrorCode, JobId, OpKind, OpsEvent};
use pairpane_core::ops::{
    Engine, MAX_NAMES, MAX_NAMES_BYTES, Request, Task, has_nested_names, is_item_path, name_fits, rename_in,
};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use self::extractions::Extractions;
use self::rename_undo::RenameUndo;
use super::limit::{Permit, ThreadCap};
use super::panes::resolve;
use super::within::{Within, confirm};
use crate::error::{CommandError, CommandResult};

/// Rename threads the app may have at once: a stuck one is stepped around by the other, and two stuck ones refuse
/// renames as busy without ever taking more of the thread pool.
const RENAME_THREADS: usize = 2;

/// Threads the app may have at once to check that the folder of search results is still the one searched (see
/// `within.rs`): the same stuck-folder rule as the renames, with places of their own.
const GUARD_THREADS: usize = 2;

/// How long quitting waits for the transfers to clean up after themselves (partial files, folders they made).
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// The latest channel the UI registered; `None` until it does.
type Listener = Arc<Mutex<Option<Channel<OpsEvent>>>>;

/// The transfer engine, the channel its events go to, the extraction jobs that run with a password and the caps on rename
/// and check threads.
pub struct Ops {
    engine: Engine,
    listener: Listener,
    extractions: Arc<Extractions>,
    rename_undo: RenameUndo,
    renames: ThreadCap,
    guards: ThreadCap,
}

impl Default for Ops {
    fn default() -> Self {
        let engine = Engine::new();
        let listener = Listener::default();

        let extractions = Arc::<Extractions>::default();

        let sink = Arc::clone(&listener);
        let watch = Arc::clone(&extractions);
        engine.set_sink(Arc::new(move |event| {
            // First: a password the job found wrong is forgotten before the page is told the job failed.
            watch.observe(&event);
            deliver(&sink, event);
        }));

        Self {
            engine,
            listener,
            extractions,
            rename_undo: RenameUndo::default(),
            renames: ThreadCap::default(),
            guards: ThreadCap::default(),
        }
    }
}

impl Ops {
    /// Makes `channel` the one events go to, replacing the earlier one, after it has received the engine's snapshot.
    ///
    /// The snapshot is taken and sent while the listener is locked, so an event the engine produces meanwhile waits
    /// behind it: the page sees the snapshot first, then what happened after it. (An event produced just before the
    /// snapshot may still arrive after it; the UI treats events for state it already has as updates.)
    fn subscribe(&self, channel: Channel<OpsEvent>) {
        let mut listener = lock(&self.listener);

        // A closed channel only means the page went away; it subscribes again when it loads.
        channel.send(self.engine.snapshot()).ok();
        *listener = Some(channel);
    }

    /// Cancels every job, as the app quits, and waits a moment ([`SHUTDOWN_WAIT`]) for each to remove its partial
    /// file and the folders it made. A job stuck on a folder that does not answer is left behind.
    pub fn shutdown(&self) {
        self.engine.shutdown(SHUTDOWN_WAIT);
    }

    /// Validates the request, checks the folder of search results when `within` says so, then queues it. Touches no disk
    /// otherwise.
    async fn start(
        &self,
        kind: OpKind,
        from: String,
        names: Vec<String>,
        to: String,
        verify: bool,
        within: Option<Within>,
    ) -> CommandResult<JobId> {
        let request = request(kind, from, names, to, verify)?;
        confirm(&self.guards, GUARD_THREADS, within, &request.from.to_string_lossy()).await?;

        self.engine.submit(request).map_err(failed)
    }

    /// Takes one of the rename threads, or refuses as busy.
    fn begin_rename(&self) -> CommandResult<Permit> {
        self.renames.acquire(RENAME_THREADS).ok_or_else(|| {
            CommandError::new(
                IpcErrorCode::Busy,
                "a folder is not responding; try renaming again shortly",
            )
        })
    }

    /// Renames `from` to `to` inside the folder `path`, never replacing anything, on a blocking thread.
    async fn rename(&self, path: String, from: String, to: String) -> CommandResult<()> {
        let source = resolve(Some(path.clone()), Some(&from))?;
        resolve(Some(path), Some(&to))?;
        if !name_fits(&to) {
            return Err(names_too_large());
        }
        let folder = source.parent().map(ToOwned::to_owned).ok_or_else(unnamed)?;
        let permit = self.begin_rename()?;

        let renamed = async_runtime::spawn_blocking(move || {
            // The place is held until the rename ends, however it ends.
            let _permit = permit;

            // First of all: a link on the path may lead to a network or device path, which Windows would connect to.
            // Only the path is read here, and not on the command's thread.
            let actual = platform::local_path(&folder)?;

            rename_in(&actual, &from, &to)
        })
        .await;

        match renamed {
            Ok(result) => result.map_err(|error| rename_error(&error)),
            Err(_) => Err(CommandError::new(IpcErrorCode::Internal, "the rename was interrupted")),
        }
    }
}

/// Registers the channel transfer events are sent to: first a `snapshot` of the jobs that have not ended and the
/// questions still waiting, then whatever happens next. The UI calls it when it loads; a later call replaces the
/// earlier channel (e.g. after a reload).
#[tauri::command]
pub fn ops_subscribe(ops: State<'_, Ops>, on_event: Channel<OpsEvent>) {
    ops.subscribe(on_event);
}

/// Queues a copy or move of the `names` in the folder `from` into the folder `to` and returns its id at once.
/// Nothing is read from the disk here: a folder that does not exist, or one copied into itself, ends the job with
/// a failed `ended` event. The one exception is a request with `within` (the folder `from` of search results): it is
/// first checked to be the folder that was searched (see `within.rs`) and refused when it is not.
#[tauri::command]
pub async fn ops_start(
    ops: State<'_, Ops>,
    kind: OpKind,
    from: String,
    names: Vec<String>,
    to: String,
    verify: bool,
    within: Option<Within>,
) -> CommandResult<JobId> {
    ops.start(kind, from, names, to, verify, within).await
}

/// Pauses, resumes or cancels a job.
#[tauri::command]
pub async fn ops_control(ops: State<'_, Ops>, job: JobId, action: ControlAction) -> CommandResult<()> {
    ops.engine.control(job, action).map_err(failed)
}

/// Answers the open conflict `question` of `job`; with `all`, for the job's later conflicts too.
#[tauri::command]
pub async fn ops_answer(
    ops: State<'_, Ops>,
    job: JobId,
    question: u32,
    answer: Answer,
    all: bool,
) -> CommandResult<()> {
    ops.engine.answer(job, question, answer, all).map_err(failed)
}

/// Renames `from` to `to` inside the folder `path`; refused when `to` exists.
#[tauri::command]
pub async fn ops_rename(ops: State<'_, Ops>, path: String, from: String, to: String) -> CommandResult<()> {
    ops.rename(path, from, to).await
}

/// Puts back the last multi-rename, which the job `job` was (the id `ops_task` returned), and returns the id of the job that
/// does it. Not found when `job` is not the most recent rename, was undone already or renamed nothing; invalid input while
/// it still runs. Items renamed again or removed since are skipped by the job (it counts them), never overwritten.
#[tauri::command]
pub async fn ops_rename_undo(ops: State<'_, Ops>, job: JobId) -> CommandResult<JobId> {
    ops.rename_undo.undo(&ops.engine, job)
}

/// Checks a request from the webview and builds the engine's: both folders pass `check_folder`, and the names pass
/// [`checked_names`].
fn request(kind: OpKind, from: String, names: Vec<String>, to: String, verify: bool) -> CommandResult<Request> {
    let from = resolve(Some(from), None)?;
    let to = resolve(Some(to), None)?;
    let names = checked_names(&from, names)?;

    Ok(Request {
        kind,
        from,
        names,
        to,
        verify,
        task: Task::Transfer,
    })
}

/// Checks the names of the items to change inside `folder`: every one is a plain name inside it and short enough (the
/// names together take at most [`MAX_NAMES_BYTES`], counted before duplicates are dropped); duplicates are dropped
/// (first one kept) and what is left is between 1 and [`MAX_NAMES`].
pub(super) fn checked_names(folder: &Path, names: Vec<String>) -> CommandResult<Vec<String>> {
    unique_names(names, |name| check_name(folder, name))
}

/// [`checked_names`] for the items of a pack, which may also be paths below `folder` (`a/b.txt`, each part a plain
/// name that fits): none may be inside another (`a` and `a/b`).
pub(super) fn checked_item_paths(folder: &Path, names: Vec<String>) -> CommandResult<Vec<String>> {
    let names = unique_names(names, |name| check_item_path(folder, name))?;

    if has_nested_names(&names) {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "an item cannot be chosen together with a folder that holds it",
        ));
    }

    Ok(names)
}

/// The names of `names` that are not repeated, each passing `check` and every part of it short enough.
fn unique_names(names: Vec<String>, check: impl Fn(&str) -> CommandResult<()>) -> CommandResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    let mut bytes = 0_usize;
    for name in names {
        // First: the size of a name bounds the work on it, and the total bounds the work on the list.
        bytes = bytes.saturating_add(name.len());
        if !name.split('/').all(name_fits) || bytes > MAX_NAMES_BYTES {
            return Err(names_too_large());
        }
        check(&name)?;

        if seen.contains(&name) {
            continue;
        }
        // Stops at the first name over the limit, so a huge list is not walked to the end.
        if unique.len() == MAX_NAMES {
            return Err(item_count());
        }
        seen.insert(name.clone());
        unique.push(name);
    }
    if unique.is_empty() {
        return Err(item_count());
    }

    Ok(unique)
}

/// An item of a pack is a plain name or a relative path of them whose path inside `folder` passes the folder rules.
fn check_item_path(folder: &Path, name: &str) -> CommandResult<()> {
    if !is_item_path(name) {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "an item must be a plain name or a path of plain names below the folder",
        ));
    }

    platform::check_folder(&folder.join(name)).map_err(|reason| CommandError::new(IpcErrorCode::InvalidInput, reason))
}

/// A name is a single plain name whose path inside `folder` passes the folder rules.
pub(super) fn check_name(folder: &Path, name: &str) -> CommandResult<()> {
    if !platform::is_plain_name(name) {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "an item name must be a single plain name",
        ));
    }

    platform::check_folder(&folder.join(name)).map_err(|reason| CommandError::new(IpcErrorCode::InvalidInput, reason))
}

fn item_count() -> CommandError {
    CommandError::new(
        IpcErrorCode::InvalidInput,
        format!("choose between 1 and {MAX_NAMES} different items"),
    )
}

pub(super) fn names_too_large() -> CommandError {
    CommandError::new(
        IpcErrorCode::InvalidInput,
        "an item name, or the names together, are too long",
    )
}

pub(super) fn unnamed() -> CommandError {
    CommandError::new(IpcErrorCode::InvalidInput, "an item name must be a single plain name")
}

/// What the page is told when a rename fails. Never a path: only the kind of failure.
fn rename_error(error: &io::Error) -> CommandError {
    let (code, message) = match error.kind() {
        io::ErrorKind::AlreadyExists => (
            IpcErrorCode::InvalidInput,
            "An item with that name already exists.".to_owned(),
        ),
        io::ErrorKind::NotFound => (IpcErrorCode::NotFound, "This item no longer exists.".to_owned()),
        io::ErrorKind::PermissionDenied => (
            IpcErrorCode::Io,
            "You do not have permission to rename this item.".to_owned(),
        ),
        io::ErrorKind::InvalidInput => (IpcErrorCode::InvalidInput, error.to_string()),
        _ => (IpcErrorCode::Io, format!("Cannot rename this item: {error}")),
    };

    CommandError::new(code, message)
}

/// A refusal as a command error.
pub(super) fn failed(error: impl Into<IpcError>) -> CommandError {
    let error = error.into();

    CommandError::new(error.code, error.message)
}

/// Sends `event` to the listener's channel, if it has one.
fn deliver(listener: &Listener, event: OpsEvent) {
    if let Some(channel) = lock(listener).as_ref() {
        // A closed channel only means the page went away; the job goes on and the page asks again when it loads.
        channel.send(event).ok();
    }
}

fn lock(listener: &Listener) -> MutexGuard<'_, Option<Channel<OpsEvent>>> {
    listener.lock().unwrap_or_else(PoisonError::into_inner)
}

pub mod add;
pub mod extract;
mod extractions;
pub mod pack;
mod rename_undo;
pub mod tasks;

#[cfg(test)]
mod rename_tests;
#[cfg(test)]
mod tasks_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod within_tests;

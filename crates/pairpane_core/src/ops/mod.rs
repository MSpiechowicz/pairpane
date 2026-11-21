//! Copy and move jobs with a background queue: the public API of the engine.
//!
//! A job copies or moves the named items of one folder into another. It runs on a worker thread of the engine,
//! never on the caller's thread, and reports through a [`Sink`] of [`OpsEvent`]s.
//!
//! # Contract
//!
//! - Jobs are queued per destination volume: jobs for one volume run in submit order, jobs for different
//!   volumes run in parallel, up to [`MAX_WORKERS`] at once. At most [`MAX_JOBS`] jobs (queued plus running)
//!   exist; beyond that `submit` refuses with [`Refused::Busy`].
//! - `submit` does no disk access. Resolving the paths and every check that needs the disk (a folder into
//!   itself, a missing source) run in the worker and fail the job through [`OpsEvent::Ended`].
//! - Every accepted job produces [`OpsEvent::Added`] first and exactly one [`OpsEvent::Ended`] last, whatever
//!   happens, including `cancel_all` and a panic. (One exception to the order: a cancel that lands just as a
//!   job finishes by itself can send its `phase` event right after `ended`; a listener ignores events of a job
//!   it no longer knows.)
//! - Events reach the sink on engine threads or on the caller's thread, never while an engine lock is held, so
//!   a sink may call back into the engine.
//!
//! # What a copy does to the filesystem
//!
//! Nothing is ever read or written through a link: links are copied as links, and a file is opened in a way that
//! makes the system refuse a link. Each file is written to a temporary file (`.pairpane-<run>-<job>-<n>.part`, the
//! run being a token of this start of the app) next to its target, created exclusively, and renamed into place when
//! complete, so a target is absent or whole. A name that is taken is skipped, never opened. Pipes, sockets and
//! devices are never opened.
//!
//! "The same file" and "into itself" are decided by identity (device and inode; on Windows volume serial and file
//! index), not by the spelling of a path. A move never deletes a source that is the committed target.
//!
//! # Known limits, accepted
//!
//! - The walk lists folders by path (`read_dir`, `symlink_metadata`), not through handles (`openat`): a folder swapped
//!   for a link between the moment the walk read it and the moment it is listed is the same race, needing write
//!   access to the folders involved, that `resolve_local` documents. Files are opened without following links, and
//!   nothing is ever written through a link that is already there.
//! - On Windows the paths are resolved once and then used by name; a link swapped in later is the same race.
//! - A worker stuck on a folder that does not answer (a hung mount) keeps its slot, and so does a lookup of its
//!   volume; [`Engine::shutdown`] gives up on it after its timeout.
//! - Two jobs for one volume start in submit order once both volumes are known; while an earlier job's volume is
//!   still being looked up, a later job that is ready does not wait for it (see `engine`).
//!
//! # Modules
//!
//! `engine` (queue, workers, public API), `job` (one job's shared state), `control` (pause, cancel, answers),
//! `validate` (what `submit` checks), `volume` (scheduling key), `transfer` (one job from start to end), `plan`
//! (looking before copying), `identity` (what makes two paths one file), `copy` (folders, files, links), `fast`
//! (clone, kernel copy, buffered loop), `open` and `meta` (no-follow opens; times and permissions), `conflict`
//! (existing targets and their questions), `verify` (BLAKE3), `rename` (no-replace rename), `progress` and `rate`
//! (counters, speed), `report` (error texts), `hooks` (test switches).
//!
//! Outside the engine, single changes that are not jobs: `items` (new folders and files, links, permanent
//! delete) and `trash` (move to the Trash, put back). Each is a plain blocking function; the caller runs it off the
//! command thread.

mod announce;
mod conflict;
mod control;
mod copy;
mod delete;
mod engine;
mod fast;
mod hooks;
mod identity;
mod items;
mod job;
mod meta;
mod open;
mod plan;
mod progress;
mod rate;
mod rename;
mod report;
mod runnable;
mod transfer;
mod trash;
mod validate;
mod verify;
mod volume;

#[cfg(test)]
mod announce_tests;
#[cfg(test)]
mod conflict_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod failure_tests;
#[cfg(test)]
mod items_tests;
#[cfg(test)]
mod move_tests;
#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod transfer_tests;
#[cfg(test)]
mod trash_tests;

pub use engine::Engine;
pub use items::{ItemFailure, MAX_FOLDER_DEPTH};
pub use trash::{CAN_UNTRASH, Trashed, UntrashFailure};
pub use validate::name_fits;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::{IpcError, IpcErrorCode, LinkKind, OpKind, OpsEvent};

/// Most destination volumes that copy at the same time.
pub const MAX_WORKERS: usize = 4;

/// Most jobs that exist at once, queued and running together.
pub const MAX_JOBS: usize = 64;

/// Most items one job may start with.
pub const MAX_NAMES: usize = 100_000;

/// Longest file name of a request: bytes, and on Windows UTF-16 units (see [`name_fits`]).
pub const MAX_NAME_UNITS: usize = 255;

/// Most bytes the names of one request may take together.
pub const MAX_NAMES_BYTES: usize = 8 * 1024 * 1024;

/// Where the engine sends its events.
pub type Sink = Arc<dyn Fn(OpsEvent) + Send + Sync>;

/// A copy or move to start. Paths are what the webview sent, already checked for syntax by the caller
/// (`check_folder` for the folders, `is_plain_name` for every name); the engine resolves them on its worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: OpKind,
    /// The folder the items are in.
    pub from: PathBuf,
    /// The items, as plain names inside `from`. At least 1 and at most [`MAX_NAMES`], without duplicates.
    pub names: Vec<String>,
    /// The folder to put the items into; created when missing.
    pub to: PathBuf,
    /// Re-read and hash each copied file before committing it.
    pub verify: bool,
}

/// Why `submit` did not accept a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// [`MAX_JOBS`] jobs exist already; trying again later may work.
    Busy,
    /// The request has no names, more than [`MAX_NAMES`], a name twice, or a name that is not a plain file name.
    InvalidNames,
    /// A name is longer than [`MAX_NAME_UNITS`], or the names together take more than [`MAX_NAMES_BYTES`].
    NamesTooLarge,
}

/// A job that is not known: it never existed, or it has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownJob;

/// Why `answer` was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerError {
    /// The job is not known (see [`UnknownJob`]).
    UnknownJob,
    /// The job has no open question with that number: it was answered already, or the number is made up.
    StaleQuestion,
}

impl From<Refused> for IpcError {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::Busy => Self::new(IpcErrorCode::Busy, "too many transfers are running; try again shortly"),
            Refused::InvalidNames => Self::new(
                IpcErrorCode::InvalidInput,
                "choose between 1 and 100000 different items",
            ),
            Refused::NamesTooLarge => Self::new(
                IpcErrorCode::InvalidInput,
                "an item name, or the names together, are too long",
            ),
        }
    }
}

impl From<UnknownJob> for IpcError {
    fn from(_: UnknownJob) -> Self {
        Self::new(IpcErrorCode::NotFound, "that transfer is not running")
    }
}

impl From<AnswerError> for IpcError {
    fn from(error: AnswerError) -> Self {
        match error {
            AnswerError::UnknownJob => UnknownJob.into(),
            AnswerError::StaleQuestion => Self::new(IpcErrorCode::InvalidInput, "that question is no longer open"),
        }
    }
}

/// Renames `from` to `to` inside `folder`, never replacing anything. For the rename shortcut, not a job.
///
/// `folder` is a resolved local path, and `from` and `to` are plain names (no separators), checked by the
/// caller. Blocking disk access: call it off the command thread.
///
/// # Errors
///
/// [`io::ErrorKind::AlreadyExists`] when `to` exists, and [`io::ErrorKind::NotFound`] when `from` does not. A
/// change of case only (`a.txt` to `A.txt`) succeeds on a case-insensitive filesystem, where `to` already
/// "exists" as `from` itself.
pub fn rename_in(folder: &Path, from: &str, to: &str) -> io::Result<()> {
    rename::rename_in(folder, from, to)
}

/// Splits the path of a new folder into its parts, checking each: `a/b/c` gives `["a", "b", "c"]`.
///
/// The separator is `/`, and on Windows `\` too. Pure: no disk access.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidInput`] when a part is empty (`/a`, `a//b`, `a/`, or the whole path), is not a plain
/// name (`.`, `..`, a Windows device name), is longer than [`MAX_NAME_UNITS`], or when there are more than
/// [`MAX_FOLDER_DEPTH`] parts. The text of the error holds no path.
pub fn folder_steps(relative: &str) -> io::Result<Vec<&str>> {
    items::folder_steps(relative)
}

/// Creates the folders of `relative` (see [`folder_steps`]) one inside the other, starting in `folder`, and returns
/// the first part. Blocking disk access: call it off the command thread.
///
/// A real folder that is on the way already is used as it is. The last folder must be new. Folders made before
/// a failure stay.
///
/// # Errors
///
/// Those of [`folder_steps`]; [`io::ErrorKind::AlreadyExists`] when the last folder exists; and
/// [`io::ErrorKind::InvalidInput`] when a name on the way is a link or a file, because nothing is made through a
/// link. Others come from the system, such as `NotFound` for a `folder` that is gone.
pub fn create_folders(folder: &Path, relative: &str) -> io::Result<String> {
    items::create_folders(folder, relative)
}

/// Creates the empty file `name` in `folder` and returns its path. Blocking disk access.
///
/// The file gets the usual permissions of a new file (not the private ones of a file being copied). A link at
/// the name, even one that leads nowhere, counts as taken.
///
/// # Errors
///
/// [`io::ErrorKind::AlreadyExists`] when anything is at the name, [`io::ErrorKind::InvalidInput`] when `name` is
/// not a plain name.
pub fn create_file(folder: &Path, name: &str) -> io::Result<PathBuf> {
    items::create_file(folder, name)
}

/// Whether the system would run a file with this `name` when it is opened: a program or a script, by its extension
/// (case does not matter). A fixed list on every system, so the answer does not depend on where the app runs.
#[must_use]
pub fn is_runnable(name: &str) -> bool {
    runnable::is_runnable(name)
}

/// Makes the link `name` in `folder`, leading to `target` (an absolute path). Blocking disk access.
///
/// A symbolic link stores `target` as given, not resolved. On Windows it is made as a folder link or a file link
/// by what `target` is. A hard link needs a regular file: a folder or a link is refused.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidInput`] for a name that is not plain, a relative `target`, or a hard link to
/// something that is not a regular file; [`io::ErrorKind::AlreadyExists`] when the name is taken;
/// [`io::ErrorKind::CrossesDevices`] for a hard link to another disk. On Windows a symbolic link without
/// Developer Mode or administrator rights fails with a text that says so. The texts hold no path.
pub fn make_link(kind: LinkKind, target: &Path, folder: &Path, name: &str) -> io::Result<()> {
    items::make_link(kind, target, folder, name)
}

/// Deletes the items `names` of `folder` for good, in order, and stops at the first that fails. Blocking disk access.
///
/// A folder goes with everything in it. A link goes alone: what it leads to is never touched.
///
/// # Errors
///
/// The [`ItemFailure`] of the item that failed: [`io::ErrorKind::NotFound`] when it is gone,
/// [`io::ErrorKind::InvalidInput`] when a name is not plain or (Unix) the item is or holds a mounted drive, which is
/// refused whole before anything of it is removed, others from the system. The items before it are gone.
pub fn delete_permanently(folder: &Path, names: &[String]) -> Result<(), ItemFailure> {
    items::delete_permanently(folder, names)
}

/// Moves the items `names` of `folder` to the Trash (the Recycle Bin on Windows), in order, and stops at the first
/// that fails. Blocking disk access, and on Windows it may be slow.
///
/// Returns what [`untrash`] needs to find the items again, listing only those that did reach the Trash (none, when
/// the first one failed), and the failure of the item that stopped the move, if one did. A link is moved as the
/// link. An error never holds a path.
///
/// On Linux (and the other freedesktop systems) an item is renamed into the Trash folder of its own drive, which must
/// be the user's own real folder that nobody else can rename entries in (made when missing); where there is none it is
/// refused (`Other`) and never copied to another disk.
pub fn trash(folder: &Path, names: &[String]) -> (Trashed, Result<(), ItemFailure>) {
    trash::trash(folder, names)
}

/// Puts back the items of an earlier [`trash`] that are still in the Trash, and returns how many. Blocking disk
/// access.
///
/// Only [`CAN_UNTRASH`] systems can: others answer [`UntrashFailure::Unsupported`]. On Linux each item is taken from the
/// Trash folder and the name it was moved to, as the record says, and put back only into the folder it came from (the
/// same folder, not one of the same name); on Windows the items are found by name, folder and the time of the move, so
/// something trashed from the same folder under the same name in those few seconds is taken for one of them.
///
/// # Errors
///
/// See [`UntrashFailure`].
pub fn untrash(record: &Trashed) -> Result<usize, UntrashFailure> {
    trash::untrash(record)
}

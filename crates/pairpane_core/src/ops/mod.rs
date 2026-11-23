//! Copy, move and file-tool jobs with a background queue: the public API of the engine.
//!
//! A job copies or moves the named items of one folder into another, or does one of the file tools to them (a
//! checksum file, the check of one, splitting a file, joining its parts, changing permissions and times; see
//! [`Task`]). It runs on a worker thread of the engine, never on the caller's thread, and reports through a
//! [`Sink`] of [`OpsEvent`]s.
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
//! # What the file tools do to the filesystem
//!
//! The tools read and change files by the same rules: nothing is opened through a link (a link in the selection, or
//! inside a selected folder, is skipped and counted, never followed), and what a tool writes goes to a temporary
//! file first and is put in place with a rename that refuses to replace, so a name that is taken is never
//! overwritten. A checksum file, the parts of a split and the file a combine joins are removed again when the job is
//! cancelled or fails. Changing permissions and times works on the item itself and never on what a link leads to:
//! times through the folder and the name (no-follow), permissions through a handle opened without following links.
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
//! - Changing the permissions of an item that cannot be opened (no read permission, so a no-follow open fails) goes
//!   through a handle taken from the folder's own handle that needs no permission and never follows a link (Linux: an
//!   `O_PATH` handle changed through `/proc/self/fd`; where `/proc` is missing the item is refused; other Unix: `fstatat`
//!   and `fchmodat` with no-follow). It is checked to be the item that was seen. What is left: the item can be replaced
//!   by another file of the same owner and kind between the check and the change only in a folder the user can write to,
//!   and then the one changed is a file inside that folder.
//! - The checksum file and the attribute change walk the tree by handle on Unix (`held`): the job's folder is opened once
//!   (a link to it is followed, as the pane showed it) and every step after is relative to a handle, compared with what
//!   was seen, so a folder swapped for a link during the walk is refused and nothing is read or changed through it. The
//!   checksum check does the same for each file it lists (`listed`). A copy, a move, a split and a combine still use
//!   paths below the job's folder (files are opened without following a link). On Windows all of them check the way by
//!   name and open without following a link: a folder swapped for a link between the look and the open is the same race
//!   the other Windows jobs have, needing write access to the folders involved.
//! - On Windows the read-only attribute is set on files only (a folder takes the other attributes and the times); hidden,
//!   system and archive are set on files and folders, through the handle opened without following links, by the one
//!   `unsafe` call of the workspace (`win_attrs`). The other attributes (compressed, encrypted, indexing and so on) are
//!   shown but not changed.
//! - Two jobs for one volume start in submit order once both volumes are known; while an earlier job's volume is
//!   still being looked up, a later job that is ready does not wait for it (see `engine`).
//!
//! # Modules
//!
//! `engine` (queue, workers, public API), `job` (one job's shared state), `control` (pause, cancel, answers),
//! `request` (the request, its task and the refusals), `validate` (what `submit` checks), `volume` (scheduling key), `transfer` (one job from start to end), `plan`
//! (looking before copying), `identity` (what makes two paths one file), `copy` (folders, files, links), `fast`
//! (clone, kernel copy, buffered loop), `open` and `meta` (no-follow opens; times and permissions), `conflict`
//! (existing targets and their questions), `verify` (BLAKE3), `rename` (no-replace rename), `progress` and `rate`
//! (counters, speed), `report` (error texts), `hooks` (test switches).
//!
//! Extracting an archive: `extract` (the job: open, select, walk the source) and `extract_write` (folders, files, the
//! disk reserve, modes and times). Packing files into one: `pack` (the job: plan the entries, ask about a taken name, write
//! a temporary file, put it in place) over `tree` (the no-follow walk it shares with the checksum file). Adding files to an archive: `add` (the job: plan the new entries
//! against the archive's index, ask about the files it has, write a new archive beside it with [`crate::archive::rewrite`], replace
//! the old one). The archive formats themselves are in [`crate::archive`].
//!
//! The file tools: `task` (the start of each and what they share), `digest` (checksums and the checksum-file
//! format), `checksum`, `checksum_verify` and `listed` (opening the files it lists), `split`, `combine`, `held` (walking a tree by
//! handle), `attrs` and `attrs_apply`, `partial` (temporary files that clean up after themselves) and `props` (the
//! Properties of an item).
//!
//! Renaming many items at once (multi-rename and its undo): `batch_rename` (the job: moves a name out of the way before another
//! item takes it, so a swap, a cycle or a chain works and nothing is replaced), `held_rename` (a rename that refuses to replace,
//! inside a folder held open) and `rename_log` (the pairs it renamed, which an undo reads). The rule that makes the new names
//! is in [`crate::rename`].
//!
//! Synchronizing two compared folders: `sync` (the job: look at every item again, copy it with the checks of a copy) and
//! `sync_archive` (the items that come out of an archive, extracted by the writing of `extract_write`). What the job promises
//! is the text of `sync`.
//!
//! Outside the engine, single changes that are not jobs: `items` (new folders and files, links, permanent
//! delete) and `trash` (move to the Trash, put back). Each is a plain blocking function; the caller runs it off the
//! command thread.

mod add;
mod announce;
mod attrs;
mod attrs_apply;
mod batch_rename;
mod checksum;
mod checksum_verify;
mod combine;
mod conflict;
mod control;
mod copy;
mod delete;
mod digest;
mod engine;
mod extract;
mod extract_write;
mod fast;
mod held;
mod held_rename;
mod hooks;
mod identity;
mod items;
mod job;
mod listed;
mod meta;
mod open;
mod origin;
mod pack;
mod pack_items;
mod partial;
mod plan;
mod progress;
mod props;
mod rate;
mod rename;
mod rename_log;
mod report;
mod request;
mod runnable;
mod split;
mod sync;
mod sync_archive;
mod sync_aside;
mod sync_holding;
mod sync_into;
mod sync_log;
mod sync_remove;
mod sync_terms;
mod sync_undo;
mod sync_unlink;
#[cfg(unix)]
mod sync_write;
mod task;
mod transfer;
mod trash;
mod tree;
mod validate;
mod verify;
mod volume;
#[cfg(any(windows, test))]
mod win_attrs;

/// Checks that the folder `dir` (names joined with `/`, `""` for `base` itself) can be reached below `base` one name at a
/// time with no link and no file on the way (the search results of a folder are read through it).
pub(crate) fn check_folder_below(base: &std::path::Path, dir: &str) -> std::io::Result<()> {
    listed::check_folder_below(base, dir)
}

/// [`check_folder_below`], and the folder reached is, now, the folder `target` (same device and inode, or the Windows
/// equivalent): a link or file on the way, and a `target` that is another folder than the one the names reach, are
/// refused, and so is one whose identity cannot be read. The identity of the folder the search found is not kept, so a
/// real folder moved into place after the search is not seen. The check and the operation are two steps; what a folder
/// swapped in between them does is the race every pane has.
pub fn check_same_folder_below(base: &std::path::Path, dir: &str, target: &std::path::Path) -> std::io::Result<()> {
    listed::check_same_folder_below(base, dir, target)
}

/// Opens the regular file `listed` below `base` for reading, with its metadata, the way a checksum check opens the files
/// it lists (no link followed, no pipe waited on); the file viewer opens its file through this.
pub(crate) fn open_listed(base: &std::path::Path, listed: &str) -> std::io::Result<(std::fs::File, std::fs::Metadata)> {
    listed::open_listed(base, listed)
}

#[cfg(test)]
mod add_conflict_tests;
#[cfg(test)]
mod add_refusal_tests;
#[cfg(test)]
mod add_tests;
#[cfg(all(test, unix))]
mod add_unix_tests;
#[cfg(test)]
mod announce_tests;
#[cfg(test)]
mod attrs_tests;
#[cfg(all(test, unix))]
mod attrs_unix_tests;
#[cfg(test)]
mod batch_rename_hook_tests;
#[cfg(test)]
mod batch_rename_tests;
#[cfg(test)]
mod checksum_tests;
#[cfg(test)]
mod combine_tests;
#[cfg(test)]
mod conflict_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod digest_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod extract_scripted_tests;
#[cfg(test)]
mod extract_tests;
#[cfg(test)]
mod failure_tests;
#[cfg(test)]
mod folder_below_tests;
#[cfg(all(test, unix))]
mod held_tests;
#[cfg(test)]
mod items_tests;
#[cfg(test)]
mod move_tests;
#[cfg(test)]
mod origin_tests;
#[cfg(test)]
mod pack_conflict_tests;
#[cfg(test)]
mod pack_items_tests;
#[cfg(test)]
mod pack_tests;
#[cfg(all(test, unix))]
mod pack_unix_tests;
#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod props_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod split_tests;
#[cfg(test)]
mod sync_archive_tests;
#[cfg(test)]
mod sync_into_tests;
#[cfg(test)]
mod sync_race_tests;
#[cfg(test)]
mod sync_refusal_tests;
#[cfg(all(test, unix))]
mod sync_remove_link_tests;
#[cfg(test)]
mod sync_remove_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod sync_undo_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod transfer_tests;
#[cfg(test)]
mod trash_tests;

pub(crate) use digest::Digester;
pub use engine::Engine;
pub use extract::ArchiveJob;
pub use items::{ItemFailure, MAX_FOLDER_DEPTH};
#[cfg(windows)]
pub(crate) use open::OPEN_REPARSE_POINT;
pub(crate) use open::create_new;
pub(crate) use partial::{Partial, run_token};
pub(crate) use plan::{Child, Kind, is_inside, read_children, real};
pub use rename_log::RenameLog;
pub use request::{AnswerError, PackJob, Refused, Request, SyncPlan, SyncRemove, SyncRoot, Task, UnknownJob};
pub use sync_aside::HOLDING_PREFIX;
pub use sync_log::SyncLog;
pub(crate) use transfer::MAX_DEPTH;
pub use trash::{CAN_UNTRASH, Trashed, UntrashFailure};
pub(crate) use validate::fold_case;
pub use validate::{has_nested_names, is_item_path, name_fits};

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::{ItemProperties, LinkKind, OpsEvent};

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

/// The shortest part a split may make, in bytes.
pub const MIN_PART_SIZE: u64 = 1_024;

/// The most parts a split may make: the numbers of the parts (`.001`) have three digits.
pub const MAX_PARTS: u64 = 999;

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

/// What the Properties dialog shows about the item `name` of `folder`: kind, size, times, permissions, owner and group,
/// the Windows attributes and the target of a link. Blocking disk access: call it off the command thread.
///
/// `folder` is a resolved local path. A link is shown by what it leads to (as the panes do), unless that cannot be
/// reached, and its own target is given as written.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidInput`] when `name` is not a plain name, [`io::ErrorKind::NotFound`] when the item is gone,
/// others from the system. The text of the error holds no path.
pub fn properties(folder: &Path, name: &str) -> io::Result<ItemProperties> {
    props::properties(folder, name)
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

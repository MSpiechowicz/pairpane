//! The extra columns of the detailed view: `columns_dimensions` (the size of the pictures in view), `columns_checksum`
//! (the SHA-256 of one file) and `columns_git` (the git status of the entries of the folder). The reading is
//! `pairpane_core::view::columns` (see its module docs for why a size is read in the app's own process and what bounds it)
//! and `pairpane_core::gitstatus` (see its module docs for why reading a repository's status starts nothing it names);
//! this module is what stands around those.
//!
//! - What the page sent is checked before a thread is taken: the folder passes `resolve`, a call has at most
//!   [`MAX_NAMES`] names, and a checksum's name is one plain name (`check_name`) that fits. A name of a batch that is not
//!   such a name has `None` in its place, as it would for any file that cannot be read, so that one odd name never makes
//!   the page give up on the other 511.
//! - The folder is followed as the page sent it (a link inside its path is followed, as for thumbnails; the page joins a
//!   result's folder and its sub-folder); only the last component, the file itself, is never followed.
//! - The disk is touched on blocking threads that hold a place of a [`ThreadCap`] until they end (a file on a mount that
//!   does not answer holds its place for good and must not use up the pool): up to [`SIZE_THREADS`] batches of sizes, and
//!   [`HASH_THREADS`] checksums, which read up to 64 MiB each. A call that finds its cap full is refused as `busy`, and
//!   the page asks again.
//! - A batch of sizes is answered in the order of its names. A name that is not a regular file (a link is not followed, a
//!   folder, a missing file, anything), that cannot be read or is no picture of a format that is read has `None`; one
//!   file never fails the batch.
//! - A checksum is refused as `invalidInput` with a fixed text when the file is over the cap, and as `notFound` or `io`
//!   by the viewer's texts (`view_error`) for the other failures. No text sent to the page holds a path.
//! - A git status is up to [`GIT_THREADS`] at once, and `busy` when they are taken. It is bounded by `Limits` (5 seconds and
//!   200,000 entries): a repository that is over the entries is `invalidInput` ("the repository is too big to read here"),
//!   one that takes the time is `invalidInput` too ("the repository took too long to read"), and a folder that is in no
//!   repository (or in one that must not be read: a pointer to a network place, a pipe where a file is expected, a format
//!   that is not read) is `repo: false`, not an error. A folder that cannot be resolved is `notFound` or `io`, never
//!   `invalidInput`. The texts hold no path.
//! - A git status answers by its deadline whether or not its read has come back (`stuck`): it runs on a thread of its own,
//!   which the request waits for until the time limit and [`GIT_GRACE`] have passed; then the request answers as for the time
//!   limit, gives its place of the [`GIT_THREADS`] back, and the thread counts as stuck until it ends. A status holds a place
//!   in the count of [`stuck::MAX_STUCK`] (2) for as long as it runs and when it is stuck, and takes it before its thread is
//!   started, so that the count of those that run or are stuck is never over it; when it is full a status is refused as `busy`.
//!   The count is small because a thread that is stuck may be one that spends a processor (one object of a long chain of
//!   deltas cannot be interrupted, and gix has no hook for it): at most two can.
//! - [`Columns::shut_down`] raises the flag all of them watch, as the app exits: a checksum stops between its chunks, a
//!   batch between its files and a git status at its next entry, and none starts after.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pairpane_core::gitstatus::{self, GitError, Limits};
use pairpane_core::listing::platform;
use pairpane_core::model::{Dimensions, GitStatus, IpcErrorCode};
use pairpane_core::ops::name_fits;
use pairpane_core::view::columns as read;
use tauri::State;

use super::items::{begin, blocking};
use super::limit::ThreadCap;
use super::ops::{check_name, names_too_large};
use super::panes::resolve;
use super::view::view_error;
use crate::error::{CommandError, CommandResult};
use stuck::{Stuck, Unanswered};

mod stuck;

#[cfg(test)]
mod git_tests;
#[cfg(test)]
mod tests;

/// The most names one call of `columns_dimensions` takes.
const MAX_NAMES: usize = 512;

/// Batches of sizes read at once: each reads a small piece of up to [`MAX_NAMES`] files, one after the other.
const SIZE_THREADS: usize = 4;

/// Checksums computed at once: each reads a file of up to 64 MiB.
const HASH_THREADS: usize = 2;

/// Git statuses read at once: each walks the folder's subtree of a repository, up to the cap on entries.
const GIT_THREADS: usize = 2;

/// How long a git status is waited for after its time limit, before the request answers without it.
const GIT_GRACE: Duration = Duration::from_millis(500);

/// The caps on the blocking threads and the flag that stops them when the app exits.
#[derive(Default)]
pub struct Columns {
    sizes: ThreadCap,
    hashes: ThreadCap,
    statuses: ThreadCap,
    /// The threads of statuses that were given up on and are still running.
    stuck: Stuck,
    cancel: Arc<AtomicBool>,
}

impl Columns {
    /// Stops the checksums and batches that run and refuses any that comes after: the app is exiting.
    pub fn shut_down(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The sizes of the pictures `names` of the folder `path`, in the order of the names.
    async fn dimensions(&self, path: String, names: Vec<String>) -> CommandResult<Vec<Option<Dimensions>>> {
        if names.len() > MAX_NAMES {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                format!("ask for at most {MAX_NAMES} names at a time"),
            ));
        }

        let folder = resolve(Some(path), None)?;
        // A name that fails the check is read as no file: `None` in its place, and nothing is opened for it.
        let names: Vec<Option<String>> = names
            .into_iter()
            .map(|name| checked(&folder, &name).is_ok().then_some(name))
            .collect();
        let permit = begin(&self.sizes, SIZE_THREADS)?;
        let cancel = Arc::clone(&self.cancel);

        let sizes = blocking(permit, move || {
            let folder = platform::local_path(&folder)?;

            names
                .iter()
                .map(|name| {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(io::Error::from(io::ErrorKind::Interrupted));
                    }

                    Ok(name.as_deref().and_then(|name| read::dimensions(&folder, name)))
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .await?;

        sizes.map_err(|error| view_error(&error))
    }

    /// The git status of the entries of the folder `path`, bounded by `limits`.
    async fn git(&self, path: String, limits: Limits) -> CommandResult<GitStatus> {
        self.git_with(path, limits, limits.time + GIT_GRACE, gitstatus::status)
            .await
    }

    /// Like [`Self::git`], with the status read by `read` and the request answered after `wait` whether or not `read` has
    /// returned (see the module docs).
    async fn git_with(
        &self,
        path: String,
        limits: Limits,
        wait: Duration,
        read: impl FnOnce(&Path, Limits, &AtomicBool) -> Result<GitStatus, GitError> + Send + 'static,
    ) -> CommandResult<GitStatus> {
        let folder = resolve(Some(path), None)?;
        // A quick answer; the place in the count is taken (and checked again) when the thread is started.
        if self.stuck.is_full() {
            return Err(git_busy());
        }
        let permit = self.statuses.acquire(GIT_THREADS).ok_or_else(git_busy)?;
        let cancel = Arc::clone(&self.cancel);
        let stuck = self.stuck.clone();

        // The wait is on a blocking thread that holds the place until it ends, which is at the deadline at the latest.
        let answer = blocking(permit, move || {
            stuck.run(wait, move || {
                let folder = platform::local_path(&folder).map_err(|error| GitError::Folder(error.kind()))?;

                read(&folder, limits, &cancel)
            })
        })
        .await?;

        match answer {
            Ok(status) => status.map_err(|error| git_error(&error)),
            Err(Unanswered::TimedOut) => Err(git_error(&GitError::TimedOut)),
            Err(Unanswered::Busy) => Err(git_busy()),
            Err(Unanswered::Failed) => Err(CommandError::new(IpcErrorCode::Internal, "the status was interrupted")),
        }
    }

    /// The SHA-256 of the file `name` of the folder `path`, as lowercase hex.
    async fn checksum(&self, path: String, name: String) -> CommandResult<String> {
        let folder = resolve(Some(path), None)?;
        checked(&folder, &name)?;
        let permit = self.hashes.acquire(HASH_THREADS).ok_or_else(busy)?;
        let cancel = Arc::clone(&self.cancel);

        let hash = blocking(permit, move || {
            let folder = platform::local_path(&folder)?;

            read::checksum(&folder, &name, &cancel)
        })
        .await?;

        hash.map_err(|error| view_error(&error))
    }
}

/// A name is one plain name that fits, inside `folder`.
fn checked(folder: &std::path::Path, name: &str) -> CommandResult<()> {
    if !name_fits(name) {
        return Err(names_too_large());
    }

    check_name(folder, name)
}

/// What the page is told of a status that could not be given: the text of the error, which holds no path.
fn git_error(error: &GitError) -> CommandError {
    // `invalidInput` is for a repository that cannot be read here for its size or its time, and for nothing else.
    let code = match error {
        GitError::TooBig | GitError::TimedOut => IpcErrorCode::InvalidInput,
        GitError::Folder(io::ErrorKind::NotFound) => IpcErrorCode::NotFound,
        GitError::Cancelled | GitError::Folder(_) | GitError::Repository => IpcErrorCode::Io,
    };

    CommandError::new(code, error.to_string())
}

fn git_busy() -> CommandError {
    CommandError::new(IpcErrorCode::Busy, "git statuses are being read; try again shortly")
}

fn busy() -> CommandError {
    CommandError::new(IpcErrorCode::Busy, "checksums are being computed; try again shortly")
}

/// The sizes in pixels of the picture files `names` of the folder `path`, read from their headers: one entry for each name,
/// in order, `null` for a name that is not a plain name, is not a regular file (a link is not followed), is a cloud
/// placeholder, cannot be read or is not a PNG, JPEG, GIF, BMP, WebP, HEIC or AVIF picture whose header says its size. At
/// most 512 names (`invalidInput` beyond that), and `busy` when too many batches are being read.
#[tauri::command]
pub async fn columns_dimensions(
    columns: State<'_, Columns>,
    path: String,
    names: Vec<String>,
) -> CommandResult<Vec<Option<Dimensions>>> {
    columns.dimensions(path, names).await
}

/// The SHA-256 of the file `name` of the folder `path` as lowercase hex. `invalidInput` ("the file is too big to checksum
/// here") for a file over 64 MiB, `invalidInput` ("the file is not on this computer") for a cloud placeholder, and
/// `invalidInput` for anything that is not a regular file, `busy` when two checksums are already being computed.
#[tauri::command]
pub async fn columns_checksum(columns: State<'_, Columns>, path: String, name: String) -> CommandResult<String> {
    columns.checksum(path, name).await
}

/// The git status of the direct children of the folder `path`: `repo` says whether the folder is in the work tree of a git
/// repository (`false`: no marks), and `marks` has an entry for each child that is `modified`, `staged`, `untracked`,
/// `ignored` or `conflicted`; a folder has the most severe mark of anything below it (`conflicted`, `modified`, `staged`,
/// `untracked`), and is `ignored` only when it is itself. The repository is read in process and nothing it names (a filter, a
/// hook, `core.fsmonitor`, a credential helper) is ever started, and a repository that points to a network place, or has a
/// pipe or a device where a file is expected, or is of a format that is not read, is `repo: false`. `invalidInput` ("the
/// repository is too big to read here") when a step of it walks over 200,000 entries or it has an index over 64 MiB, `invalidInput` ("the
/// repository took too long to read") when it takes over 5 seconds (the answer comes by then whether or not the read has
/// returned), and `busy` when two statuses are already being read, or the ones being read and the ones stuck add up to two.
#[tauri::command]
pub async fn columns_git(columns: State<'_, Columns>, path: String) -> CommandResult<GitStatus> {
    columns.git(path, Limits::default()).await
}

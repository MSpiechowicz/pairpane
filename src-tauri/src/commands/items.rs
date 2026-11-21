//! Changes to single items: the Trash and its undo, permanent delete, new folders and files, and links.
//!
//! Like `ops_rename`, each is a quick disk change and so runs on a blocking thread that holds a place of a
//! [`ThreadCap`] until it ends, because a change on a hung mount never returns and must not use up the thread pool
//! (see `limit.rs`). What the page sent is checked for syntax first, on the command's thread, and a request that
//! fails the check is refused before a thread is taken. The disk is touched only on the blocking thread, and only
//! after `platform::local_path` has resolved the folder (a link on its path may lead to a network path, which
//! Windows would connect to).
//!
//! The last move to the Trash is remembered here, and the page only gets a token for it: what can be undone never
//! comes from the page. No text sent to the page holds a path; details go to the log.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use pairpane_core::listing::platform;
use pairpane_core::model::{IpcError, IpcErrorCode, LinkKind, TrashOutcome};
use pairpane_core::ops::{self, CAN_UNTRASH, ItemFailure, Trashed, UntrashFailure, name_fits};
use tauri::async_runtime;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::limit::{Permit, ThreadCap};
use super::ops::{check_name, checked_names, failed, names_too_large};
use super::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// Threads the app may have at once for creating, linking and moving to the Trash: a stuck one is stepped around by
/// the other, and two stuck ones refuse the rest as busy.
const QUICK_THREADS: usize = 2;

/// Threads for permanent deletes, which can take long (a big folder) and so have places of their own.
const DELETE_THREADS: usize = 2;

/// What the system calls the place deleted items go to.
const BIN: &str = if cfg!(windows) { "Recycle Bin" } else { "Trash" };

/// What a failed change is called in "You do not have permission to ...".
const TRASH: &str = if cfg!(windows) {
    "move this item to the Recycle Bin"
} else {
    "move this item to the Trash"
};
const DELETE: &str = "delete this item";
const MAKE_FOLDER: &str = "create this folder";
const MAKE_FILE: &str = "create this file";
const MAKE_LINK: &str = "create this link";

/// Shown when a file was made but the system could not open it.
const NOT_OPENED: &str = "The file was created, but it could not be opened.";

/// The caps on the blocking threads and the move to the Trash that can be undone.
#[derive(Default)]
pub struct Items {
    quick: ThreadCap,
    deletes: ThreadCap,
    undo: Mutex<Undo>,
}

/// The last move to the Trash that left something there, with the token the page undoes it by.
#[derive(Default)]
struct Undo {
    /// The token given last; the next one is one more.
    last: u32,
    record: Option<(u32, Trashed)>,
}

impl Items {
    /// Moves the `names` in the folder `path` to the Trash, on a blocking thread. A move that stopped at an item
    /// still succeeds: the outcome says so, and holds the token for the items that did go.
    async fn trash(&self, path: String, names: Vec<String>) -> CommandResult<TrashOutcome> {
        let folder = resolve(Some(path), None)?;
        let names = checked_names(&folder, names)?;
        let permit = begin(&self.quick, QUICK_THREADS)?;

        let moved = in_folder(permit, folder, move |actual| Ok(ops::trash(actual, &names))).await?;
        let (record, result) = moved.map_err(|error| failed(item_error(TRASH, &error)))?;

        let failure = result.err().map(|failure| item_failure(TRASH, &failure));

        Ok(TrashOutcome {
            undo: self.remember(record),
            failure,
        })
    }

    /// Puts back the items of the move `token` stands for, and returns how many came back.
    ///
    /// The record is used up by a full restore and by finding nothing; an item in the way keeps it, so that the
    /// page can try again after the item is moved.
    async fn untrash(&self, token: u32) -> CommandResult<usize> {
        if !CAN_UNTRASH {
            return Err(failed(untrash_error(&UntrashFailure::Unsupported)));
        }

        let permit = begin(&self.quick, QUICK_THREADS)?;
        let record = self.take(token).ok_or_else(nothing_to_undo)?;

        let kept = record.clone();
        let restored = blocking(permit, move || ops::untrash(&record)).await;

        // A thread that panicked leaves nothing to try again with.
        let restored = restored?;
        self.settle(token, kept, &restored);

        restored.map_err(|failure| failed(untrash_error(&failure)))
    }

    /// Deletes the `names` in the folder `path` for good, on a blocking thread of the delete cap.
    async fn delete(&self, path: String, names: Vec<String>) -> CommandResult<()> {
        let folder = resolve(Some(path), None)?;
        let names = checked_names(&folder, names)?;
        let permit = begin(&self.deletes, DELETE_THREADS)?;

        let deleted = in_folder(permit, folder, move |actual| {
            Ok(ops::delete_permanently(actual, &names))
        })
        .await?;

        deleted
            .map_err(|error| failed(item_error(DELETE, &error)))?
            .map_err(|failure| failed(item_failure(DELETE, &failure)))
    }

    /// Creates the folders of `name` (`a/b/c`) in the folder `path` and returns the first part.
    async fn mkdir(&self, path: String, name: String) -> CommandResult<String> {
        let folder = resolve(Some(path), None)?;
        ops::folder_steps(&name).map_err(|error| failed(item_error(MAKE_FOLDER, &error)))?;
        platform::check_folder(&folder.join(&name))
            .map_err(|reason| CommandError::new(IpcErrorCode::InvalidInput, reason))?;
        let permit = begin(&self.quick, QUICK_THREADS)?;

        in_folder(permit, folder, move |actual| ops::create_folders(actual, &name))
            .await?
            .map_err(|error| failed(item_error(MAKE_FOLDER, &error)))
    }

    /// Creates the empty file `name` in the folder `path`, then has `open` open it (with the path as text), unless
    /// it is a program or a script: those are made and left alone. Returns whether the file was opened.
    ///
    /// The file stays when it cannot be opened; the error says so.
    async fn mkfile(
        &self,
        path: String,
        name: String,
        open: impl FnOnce(String) -> Result<(), String> + Send + 'static,
    ) -> CommandResult<bool> {
        let folder = resolve(Some(path), None)?;
        check_new_name(&folder, &name)?;
        let permit = begin(&self.quick, QUICK_THREADS)?;

        let created = in_folder(permit, folder, move |actual| {
            let file = ops::create_file(actual, &name)?;
            if ops::is_runnable(&name) {
                return Ok(Ok(false));
            }

            // A path that is not text cannot be handed to the opener without changing it, and a changed path could
            // open another file: the new file is left alone.
            let Some(text) = file.to_str() else {
                return Ok(Err("the path of the new file is not valid text".to_owned()));
            };

            Ok(open(text.to_owned()).map(|()| true))
        })
        .await?;

        match created.map_err(|error| failed(item_error(MAKE_FILE, &error)))? {
            Ok(opened) => Ok(opened),
            Err(detail) => {
                tracing::warn!(detail, "cannot open a new file");

                Err(CommandError::new(IpcErrorCode::Process, NOT_OPENED))
            }
        }
    }

    /// Makes a link named `link_name` in the folder `to`, leading to the item `name` of the folder `path`.
    async fn link(
        &self,
        kind: LinkKind,
        path: String,
        name: String,
        to: String,
        link_name: String,
    ) -> CommandResult<()> {
        let target = resolve(Some(path.clone()), Some(&name))?;
        let source_folder = resolve(Some(path), None)?;
        let folder = resolve(Some(to), None)?;
        if !name_fits(&name) {
            return Err(names_too_large());
        }
        check_new_name(&folder, &link_name)?;
        let permit = begin(&self.quick, QUICK_THREADS)?;

        let linked = blocking(permit, move || {
            let actual = platform::local_path(&folder)?;
            // The target's own folder is checked too, though a symbolic link never reads it.
            let source = platform::local_path(&source_folder)?;

            // A symbolic link leads where the pane showed the item; a hard link is made to the file itself.
            let target = match kind {
                LinkKind::Symbolic => target,
                LinkKind::Hard => source.join(&name),
            };

            ops::make_link(kind, &target, &actual, &link_name)
        })
        .await?;

        linked.map_err(|error| failed(item_error(MAKE_LINK, &error)))
    }

    /// Remembers a move to the Trash for [`Items::untrash`] and returns its token; `None` when nothing reached the
    /// Trash (an earlier record stays) or when the system cannot put items back.
    fn remember(&self, record: Trashed) -> Option<u32> {
        if !CAN_UNTRASH || record.names.is_empty() {
            return None;
        }

        let mut undo = lock(&self.undo);
        undo.last = undo.last.wrapping_add(1);
        let token = undo.last;
        undo.record = Some((token, record));

        Some(token)
    }

    /// Takes the record out when `token` is its token. A second undo with it finds nothing.
    fn take(&self, token: u32) -> Option<Trashed> {
        let mut undo = lock(&self.undo);

        match undo.record.take() {
            Some((current, record)) if current == token => Some(record),
            other => {
                undo.record = other;

                None
            }
        }
    }

    /// Gives the record of an undo that did not finish back, unless a newer move replaced it meanwhile: when an item
    /// was in the way, the folder changed (it may be put back), or the Trash could not be read, the page may try again.
    fn settle(&self, token: u32, record: Trashed, result: &Result<usize, UntrashFailure>) {
        let again = matches!(
            result,
            Err(UntrashFailure::InTheWay { .. } | UntrashFailure::FolderChanged | UntrashFailure::Other)
        );

        let mut undo = lock(&self.undo);
        if again && undo.record.is_none() {
            undo.record = Some((token, record));
        }
    }
}

/// Moves the `names` in the folder `path` to the Trash (the Recycle Bin on Windows); a move that stopped at an item
/// still returns, with the token for the items that went and the failure.
#[tauri::command]
pub async fn ops_trash(items: State<'_, Items>, path: String, names: Vec<String>) -> CommandResult<TrashOutcome> {
    items.trash(path, names).await
}

/// Puts back the items of the move to the Trash that `token` stands for and returns how many came back.
#[tauri::command]
pub async fn ops_untrash(items: State<'_, Items>, token: u32) -> CommandResult<usize> {
    items.untrash(token).await
}

/// Deletes the `names` in the folder `path` for good. A folder goes with what is in it; a link goes alone.
#[tauri::command]
pub async fn ops_delete(items: State<'_, Items>, path: String, names: Vec<String>) -> CommandResult<()> {
    items.delete(path, names).await
}

/// Creates the folder `name` in the folder `path`; `a/b/c` creates the three one inside the other. Returns the first
/// part, the one the pane shows.
#[tauri::command]
pub async fn ops_mkdir(items: State<'_, Items>, path: String, name: String) -> CommandResult<String> {
    items.mkdir(path, name).await
}

/// Creates the empty file `name` in the folder `path` and opens it with the system's default program; `false` when
/// it is a program or a script, which is created but not opened.
#[tauri::command]
pub async fn ops_mkfile(app: AppHandle, items: State<'_, Items>, path: String, name: String) -> CommandResult<bool> {
    items
        .mkfile(path, name, move |file| {
            app.opener()
                .open_path(file, None::<&str>)
                .map_err(|error| error.to_string())
        })
        .await
}

/// Makes the link `link_name` in the folder `to`, leading to the item `name` of the folder `path`.
#[tauri::command]
pub async fn ops_link(
    items: State<'_, Items>,
    kind: LinkKind,
    path: String,
    name: String,
    to: String,
    link_name: String,
) -> CommandResult<()> {
    items.link(kind, path, name, to, link_name).await
}

/// Takes one place of `cap`, or refuses as busy.
fn begin(cap: &ThreadCap, max: usize) -> CommandResult<Permit> {
    cap.acquire(max)
        .ok_or_else(|| CommandError::new(IpcErrorCode::Busy, "a folder is not responding; try again shortly"))
}

/// Runs `work` on a blocking thread that holds `permit` until it ends, however it ends.
async fn blocking<T: Send + 'static>(permit: Permit, work: impl FnOnce() -> T + Send + 'static) -> CommandResult<T> {
    let done = async_runtime::spawn_blocking(move || {
        let _permit = permit;

        work()
    })
    .await;

    done.map_err(|_| CommandError::new(IpcErrorCode::Internal, "the change was interrupted"))
}

/// Runs `work` in the folder `folder` on a blocking thread (see [`blocking`]), given the path to use for it: first of
/// all the folder is resolved by `platform::local_path`, and a path that is not local fails the work.
async fn in_folder<T: Send + 'static>(
    permit: Permit,
    folder: PathBuf,
    work: impl FnOnce(&Path) -> io::Result<T> + Send + 'static,
) -> CommandResult<io::Result<T>> {
    blocking(permit, move || work(&platform::local_path(&folder)?)).await
}

/// A name for a new item: plain, short enough and, inside `folder`, passing the folder rules.
fn check_new_name(folder: &Path, name: &str) -> CommandResult<()> {
    if !name_fits(name) {
        return Err(names_too_large());
    }

    check_name(folder, name)
}

/// What the page is told when the change `action` ("delete this item") fails. Never a path: our own texts (they hold
/// none) and fixed wording per kind of failure, and for a system error its text, which only the system's own words.
fn item_error(action: &str, error: &io::Error) -> IpcError {
    let kind = error.kind();
    let own_text = error.get_ref().is_some();

    let (code, message) = match kind {
        io::ErrorKind::AlreadyExists => (
            IpcErrorCode::InvalidInput,
            "An item with that name already exists.".to_owned(),
        ),
        io::ErrorKind::NotFound => (
            IpcErrorCode::NotFound,
            "This item or its folder no longer exists.".to_owned(),
        ),
        io::ErrorKind::PermissionDenied => (IpcErrorCode::Io, format!("You do not have permission to {action}.")),
        io::ErrorKind::InvalidInput if own_text => (IpcErrorCode::InvalidInput, error.to_string()),
        io::ErrorKind::CrossesDevices | io::ErrorKind::Other if own_text => (IpcErrorCode::Io, error.to_string()),
        _ if error.raw_os_error().is_some() => (IpcErrorCode::Io, format!("Cannot {action}: {error}")),
        _ => (IpcErrorCode::Io, format!("Cannot {action}.")),
    };

    IpcError::new(code, message)
}

/// [`item_error`] for the item a multi-item change stopped at, led by the item's name (a plain name, no path).
fn item_failure(action: &str, failure: &ItemFailure) -> IpcError {
    let error = item_error(action, &failure.error);

    IpcError::new(error.code, format!("{}: {}", failure.name, error.message))
}

/// What the page is told when an undo cannot put the items back.
fn untrash_error(failure: &UntrashFailure) -> IpcError {
    match failure {
        UntrashFailure::Unsupported => IpcError::new(
            IpcErrorCode::Unsupported,
            "Undo is not available on this system. Restore the items from the Trash by hand.",
        ),
        UntrashFailure::NothingFound => IpcError::new(
            IpcErrorCode::NotFound,
            format!("Nothing to restore: the items are no longer in the {BIN}."),
        ),
        UntrashFailure::InTheWay { name } => IpcError::new(
            IpcErrorCode::InvalidInput,
            format!("Cannot restore: an item named {name} already exists where it was."),
        ),
        UntrashFailure::FolderChanged => IpcError::new(
            IpcErrorCode::InvalidInput,
            "Cannot restore: the folder the items came from has changed.",
        ),
        UntrashFailure::Other => IpcError::new(IpcErrorCode::Io, format!("Cannot restore from the {BIN}.")),
    }
}

fn nothing_to_undo() -> CommandError {
    CommandError::new(IpcErrorCode::NotFound, "There is nothing to undo")
}

fn lock(undo: &Mutex<Undo>) -> MutexGuard<'_, Undo> {
    undo.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;

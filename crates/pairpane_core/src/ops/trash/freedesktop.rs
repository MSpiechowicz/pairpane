//! The freedesktop Trash, on Linux and the other Unix systems that are not macOS, done here and not by the `trash`
//! crate.
//!
//! The crate looks the Trash folders up by path again and again, follows links, copies an item across disks when a
//! rename is refused, and panics on a mount whose name is not text. Here every folder of the Trash is opened once as a
//! handle ([`handle`]) and judged by what was opened, an item is renamed into it ([`place`]) and nothing is ever
//! copied, and the undo ([`restore`]) opens only the Trash folder the item went to.

mod folders;
mod handle;
mod info;
mod place;
mod restore;
mod top;

#[cfg(test)]
mod testing;

/// Removes what a test left in the home Trash (see [`testing::purge_below`]).
#[cfg(test)]
pub(in crate::ops) use testing::purge_below;

/// The folder of the home Trash the way a move finds it: the one the tests of the real Trash must clean.
#[cfg(test)]
pub(in crate::ops) fn home_trash() -> Option<PathBuf> {
    Context::system().data_home.map(|data| data.join("Trash"))
}

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use super::{Trashed, UntrashFailure};
use crate::listing::platform;
use crate::ops::items::ItemFailure;
use crate::ops::report::refusal;
use folders::{Context, Kind};
use handle::Birth;
use place::Session;

/// Where one item went in the Trash, as much as the undo needs to take it from there again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// The name the item had.
    name: String,
    kind: Kind,
    /// The folder the Trash is found from: the data folder of the home Trash, else the top folder of the drive.
    base: PathBuf,
    /// The path its record gives.
    recorded: PathBuf,
    /// The name of the item in `files`; its record is `info/<stem>.trashinfo`.
    stem: OsString,
    /// The folder the item was in, by identity: a folder that was replaced is not the one to put the item back to.
    /// A device and an inode number can come back for another folder once the first is gone, so its owner and (where
    /// the file system tells it) the time it was made are kept too.
    folder_device: u64,
    folder_inode: u64,
    folder_owner: u32,
    folder_birth: Option<Birth>,
}

/// Moves the items `names` of `folder` to the Trash one by one, stopping at the first that fails (see
/// [`crate::ops::trash`]).
pub(super) fn trash(folder: &Path, names: &[String]) -> (Trashed, Result<(), ItemFailure>) {
    trash_in(&Context::system(), folder, names)
}

/// [`trash`] for the system facts in `context`.
fn trash_in(context: &Context, folder: &Path, names: &[String]) -> (Trashed, Result<(), ItemFailure>) {
    let mut record = Trashed {
        folder: folder.to_path_buf(),
        ..Trashed::default()
    };
    let mut session = None;
    let mut result = Ok(());

    for name in names {
        match trash_one(context, &mut session, folder, name) {
            Ok(placed) => {
                record.names.push(name.clone());
                record.placed.push(placed);
            }
            Err(error) => {
                result = Err(ItemFailure {
                    name: name.clone(),
                    error,
                });

                break;
            }
        }
    }

    (record, result)
}

/// Moves one item; the folder is opened with the first, and its Trash with the first that is there.
fn trash_one(context: &Context, session: &mut Option<Session>, folder: &Path, name: &str) -> io::Result<Placed> {
    if !platform::is_plain_name(name) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the name is not a plain file name",
        ));
    }

    let open = match session {
        Some(open) => open,
        None => session.insert(Session::open(folder).inspect_err(|error| {
            tracing::warn!(?error, "cannot open the folder to move items from");
        })?),
    };

    open.trash(context, name).inspect_err(|error| {
        tracing::warn!(?error, "cannot move an item to the Trash");
    })
}

/// Moves the item `name` of the folder held open as `held` (which `folder` names) to the Trash, renamed from that handle:
/// a folder on the way swapped for a link cannot send another item there.
pub(super) fn trash_held(folder: &Path, held: &std::fs::File, name: &str) -> io::Result<()> {
    let mut session = Some(Session::of_held(folder, std::os::fd::OwnedFd::from(held.try_clone()?))?);

    trash_one(&Context::system(), &mut session, folder, name).map(|_| ())
}

/// Puts the items of `record` back where they were (see [`crate::ops::untrash`]).
pub(super) fn untrash(record: &Trashed) -> Result<usize, UntrashFailure> {
    restore::untrash(&Context::system(), record)
}

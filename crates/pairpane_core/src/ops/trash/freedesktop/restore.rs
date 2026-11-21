//! Putting the items of a move back, without looking for them: the record of the move says which Trash folder each
//! went to and under what name, so only those are opened, with the rules the move used.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;

use rustix::fs::{AtFlags, Mode, OFlags, openat, unlinkat};
use rustix::io::Errno;

use super::folders::{Context, Folders, Reopened, reopen};
use super::handle::{Birth, Facts, birth_of, exists_in, open_folder, rename_noreplace};
use super::info::parse_path;
use super::{Placed, Trashed, UntrashFailure};

/// A record is a few lines; one that is longer is not one.
const LONGEST_RECORD: u64 = 16 * 1024;

/// Puts back the items of `record` and returns how many. Stops at the first that cannot go; the ones before it stay
/// back. An item that is not in the Trash any more is passed over.
pub(super) fn untrash(context: &Context, record: &Trashed) -> Result<usize, UntrashFailure> {
    if record.placed.is_empty() {
        return Err(UntrashFailure::NothingFound);
    }

    // The folder the items came from is the folder they left, or nothing is put back into it.
    let folder = match open_folder(&record.folder) {
        Ok(fd) => fd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(UntrashFailure::FolderChanged),
        Err(error) => {
            tracing::warn!(?error, "cannot open the folder the items came from");

            return Err(UntrashFailure::Other);
        }
    };
    let here = Facts::of_handle(&folder).map_err(|error| {
        tracing::warn!(?error, "cannot look at the folder the items came from");

        UntrashFailure::Other
    })?;

    let birth = birth_of(&folder);
    let mut back = 0;

    for placed in &record.placed {
        if !is_the_folder(placed, &here, birth) {
            return Err(UntrashFailure::FolderChanged);
        }

        if restore_one(context, placed, &folder, &here)? {
            back += 1;
        }
    }

    if back == 0 {
        return Err(UntrashFailure::NothingFound);
    }

    Ok(back)
}

/// Whether the folder that is open (`here`, made at `birth` where that is known) is the folder `placed` came from. The
/// device and inode number alone can be those of another folder once the first is gone and its number is used again,
/// so the owner and the time it was made must be the same too; a time that either side does not know is not compared.
fn is_the_folder(placed: &Placed, here: &Facts, birth: Option<Birth>) -> bool {
    let same_birth = match (placed.folder_birth, birth) {
        (Some(was), Some(now)) => was == now,
        _ => true,
    };

    placed.folder_device == here.device
        && placed.folder_inode == here.inode
        && placed.folder_owner == here.owner
        && same_birth
}

/// Puts one item back. `false` when it is not in the Trash any more.
fn restore_one(context: &Context, placed: &Placed, folder: &OwnedFd, here: &Facts) -> Result<bool, UntrashFailure> {
    let folders = match reopen(context.uid, placed.kind, &placed.base, here.device) {
        Ok(Reopened::Open(folders)) => folders,
        Ok(Reopened::Gone) => return Ok(false),
        Err(error) => {
            tracing::warn!(?error, "cannot open the Trash again");

            return Err(UntrashFailure::Other);
        }
    };

    let mut record_name: OsString = placed.stem.clone();
    record_name.push(".trashinfo");

    // The item must be there, and its record must be the one that was written for it.
    let in_the_trash =
        exists_in(&folders.files, &placed.stem).map_err(failed("cannot look for an item in the Trash"))?;
    if !in_the_trash {
        return Ok(false);
    }
    check_record(&folders, &record_name, placed)?;

    let name = OsString::from(&placed.name);

    match rename_noreplace(&folders.files, &placed.stem, folder, &name) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(UntrashFailure::InTheWay {
                name: placed.name.clone(),
            });
        }
        // The folder that was open is gone since: the item is still in the Trash, and nothing is put back into a folder
        // that is not there. Only an item that is not in the Trash any more is passed over.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let still_there =
                exists_in(&folders.files, &placed.stem).map_err(failed("cannot look for an item in the Trash"))?;

            return if still_there {
                Err(UntrashFailure::FolderChanged)
            } else {
                Ok(false)
            };
        }
        Err(error) => {
            tracing::warn!(?error, "cannot put an item back");

            return Err(UntrashFailure::Other);
        }
    }

    // The item is back; a record that cannot be removed is left, and does no harm.
    if let Err(errno) = unlinkat(&folders.info, &record_name, AtFlags::empty()) {
        tracing::warn!(%errno, "cannot remove the record of an item that was put back");
    }

    Ok(true)
}

/// Reads the record `record_name` of `folders` (never through a link, a regular file, a few lines) and requires that
/// it gives the path that the move wrote.
fn check_record(folders: &Folders, record_name: &OsString, placed: &Placed) -> Result<(), UntrashFailure> {
    // Not blocking: opening a named pipe for reading would wait for a writer that may never come. It is looked at
    // once open, and only a regular file is read.
    let fd = match openat(
        &folders.info,
        record_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(errno) => {
            if errno != Errno::NOENT {
                tracing::warn!(%errno, "cannot open the record of an item");
            }

            return Err(UntrashFailure::Other);
        }
    };

    let mut file = File::from(fd);

    // Anything but a regular file (a pipe, a device) is not read at all.
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        tracing::warn!("the record of an item is not a regular file");

        return Err(UntrashFailure::Other);
    }

    let mut bytes = Vec::new();
    let read = file.by_ref().take(LONGEST_RECORD + 1).read_to_end(&mut bytes);

    let short = u64::try_from(bytes.len()).is_ok_and(|length| length <= LONGEST_RECORD);
    let text = String::from_utf8(bytes).ok();
    let path = text.as_deref().and_then(parse_path);

    if read.is_ok() && short && path.as_deref() == Some(placed.recorded.as_path()) {
        return Ok(());
    }

    tracing::warn!("the record of an item is not the one that was written");

    Err(UntrashFailure::Other)
}

/// The mapping of an error to the failure that is told, logging it with `what`.
fn failed(what: &'static str) -> impl Fn(io::Error) -> UntrashFailure {
    move |error| {
        tracing::warn!(?error, "{what}");

        UntrashFailure::Other
    }
}

#[cfg(test)]
mod tests;

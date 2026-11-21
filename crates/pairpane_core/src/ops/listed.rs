//! Opening a file a checksum file lists: `open_listed` finds it below the folder of the checksum file, without ever
//! following a link, and hands back the open file.
//!
//! Unix: the way is walked with handles, one folder at a time (`openat` from the folder before, no-follow, folders only),
//! and each entry is looked at (`fstatat`, no-follow) and then opened and compared with what was seen, so a folder
//! swapped for a link, or an entry swapped for another, between two steps cannot send the open anywhere else: every
//! step is relative to a handle that cannot move. Windows: the way is checked name by name by path, and the file
//! opened without following a link; a folder swapped in between the look and the open is the same race the other
//! Windows jobs document.

use std::fs::{File, Metadata};
use std::io;
use std::path::Path;

use crate::listing::platform;

use super::report::refusal;

/// A file that is gone, told in the words of a checksum check.
pub(super) fn missing(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        return refusal(io::ErrorKind::NotFound, "the file is missing");
    }

    error
}

fn unreadable_line() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "this line cannot be read")
}

fn through_a_link() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the path goes through a link")
}

fn through_a_file() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the path goes through a file")
}

fn not_regular() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "it is not a regular file")
}

/// The plain names of `listed` (a path joined by `/`); a line that holds any other name cannot be read.
fn names_of(listed: &str) -> io::Result<Vec<&str>> {
    let names: Vec<&str> = listed.split('/').collect();

    if names.iter().all(|name| platform::is_plain_name(name)) {
        Ok(names)
    } else {
        Err(unreadable_line())
    }
}

/// Opens the regular file `listed` below the folder `base` for reading, with its metadata. A link on the way or at the
/// file, a folder where the file should be and a file where a folder should be are refused with a fixed text.
#[cfg(unix)]
pub(super) fn open_listed(base: &Path, listed: &str) -> io::Result<(File, Metadata)> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat, openat, statat};

    fn look(dir: &File, name: &str) -> io::Result<Stat> {
        statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(io::Error::from)
            .map_err(missing)
    }

    /// Opens the entry `name` of `dir` as what was seen there, and checks the open is that very entry.
    fn enter(dir: &File, name: &str, seen: &Stat, flags: OFlags) -> io::Result<File> {
        let flags = flags | OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let file = File::from(openat(dir, name, flags, Mode::empty()).map_err(io::Error::from)?);
        let now = rustix::fs::fstat(&file).map_err(io::Error::from)?;

        if (now.st_dev, now.st_ino) != (seen.st_dev, seen.st_ino) {
            return Err(refusal(io::ErrorKind::Other, "it changed while it was being checked"));
        }

        Ok(file)
    }

    let names = names_of(listed)?;
    let Some((last, folders)) = names.split_last() else {
        return Err(unreadable_line());
    };

    let mut dir = super::open::root_folder(base).map_err(missing)?;
    for name in folders {
        let seen = look(&dir, name)?;

        dir = match FileType::from_raw_mode(seen.st_mode) {
            FileType::Directory => enter(&dir, name, &seen, OFlags::DIRECTORY)?,
            FileType::Symlink => return Err(through_a_link()),
            _ => return Err(through_a_file()),
        };
    }

    let seen = look(&dir, last)?;
    match FileType::from_raw_mode(seen.st_mode) {
        FileType::RegularFile => {}
        FileType::Symlink => return Err(through_a_link()),
        _ => return Err(not_regular()),
    }

    // Never waits on a pipe that slips in, never takes a terminal.
    let file = enter(&dir, last, &seen, OFlags::NONBLOCK | OFlags::NOCTTY)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(not_regular());
    }

    Ok((file, meta))
}

#[cfg(windows)]
pub(super) fn open_listed(base: &Path, listed: &str) -> io::Result<(File, Metadata)> {
    use std::fs;

    let names = names_of(listed)?;
    let mut path = base.to_owned();

    for (index, name) in names.iter().enumerate() {
        path.push(name);

        let meta = fs::symlink_metadata(&path).map_err(missing)?;
        if meta.file_type().is_symlink() {
            return Err(through_a_link());
        }

        if index + 1 < names.len() {
            if !meta.is_dir() {
                return Err(through_a_file());
            }

            continue;
        }

        if !meta.is_file() {
            return Err(not_regular());
        }

        let file = super::open::source(&path, &meta).map_err(missing)?;
        let meta = file.metadata()?;

        return Ok((file, meta));
    }

    Err(unreadable_line())
}

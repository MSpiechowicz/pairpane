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

/// The entry `name` of `dir`, looked at without following a link.
#[cfg(unix)]
fn look(dir: &File, name: &str) -> io::Result<rustix::fs::Stat> {
    rustix::fs::statat(dir, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(io::Error::from)
        .map_err(missing)
}

/// Opens the entry `name` of `dir` as what was seen there, and checks the open is that very entry.
#[cfg(unix)]
fn enter(dir: &File, name: &str, seen: &rustix::fs::Stat, flags: rustix::fs::OFlags) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags, openat};

    let flags = flags | OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let file = File::from(openat(dir, name, flags, Mode::empty()).map_err(io::Error::from)?);
    let now = rustix::fs::fstat(&file).map_err(io::Error::from)?;

    if (now.st_dev, now.st_ino) != (seen.st_dev, seen.st_ino) {
        return Err(refusal(io::ErrorKind::Other, "it changed while it was being checked"));
    }

    Ok(file)
}

/// Opens the folder `names` lead to below the folder `base`, one folder at a time from the handle of the one before.
#[cfg(unix)]
fn open_folders(base: &Path, names: &[&str]) -> io::Result<File> {
    use rustix::fs::{FileType, OFlags};

    let mut dir = super::open::root_folder(base).map_err(missing)?;

    for name in names {
        let seen = look(&dir, name)?;

        dir = match FileType::from_raw_mode(seen.st_mode) {
            FileType::Directory => enter(&dir, name, &seen, OFlags::DIRECTORY)?,
            FileType::Symlink => return Err(through_a_link()),
            _ => return Err(through_a_file()),
        };
    }

    Ok(dir)
}

/// Opens the regular file `listed` below the folder `base` for reading, with its metadata. A link on the way or at the
/// file, a folder where the file should be and a file where a folder should be are refused with a fixed text.
#[cfg(unix)]
pub(super) fn open_listed(base: &Path, listed: &str) -> io::Result<(File, Metadata)> {
    use rustix::fs::{FileType, OFlags};

    let names = names_of(listed)?;
    let Some((last, folders)) = names.split_last() else {
        return Err(unreadable_line());
    };

    let dir = open_folders(base, folders)?;

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

/// Checks that the folder `dir` (names joined with `/`, `""` the folder `base` itself) below `base` can be reached one
/// name at a time with no link and no file on the way. Fails with a fixed text when it cannot.
pub(super) fn check_folder_below(base: &Path, dir: &str) -> io::Result<()> {
    resolve_folder_below(base, dir).map(drop)
}

/// Checks that the folder `dir` below `base` (see [`check_folder_below`]) is, right now, the folder `target`, by identity
/// and not by name: it refuses a link or a file on the way, and a `target` that is a different folder from the one `dir`
/// reaches by names (a link in the path of `target`, say). It does not know which folder the search found its items in:
/// a real folder put in the place of that one after the search passes, as long as `dir` and `target` agree now. Where the
/// check cannot be made the answer is no.
pub(super) fn check_same_folder_below(base: &Path, dir: &str, target: &Path) -> io::Result<()> {
    let folder = resolve_folder_below(base, dir)?;

    if same_folder(&folder, target)? {
        Ok(())
    } else {
        Err(refusal(io::ErrorKind::Other, "it is not the folder that was searched"))
    }
}

/// What a folder below `base` is reached as: its handle on Unix, whose steps were relative to handles that cannot move,
/// its path on Windows, which was checked name by name.
#[cfg(unix)]
type Folder = File;
#[cfg(windows)]
type Folder = std::path::PathBuf;

fn folder_names(dir: &str) -> io::Result<Vec<&str>> {
    if dir.is_empty() { Ok(Vec::new()) } else { names_of(dir) }
}

#[cfg(unix)]
fn resolve_folder_below(base: &Path, dir: &str) -> io::Result<Folder> {
    open_folders(base, &folder_names(dir)?)
}

#[cfg(windows)]
fn resolve_folder_below(base: &Path, dir: &str) -> io::Result<Folder> {
    use std::fs;

    let mut path = base.to_owned();

    for name in folder_names(dir)? {
        path.push(name);

        let meta = fs::symlink_metadata(&path).map_err(missing)?;
        if meta.file_type().is_symlink() {
            return Err(through_a_link());
        }
        if !meta.is_dir() {
            return Err(through_a_file());
        }
    }

    Ok(path)
}

#[cfg(unix)]
fn same_folder(folder: &Folder, target: &Path) -> io::Result<bool> {
    // Through std's metadata, whose device number is a u64 on every Unix (a raw `stat` has an i32 on macOS).
    let now = folder.metadata()?;
    let there = std::fs::metadata(target)?;

    Ok(platform::identity(&now).is_some() && platform::identity(&there) == platform::identity(&now))
}

#[cfg(windows)]
fn same_folder(folder: &Folder, target: &Path) -> io::Result<bool> {
    same_file::is_same_file(folder, target)
}

//! Looking at a file of a repository before gix opens it. gix opens what a repository names with a plain `open`, which on
//! a named pipe waits for a writer for ever, and reads it up to its end, which on `/dev/zero` (a link to it, or the device
//! made as a file) never comes. So each file gix is about to read is first looked at with `lstat` (nothing is opened, no
//! link is followed): it has to be a regular file, and not over its cap. The small files this module reads itself are
//! opened without blocking and without following a link, and what was opened is looked at again (`fstat`), so that a file
//! swapped in between the two looks changes nothing.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// What the look at a file found.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Seen<T> {
    /// There is nothing at the path (or a folder on its way is not there).
    Missing,
    /// A regular file within the cap; for a read, its bytes.
    Plain(T),
    /// Anything else: a link, a pipe, a device, a socket, a folder, a file that cannot be looked at.
    Odd,
    /// A regular file over the cap.
    Large,
}

impl<T> Seen<T> {
    pub(super) fn is_fine(&self) -> bool {
        matches!(self, Self::Missing | Self::Plain(_))
    }
}

fn judge(metadata: &Metadata, cap: u64) -> Seen<()> {
    if !metadata.file_type().is_file() {
        Seen::Odd
    } else if metadata.len() > cap {
        Seen::Large
    } else {
        Seen::Plain(())
    }
}

fn missing(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory)
}

/// Looks at `path` with `lstat`: a regular file of at most `cap` bytes, or not there, is fine.
pub(super) fn seen(path: &Path, cap: u64) -> Seen<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => judge(&metadata, cap),
        Err(error) if missing(&error) => Seen::Missing,
        Err(_) => Seen::Odd,
    }
}

/// The path of the file `relative` (`/` separated) below `root`, once every folder on the way is known to be a folder and
/// not a link: a link there would lead the open of the file somewhere the look at each name did not. `Err` says why not.
fn below(root: &Path, relative: &str) -> Result<PathBuf, Seen<()>> {
    let mut path = root.to_path_buf();
    let mut names = relative.split('/').peekable();

    while let Some(name) = names.next() {
        path.push(name);

        if names.peek().is_none() {
            break;
        }

        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Err(Seen::Odd),
            Err(error) if missing(&error) => return Err(Seen::Missing),
            Err(_) => return Err(Seen::Odd),
        }
    }

    Ok(path)
}

/// Like [`seen`] for the file `relative` below `root` (see [`below`]).
pub(super) fn seen_below(root: &Path, relative: &str, cap: u64) -> Seen<()> {
    match below(root, relative) {
        Ok(path) => seen(&path, cap),
        Err(why) => why,
    }
}

/// Like [`read`] for the file `relative` below `root` (see [`below`]).
pub(super) fn read_below(root: &Path, relative: &str, cap: u64) -> Seen<Vec<u8>> {
    match below(root, relative) {
        Ok(path) => read(&path, cap),
        Err(Seen::Missing) => Seen::Missing,
        Err(_) => Seen::Odd,
    }
}

/// The most a rules file of the work tree (`.gitignore`, `.gitattributes`) may hold to be read.
const RULES_BYTES: u64 = 8 * 1024 * 1024;

/// Whether the rules file `path` of the work tree must not be handed to gix, which opens it by itself where it finds one
/// (and does not follow a link): a named pipe, a device or a socket, which an open waits on or a read never ends, and a
/// regular file too big to be one. Not there, a link and a folder are not a risk: they are not opened as a file.
pub(super) fn risky_rules_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let kind = metadata.file_type();

    if kind.is_file() {
        metadata.len() > RULES_BYTES
    } else {
        !(kind.is_symlink() || kind.is_dir())
    }
}

/// Reads the small regular file `path` (at most `cap` bytes): opened without waiting for a writer and without following a
/// link, then checked as the file that was opened.
pub(super) fn read(path: &Path, cap: u64) -> Seen<Vec<u8>> {
    let file = match open(path) {
        Ok(file) => file,
        Err(error) if missing(&error) => return Seen::Missing,
        Err(_) => return Seen::Odd,
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return Seen::Odd,
    };
    match judge(&metadata, cap) {
        Seen::Plain(()) => {}
        Seen::Large => return Seen::Large,
        Seen::Odd | Seen::Missing => return Seen::Odd,
    }

    let mut bytes = Vec::new();
    match file.take(cap + 1).read_to_end(&mut bytes) {
        Ok(_) if bytes.len() as u64 <= cap => Seen::Plain(bytes),
        Ok(_) => Seen::Large,
        Err(_) => Seen::Odd,
    }
}

#[cfg(unix)]
fn open(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    use rustix::fs::OFlags;

    let flags = OFlags::NONBLOCK | OFlags::NOFOLLOW;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(i32::try_from(flags.bits()).unwrap_or(0))
        .open(path)
}

#[cfg(not(unix))]
fn open(path: &Path) -> io::Result<File> {
    // A link is looked at first: Windows opens what it leads to, and a pipe or device has no name in a folder there.
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }

    File::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regular_file_within_the_cap_is_plain_and_one_over_it_is_large() {
        let folder = tempfile::tempdir().expect("a folder");
        fs::write(folder.path().join("small"), b"12345").expect("a file");

        assert_eq!(seen(&folder.path().join("small"), 5), Seen::Plain(()));
        assert_eq!(seen(&folder.path().join("small"), 4), Seen::Large);
        assert_eq!(seen(&folder.path().join("none"), 4), Seen::Missing);
        assert_eq!(read(&folder.path().join("small"), 5), Seen::Plain(b"12345".to_vec()));
        assert_eq!(read(&folder.path().join("small"), 4), Seen::Large);
        assert_eq!(read(&folder.path().join("none"), 4), Seen::Missing);
    }

    #[test]
    fn a_folder_is_odd_and_so_is_a_file_below_a_folder_that_is_a_link() {
        let folder = tempfile::tempdir().expect("a folder");
        fs::create_dir(folder.path().join("sub")).expect("a folder");
        fs::write(folder.path().join("sub/file"), b"x").expect("a file");

        assert_eq!(seen(&folder.path().join("sub"), 10), Seen::Odd);
        assert_eq!(seen_below(folder.path(), "sub/file", 10), Seen::Plain(()));
        assert_eq!(seen_below(folder.path(), "sub/none", 10), Seen::Missing);
        assert_eq!(seen_below(folder.path(), "none/file", 10), Seen::Missing);
        assert_eq!(seen_below(folder.path(), "sub/file/deeper", 10), Seen::Odd);
    }

    #[cfg(unix)]
    #[test]
    fn pipes_devices_and_links_are_odd_and_nothing_waits_for_them() {
        let folder = tempfile::tempdir().expect("a folder");
        crate::view::testing::fifo(folder.path(), "pipe");
        std::os::unix::fs::symlink("/dev/zero", folder.path().join("zero")).expect("a link");
        fs::create_dir(folder.path().join("real")).expect("a folder");
        std::os::unix::fs::symlink(folder.path().join("real"), folder.path().join("via")).expect("a link");
        fs::write(folder.path().join("real/file"), b"x").expect("a file");

        for name in ["pipe", "zero"] {
            assert_eq!(seen(&folder.path().join(name), 10), Seen::Odd, "{name}");
            assert_eq!(read(&folder.path().join(name), 10), Seen::Odd, "{name}");
        }
        assert_eq!(read(Path::new("/dev/zero"), 10), Seen::Odd);
        assert_eq!(seen_below(folder.path(), "via/file", 10), Seen::Odd);
    }
}

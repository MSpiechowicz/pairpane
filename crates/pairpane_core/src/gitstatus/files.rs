//! Looking at a file of a repository before gix opens it. gix opens what a repository names with a plain `open`, which on
//! a named pipe waits for a writer for ever, and reads it up to its end, which on `/dev/zero` (a link to it, or the device
//! made as a file) never comes. So each file gix is about to read is first looked at with `lstat` (nothing is opened, no
//! link is followed): it has to be a regular file, and not over its cap. The small files this module reads itself are
//! opened without blocking and without following a link, and what was opened is looked at again (`fstat`), so that a file
//! swapped in between the two looks changes nothing. A few files that gix reads through a link (`HEAD`, `info/exclude`,
//! `info/attributes`) are allowed to be one: the link is resolved hop by hop to a local place (see [`super::links`]), and
//! what it leads to has to be a regular file within the cap.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::links;

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

/// Looks at the folder `path` with `lstat`: a real folder is plain, one that is not there (or is no folder, or has a file on its
/// way) is missing, and a link or a reparse point (a junction, a mount point) is odd: gix would go through it to a place that
/// was not looked at, and the files below it are not the ones that were.
pub(super) fn folder(path: &Path) -> Seen<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if missing(&error) => return Seen::Missing,
        Err(_) => return Seen::Odd,
    };

    if is_link(&metadata) {
        Seen::Odd
    } else if metadata.file_type().is_dir() {
        Seen::Plain(())
    } else {
        Seen::Missing
    }
}

/// A symbolic link, or on Windows any reparse point that names another place (a junction, a mount point).
fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink() || is_reparse_point(metadata)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &Metadata) -> bool {
    false
}

/// The path that `path` is read through: itself, or when it is a link, the local file it leads to (resolved hop by hop). `Err`
/// says why not: nothing is there (or the link is dangling, which gix reads as nothing too), or it leads somewhere that must not
/// be followed.
fn followed(path: &Path) -> Result<PathBuf, Seen<()>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if missing(&error) => return Err(Seen::Missing),
        Err(_) => return Err(Seen::Odd),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(path.to_path_buf());
    }

    match links::resolve(path) {
        Ok(resolved) => Ok(resolved),
        Err(error) if links::leads_nowhere(&error) => Err(Seen::Missing),
        Err(_) => Err(Seen::Odd),
    }
}

/// Like [`seen`], and a link is followed to a local place (see [`followed`]) and what it leads to is looked at.
pub(super) fn seen_following(path: &Path, cap: u64) -> Seen<()> {
    match followed(path) {
        Ok(resolved) => seen(&resolved, cap),
        Err(why) => why,
    }
}

/// Like [`read`], and a link is followed to a local place (see [`followed`]) and what it leads to is read.
pub(super) fn read_following(path: &Path, cap: u64) -> Seen<Vec<u8>> {
    match followed(path) {
        // Looked at before it is opened: a device at the end of a link is not opened at all.
        Ok(resolved) => match seen(&resolved, cap) {
            Seen::Plain(()) => read(&resolved, cap),
            Seen::Missing => Seen::Missing,
            Seen::Odd => Seen::Odd,
            Seen::Large => Seen::Large,
        },
        Err(Seen::Missing) => Seen::Missing,
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

/// Like [`seen_following`] for the file `relative` below `root` (see [`below`]).
pub(super) fn seen_following_below(root: &Path, relative: &str, cap: u64) -> Seen<()> {
    match below(root, relative) {
        Ok(path) => seen_following(&path, cap),
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

/// The most a rules file of the work tree (`.gitignore`, `.gitattributes`) may hold to be read: a pattern file bigger than this is
/// not one a person wrote, and gix holds every pattern of the files of the folders it is in.
const RULES_BYTES: u64 = 256 * 1024;

/// The most the rules files of one status may add up to: the ones of the folder and its parents, and of every folder the
/// directory walk enters.
pub(super) const RULES_TOTAL_BYTES: u64 = 4 * 1024 * 1024;

/// What a rules file of the work tree is to the directory walk, which opens it by itself where it finds one (and does not
/// follow a link).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Rules {
    /// Not one to open (not there, a link, a folder), or a regular file of this many bytes.
    Bytes(u64),
    /// A named pipe, a device or a socket, which an open waits on or a read never ends, or a regular file too big to be rules.
    Risky,
}

/// Looks at the rules file `path` of the work tree with `lstat`: see [`Rules`].
pub(super) fn rules_file(path: &Path) -> Rules {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Rules::Bytes(0);
    };
    let kind = metadata.file_type();

    if kind.is_file() {
        if metadata.len() > RULES_BYTES {
            Rules::Risky
        } else {
            Rules::Bytes(metadata.len())
        }
    } else if kind.is_symlink() || kind.is_dir() {
        Rules::Bytes(0)
    } else {
        Rules::Risky
    }
}

/// Whether the rules file `path` is one gix must not be handed (see [`Rules::Risky`]).
pub(super) fn risky_rules_file(path: &Path) -> bool {
    rules_file(path) == Rules::Risky
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
        assert_eq!(seen_following_below(folder.path(), "sub/file", 10), Seen::Plain(()));
        assert_eq!(seen_following_below(folder.path(), "sub/none", 10), Seen::Missing);
        assert_eq!(seen_following_below(folder.path(), "none/file", 10), Seen::Missing);
        assert_eq!(seen_following_below(folder.path(), "sub/file/deeper", 10), Seen::Odd);
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
        assert_eq!(seen_following_below(folder.path(), "via/file", 10), Seen::Odd);
    }

    fn folder_of(path: &Path) -> Seen<()> {
        folder(path)
    }

    #[test]
    fn a_real_folder_is_plain_and_a_file_or_nothing_is_missing() {
        let folder = tempfile::tempdir().expect("a folder");
        fs::create_dir(folder.path().join("sub")).expect("a folder");
        fs::write(folder.path().join("file"), b"x").expect("a file");

        assert_eq!(folder_of(&folder.path().join("sub")), Seen::Plain(()));
        assert_eq!(folder_of(&folder.path().join("file")), Seen::Missing);
        assert_eq!(folder_of(&folder.path().join("none")), Seen::Missing);
        assert_eq!(folder_of(&folder.path().join("file/deeper")), Seen::Missing);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_for_a_folder_is_odd_and_a_link_for_a_file_is_followed_only_where_asked_and_only_to_a_plain_local_file() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        fs::create_dir(root.join("sub")).expect("a folder");
        fs::write(root.join("file"), b"12345").expect("a file");
        crate::view::testing::fifo(&root, "pipe");
        for (name, target) in [
            ("to-folder", root.join("sub")),
            ("to-file", root.join("file")),
            ("to-pipe", root.join("pipe")),
            ("to-device", "/dev/zero".into()),
            ("to-network", "/net/host/file".into()),
            ("dangling", root.join("gone")),
        ] {
            std::os::unix::fs::symlink(target, root.join(name)).expect("a link");
        }

        assert_eq!(folder_of(&root.join("to-folder")), Seen::Odd);
        // Not followed by the strict look, followed by the other.
        assert_eq!(seen(&root.join("to-file"), 10), Seen::Odd);
        assert_eq!(seen_following(&root.join("to-file"), 10), Seen::Plain(()));
        assert_eq!(seen_following(&root.join("to-file"), 4), Seen::Large);
        assert_eq!(
            read_following(&root.join("to-file"), 10),
            Seen::Plain(b"12345".to_vec())
        );
        assert_eq!(read_following(&root.join("file"), 10), Seen::Plain(b"12345".to_vec()));
        for name in ["to-pipe", "to-device", "to-network", "to-folder"] {
            assert_eq!(seen_following(&root.join(name), 10), Seen::Odd, "{name}");
            assert_eq!(read_following(&root.join(name), 10), Seen::Odd, "{name}");
        }
        // Nothing at the end of it is a file that is not there, which gix reads as none.
        assert_eq!(seen_following(&root.join("dangling"), 10), Seen::Missing);
        assert_eq!(read_following(&root.join("none"), 10), Seen::Missing);
    }

    #[test]
    fn a_rules_file_is_its_size_unless_it_is_a_pipe_a_device_or_too_big() {
        let folder = tempfile::tempdir().expect("a folder");
        fs::write(folder.path().join("small"), b"*.log\n").expect("rules");
        fs::write(folder.path().join("big"), vec![b'#'; RULES_BYTES as usize + 1]).expect("rules");
        fs::create_dir(folder.path().join("folder")).expect("a folder");

        assert_eq!(rules_file(&folder.path().join("small")), Rules::Bytes(6));
        assert_eq!(rules_file(&folder.path().join("none")), Rules::Bytes(0));
        assert_eq!(rules_file(&folder.path().join("folder")), Rules::Bytes(0));
        assert_eq!(rules_file(&folder.path().join("big")), Rules::Risky);
        assert!(risky_rules_file(&folder.path().join("big")));
        #[cfg(unix)]
        {
            crate::view::testing::fifo(folder.path(), "pipe");
            assert_eq!(rules_file(&folder.path().join("pipe")), Rules::Risky);
        }
    }
}

//! Looking at a file of a repository before gix opens it. gix opens what a repository names with a plain `open`, which on
//! a named pipe waits for a writer for ever, and reads it up to its end, which on `/dev/zero` (a link to it, or the device
//! made as a file) never comes. So each file gix is about to read is first looked at: `lstat` says it is a regular file (no
//! link is followed, nothing is opened for anything else), and then it is opened without blocking and without following a
//! link, and what was opened is looked at again with `fstat`, which is the size that counts: the size `lstat` gives is
//! nothing to rely on (a file of `/proc`, `/sys` or `/sys/kernel/debug` says 0 and reads without an end), so on Linux a file
//! of a pseudo file system (procfs, sysfs, debugfs, cgroup, tracefs and the like) is odd too. A file that gix reads whole
//! and that is small is read here, up to its cap and one byte more ([`read`]), which is the only size there is. No file of
//! a repository's own is allowed to be a link: a link anywhere among them is a repository that is not read (see
//! [`super::inspect`]).

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
    /// Anything else: a link, a pipe, a device, a socket, a folder, a file of a pseudo file system, a file that cannot be
    /// looked at.
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

/// The file at `path` opened to be looked at or read: `lstat` first (a link, a pipe or a device is not opened), then an
/// open that neither waits nor follows a link, then `fstat` on what was opened, which is also the file that is read.
fn opened(path: &Path, cap: u64) -> Result<File, Seen<()>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(Seen::Odd),
        Err(error) if missing(&error) => return Err(Seen::Missing),
        Err(_) => return Err(Seen::Odd),
    }

    let file = match open(path) {
        Ok(file) => file,
        Err(error) if missing(&error) => return Err(Seen::Missing),
        Err(_) => return Err(Seen::Odd),
    };
    let metadata = file.metadata().map_err(|_| Seen::Odd)?;

    match judge(&metadata, cap) {
        Seen::Plain(()) if on_pseudo_file_system(&file) => Err(Seen::Odd),
        Seen::Plain(()) => Ok(file),
        Seen::Large => Err(Seen::Large),
        Seen::Odd | Seen::Missing => Err(Seen::Odd),
    }
}

/// Looks at `path`: a regular file of at most `cap` bytes (as `fstat` of the opened file says), or not there, is fine.
pub(super) fn seen(path: &Path, cap: u64) -> Seen<()> {
    match opened(path, cap) {
        Ok(_) => Seen::Plain(()),
        Err(why) => why,
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

/// `f_type` of `statfs(2)` for the file systems that make their files up: procfs, sysfs, debugfs, cgroup, tracefs and the like.
#[cfg(target_os = "linux")]
const PSEUDO: [u64; 14] = [
    0x0000_9fa0, // procfs
    0x6265_6572, // sysfs
    0x6462_6720, // debugfs
    0x0027_e0eb, // cgroup
    0x6367_7270, // cgroup2
    0x7472_6163, // tracefs
    0x7363_6673, // securityfs
    0xcafe_4a11, // bpf
    0xf97c_ff8c, // selinuxfs
    0x6165_676c, // pstore
    0x6265_6570, // configfs
    0x4249_4e4d, // binfmt_misc
    0x6573_5543, // fuse control
    0x0000_1cd1, // devpts
];

/// Whether `file` is on a file system of the kernel's own (procfs, sysfs, debugfs, cgroup, tracefs, ...), whose files say
/// they are regular, give a size of 0 and read as much as the kernel makes up. A file whose file system cannot be told is
/// taken for one.
#[cfg(target_os = "linux")]
fn on_pseudo_file_system(file: &File) -> bool {
    match rustix::fs::fstatfs(file) {
        Ok(stat) => u64::try_from(stat.f_type).map_or(true, |kind| PSEUDO.contains(&kind)),
        Err(_) => true,
    }
}

#[cfg(not(target_os = "linux"))]
fn on_pseudo_file_system(_file: &File) -> bool {
    false
}

/// Whether the folder `path` is on a file system of the kernel's own (see [`on_pseudo_file_system`]), as `statfs` says. One that
/// is not there is not on any; one that cannot be told is taken for one. Only Linux has such file systems that are not special
/// folders of their own (macOS and Windows have none that hold a repository).
#[cfg(target_os = "linux")]
pub(super) fn on_pseudo_path(path: &Path) -> bool {
    match rustix::fs::statfs(path) {
        Ok(stat) => u64::try_from(stat.f_type).map_or(true, |kind| PSEUDO.contains(&kind)),
        Err(error) => error != rustix::io::Errno::NOENT && error != rustix::io::Errno::NOTDIR,
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn on_pseudo_path(_path: &Path) -> bool {
    false
}

/// Whether the entry of a folder that was listed is a regular file, by what the listing says of it and without following a
/// link; on Windows a reparse point is not one either, whatever its attributes otherwise say.
pub(super) fn is_plain_entry(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_file()) && not_reparse_point(entry)
}

/// Whether the entry of a folder that was listed is a real folder (no link or reparse point).
pub(super) fn is_plain_folder_entry(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_dir()) && not_reparse_point(entry)
}

/// Windows says what a reparse point is by the attributes of the entry, which the listing has already (no call is made for them).
#[cfg(windows)]
fn not_reparse_point(entry: &fs::DirEntry) -> bool {
    entry.metadata().is_ok_and(|metadata| !is_reparse_point(&metadata))
}

#[cfg(not(windows))]
fn not_reparse_point(_entry: &fs::DirEntry) -> bool {
    true
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

/// Like [`read`] for the file `relative` below `root` (see [`below`]).
pub(super) fn read_below(root: &Path, relative: &str, cap: u64) -> Seen<Vec<u8>> {
    match below(root, relative) {
        Ok(path) => read(&path, cap),
        Err(Seen::Missing) => Seen::Missing,
        Err(_) => Seen::Odd,
    }
}

/// What a rules file of the work tree is to the directory walk, which opens it by itself where it finds one (and does not
/// follow a link).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Rules {
    /// Not one to open (not there, a link, a folder), or a regular file of this many bytes.
    Bytes(u64),
    /// A regular file over the cap: no one wrote this as rules, and gix holds every pattern of the files it reads.
    Large,
    /// A named pipe, a device, a socket or a file of a pseudo file system, which an open waits on or a read never ends.
    Odd,
}

/// Looks at the rules file `path` of the work tree (`.gitignore`, `.gitattributes`): see [`Rules`]. The size is the one
/// `fstat` gives of the opened file.
pub(super) fn rules_file(path: &Path, cap: u64) -> Rules {
    let kind = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if missing(&error) => return Rules::Bytes(0),
        Err(_) => return Rules::Odd,
    };
    if kind.is_symlink() || kind.is_dir() {
        return Rules::Bytes(0);
    }

    match opened(path, cap) {
        Ok(file) => Rules::Bytes(file.metadata().map_or(0, |metadata| metadata.len())),
        Err(Seen::Large) => Rules::Large,
        Err(Seen::Missing) => Rules::Bytes(0),
        Err(_) => Rules::Odd,
    }
}

/// Reads the small regular file `path` (at most `cap` bytes, and one byte more is read to tell a file that grew): see
/// [`opened`] for how it is looked at first.
pub(super) fn read(path: &Path, cap: u64) -> Seen<Vec<u8>> {
    let file = match opened(path, cap) {
        Ok(file) => file,
        Err(Seen::Missing) => return Seen::Missing,
        Err(Seen::Large) => return Seen::Large,
        Err(_) => return Seen::Odd,
    };

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

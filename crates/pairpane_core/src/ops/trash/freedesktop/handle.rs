//! Folders held as open handles, and the rules a Trash folder must meet.
//!
//! Every folder of the Trash is opened relative to the one before it, without following a link, and judged by the
//! `fstat` of the handle that was opened, never by its path. What is judged is then what is used: swapping a name for
//! something else afterwards changes nothing for the handles already held.

use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;

use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, Stat, fstat, mkdirat, openat, statat};
use rustix::io::Errno;

use crate::ops::report::refusal;

/// What the user is told when an item cannot go to the Trash of its drive.
pub(super) const NO_TRASH: &str = "this drive has no usable Trash; delete it permanently instead";

/// The sticky bit.
const STICKY: u32 = 0o1000;
/// The write bits of the group and of others.
const OTHERS_WRITE: u32 = 0o022;

/// Opens a folder as a handle only: Linux has a flag for exactly that, which needs no permission to read the folder
/// (the folders above an item are often only searchable). Elsewhere the folder is opened for reading, and so a folder
/// that cannot be read there cannot be climbed out of through `..` (see [`open_parent`]): an item below one has no
/// Trash on those systems, and is refused like any item whose drive cannot be found.
#[cfg(target_os = "linux")]
const HANDLE: OFlags = OFlags::PATH;
#[cfg(not(target_os = "linux"))]
const HANDLE: OFlags = OFlags::RDONLY;

/// How every folder is opened: it must be a folder.
const FOLDER: OFlags = OFlags::DIRECTORY.union(OFlags::CLOEXEC).union(HANDLE);

/// What the rules need to know of a folder, read from its handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Facts {
    /// A folder, not a link to one.
    pub is_folder: bool,
    pub owner: u32,
    pub mode: u32,
    pub device: u64,
    pub inode: u64,
}

/// A number read from `stat` that is a different integer type on different systems.
fn widen<T: TryInto<u64>>(value: T) -> u64 {
    value.try_into().unwrap_or(0)
}

impl Facts {
    fn of(stat: &Stat) -> Self {
        Self {
            is_folder: FileType::from_raw_mode(stat.st_mode) == FileType::Directory,
            owner: stat.st_uid,
            mode: u32::try_from(widen(stat.st_mode)).unwrap_or(0),
            device: widen(stat.st_dev),
            inode: widen(stat.st_ino),
        }
    }

    /// The facts of the folder (or other file) `fd` is a handle of.
    pub(super) fn of_handle(fd: &impl AsFd) -> io::Result<Self> {
        fstat(fd).map(|stat| Self::of(&stat)).map_err(io::Error::from)
    }

    /// Whether nobody else can rename entries in this folder: it is a real folder of root or of `uid`, and the group
    /// and others cannot write to it, or the sticky bit keeps them to their own entries. (Looser mode bits are common
    /// on FAT, exFAT and NTFS drives, where every file has the same owner and mode, so the mode alone is not asked to
    /// be private.)
    pub(super) fn is_guarded(&self, uid: u32) -> bool {
        let owner_ok = self.owner == 0 || self.owner == uid;
        let closed = self.mode & OTHERS_WRITE == 0 || self.mode & STICKY != 0;

        self.is_folder && owner_ok && closed
    }

    /// Whether both are the same folder: one device and one inode.
    pub(super) fn is_same_place(&self, other: &Self) -> bool {
        self.device == other.device && self.inode == other.inode
    }

    /// A guarded folder of `uid` itself.
    fn is_own(&self, uid: u32) -> bool {
        self.owner == uid && self.is_guarded(uid)
    }

    /// A guarded folder with the sticky bit.
    fn is_shared(&self, uid: u32) -> bool {
        self.mode & STICKY != 0 && self.is_guarded(uid)
    }
}

/// What a folder must be, for the user `uid`, to be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rule {
    /// Guarded (see [`Facts::is_guarded`]), of root or of the user: the top folder of a drive.
    Guarded,
    /// Guarded, and sticky: `.Trash` of a drive.
    Shared,
    /// Guarded and the user's own: a Trash folder and its `files` and `info`.
    Own,
}

impl Rule {
    pub(super) fn allows(self, facts: &Facts, uid: u32) -> bool {
        match self {
            Self::Guarded => facts.is_guarded(uid),
            Self::Shared => facts.is_shared(uid),
            Self::Own => facts.is_own(uid),
        }
    }
}

/// What opening a folder step by step found.
#[derive(Debug)]
pub(super) enum Step {
    /// There is nothing of that name.
    Missing,
    /// Something is there that must not be used: a link, a file, or a folder that breaks the rule.
    Refused,
    /// The folder, opened and judged.
    Open(OwnedFd),
}

impl Step {
    /// The folder, or the refusal every unusable Trash folder is answered with (the reason goes to the log).
    pub(super) fn or_refuse(self, what: &str) -> io::Result<OwnedFd> {
        match self {
            Self::Open(fd) => Ok(fd),
            Self::Missing | Self::Refused => Err(unusable(what)),
        }
    }
}

/// Opens the folder `name` of `parent` without following a link, and judges the folder that was opened by `rule`.
pub(super) fn step(parent: &impl AsFd, name: &OsStr, rule: Rule, uid: u32) -> Step {
    let fd = match openat(parent, name, FOLDER | OFlags::NOFOLLOW, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Step::Missing,
        Err(errno) => {
            tracing::debug!(%errno, "a Trash folder cannot be opened");

            return Step::Refused;
        }
    };

    match Facts::of_handle(&fd) {
        Ok(facts) if rule.allows(&facts, uid) => Step::Open(fd),
        Ok(facts) => {
            tracing::debug!(?facts, ?rule, "a Trash folder breaks the rule for it");

            Step::Refused
        }
        Err(error) => {
            tracing::debug!(?error, "a Trash folder cannot be looked at");

            Step::Refused
        }
    }
}

/// [`step`], and when the folder is missing it is made (permissions of the owner only) and opened and judged again.
pub(super) fn step_or_make(parent: &impl AsFd, name: &OsStr, rule: Rule, uid: u32) -> Step {
    match step(parent, name, rule, uid) {
        Step::Missing => {}
        found => return found,
    }

    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(errno) => {
            tracing::debug!(%errno, "a Trash folder cannot be made");

            return Step::Refused;
        }
    }

    match step(parent, name, rule, uid) {
        Step::Missing => Step::Refused,
        found => found,
    }
}

/// Opens the folder at `path`, following links on the way (the path was named by the user or by the environment).
pub(super) fn open_folder(path: &Path) -> io::Result<OwnedFd> {
    openat(CWD, path, FOLDER, Mode::empty()).map_err(io::Error::from)
}

/// Opens the folder at `path` without following a link at its end (a link on the way is still followed, so the path
/// should be a canonical one).
pub(super) fn open_folder_exact(path: &Path) -> io::Result<OwnedFd> {
    openat(CWD, path, FOLDER | OFlags::NOFOLLOW, Mode::empty()).map_err(io::Error::from)
}

/// Opens the folder that holds the folder `fd` is a handle of (the folder itself, at the root).
pub(super) fn open_parent(fd: &impl AsFd) -> io::Result<OwnedFd> {
    openat(fd, "..", FOLDER, Mode::empty()).map_err(io::Error::from)
}

/// When a folder was made (seconds and nanoseconds since 1970), which the system tells only for some file systems.
pub(super) type Birth = (i64, u32);

/// The time the folder (or file) `fd` is a handle of was made, as Linux 4.11 and later tell it. `None` where the
/// system or the file system does not say.
#[cfg(target_os = "linux")]
pub(super) fn birth_of(fd: &impl AsFd) -> Option<Birth> {
    use rustix::fs::{StatxFlags, statx};

    let stat = statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::BTIME).ok()?;

    // The field means something only when the mask says it was filled in.
    if !StatxFlags::from_bits_truncate(stat.stx_mask).contains(StatxFlags::BTIME) {
        return None;
    }

    Some((stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn birth_of(_: &impl AsFd) -> Option<Birth> {
    None
}

/// Whether `name` is in the folder `dir`, a link counted as itself.
pub(super) fn exists_in(dir: &impl AsFd, name: &OsStr) -> io::Result<bool> {
    match statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(errno) => Err(io::Error::from(errno)),
    }
}

/// Moves `from` of `from_dir` to `to` of `to_dir` and replaces nothing: `AlreadyExists` when `to` is taken.
///
/// Where the system or the file system cannot refuse by itself, the name is looked at first, and someone who can write
/// to the folder in that moment can still win the race.
pub(super) fn rename_noreplace(from_dir: &impl AsFd, from: &OsStr, to_dir: &impl AsFd, to: &OsStr) -> io::Result<()> {
    match system_noreplace(from_dir, from, to_dir, to) {
        Err(errno) if [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP].contains(&errno) => {
            if exists_in(to_dir, to)? {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }

            rustix::fs::renameat(from_dir, from, to_dir, to).map_err(io::Error::from)
        }
        result => result.map_err(io::Error::from),
    }
}

#[cfg(target_os = "linux")]
fn system_noreplace(from_dir: &impl AsFd, from: &OsStr, to_dir: &impl AsFd, to: &OsStr) -> Result<(), Errno> {
    rustix::fs::renameat_with(from_dir, from, to_dir, to, rustix::fs::RenameFlags::NOREPLACE)
}

/// Systems with no such flag are answered like the ones that refuse it.
#[cfg(not(target_os = "linux"))]
fn system_noreplace(_: &impl AsFd, _: &OsStr, _: &impl AsFd, _: &OsStr) -> Result<(), Errno> {
    Err(Errno::NOSYS)
}

/// The error for a Trash folder that is refused: the reason goes to the log, the user gets [`NO_TRASH`].
pub(super) fn unusable(reason: &str) -> io::Error {
    tracing::warn!(reason, "a Trash folder cannot be used");

    refusal(io::ErrorKind::Other, NO_TRASH)
}

#[cfg(test)]
mod tests;

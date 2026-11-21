//! What the editor asks of the system that differs by platform: the revision of a file, whether the user may write it,
//! how it is opened for writing, whether a save may replace it with a new file, how it does that and whether two paths
//! are one file.

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
use crate::listing::platform;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

/// What a file is, as one text, to tell on save whether it changed since it was read: Unix has its size, its times
/// (modification and status change, to the nanosecond), its device and its inode; Windows its size, its last write
/// and its creation time. A file that is replaced by another, or written, has another revision.
#[cfg(unix)]
pub(super) fn revision(meta: &Metadata) -> String {
    format!(
        "{}-{}.{}-{}.{}-{}-{}",
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
        meta.dev(),
        meta.ino()
    )
}

#[cfg(windows)]
pub(super) fn revision(meta: &Metadata) -> String {
    format!("{}-{}-{}", meta.len(), meta.last_write_time(), meta.creation_time())
}

/// Whether the user may write the file at `path` (Unix: asked of the system, which looks at the owner, the group, the
/// ACLs and a read-only mount; Windows: the read-only attribute).
#[cfg(unix)]
pub(super) fn writable(path: &Path, _meta: &Metadata) -> bool {
    rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_ok()
}

#[cfg(windows)]
pub(super) fn writable(_path: &Path, meta: &Metadata) -> bool {
    !meta.permissions().readonly()
}

/// Opens the existing file at `path` for writing without following a link; it is not cut short, and a pipe that slips
/// in is never waited on and a terminal never taken. Whether it is the file that was seen is for the caller to check.
#[cfg(unix)]
pub(super) fn open_for_write(path: &Path) -> io::Result<File> {
    use rustix::fs::OFlags;

    let flags = (OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY).bits();

    OpenOptions::new().write(true).custom_flags(flags as i32).open(path)
}

#[cfg(windows)]
pub(super) fn open_for_write(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .custom_flags(crate::ops::OPEN_REPARSE_POINT)
        .open(path)
}

/// How a save puts the text in place of the file that is there.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Strategy {
    /// Write a new file next to it and rename that over it: the file is whole or as it was.
    Rename,
    /// Write into the file itself, which keeps its identity.
    InPlace,
}

/// Whether a save may replace the file with a new one: only a file with one name and no extended attributes (which
/// hold POSIX ACLs and labels that a new file would not have), that the user owns and whose group a new file next to it
/// would keep (`temp_gid` is the group the new file got: in a setgid folder it is the folder's). Any other file is
/// written in place, so that its other names, its owner, its group, its ACLs and its attributes stay what they are.
#[cfg(unix)]
pub(super) fn strategy(nlink: u64, uid: u32, gid: u32, my_uid: u32, temp_gid: u32, has_xattrs: bool) -> Strategy {
    if nlink == 1 && uid == my_uid && gid == temp_gid && !has_xattrs {
        Strategy::Rename
    } else {
        Strategy::InPlace
    }
}

/// Whether the file behind `file` has any extended attribute (an ACL is kept in one). When that cannot be told, it
/// does: a file that may have one is written in place.
#[cfg(all(
    unix,
    any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios")
))]
pub(super) fn has_xattrs(file: &File) -> bool {
    match rustix::fs::flistxattr_query(file) {
        Ok(length) => length > 0,
        // A filesystem without extended attributes has none.
        Err(error) => error != rustix::io::Errno::NOTSUP,
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))
))]
pub(super) fn has_xattrs(_file: &File) -> bool {
    true
}

/// Whether a save of the file `seen`, which `file` is the handle it was checked through, may replace it with `temp`, the
/// new file next to it.
#[cfg(unix)]
pub(super) fn may_rename(seen: &Metadata, file: &File, temp: &File) -> io::Result<bool> {
    let made = temp.metadata()?;
    let mine = rustix::process::geteuid().as_raw();

    Ok(strategy(seen.nlink(), seen.uid(), seen.gid(), mine, made.gid(), has_xattrs(file)) == Strategy::Rename)
}

#[cfg(windows)]
pub(super) fn may_rename(_seen: &Metadata, _file: &File, _temp: &File) -> io::Result<bool> {
    Ok(true)
}

/// The permissions the new file gets: those of the replaced file, without the setuid and setgid bits (a new file made by
/// the user is not to carry a privilege that the old one had from its owner).
#[cfg(unix)]
pub(super) fn kept_permissions(seen: &Metadata) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;

    std::fs::Permissions::from_mode(seen.mode() & !0o6000)
}

#[cfg(windows)]
pub(super) fn kept_permissions(seen: &Metadata) -> std::fs::Permissions {
    seen.permissions()
}

/// Puts the new file `temp` in place of `target` (Unix: renamed over it; Windows: `ReplaceFileW`, which keeps the ACLs,
/// the alternate streams and the attributes of the replaced file).
#[cfg(unix)]
pub(super) fn replace_with(temp: &Path, target: &Path) -> io::Result<()> {
    std::fs::rename(temp, target)
}

#[cfg(windows)]
pub(super) fn replace_with(temp: &Path, target: &Path) -> io::Result<()> {
    super::win_replace::replace_file(target, temp)
}

/// `ERROR_SHARING_VIOLATION`: another program has the replaced file open without sharing.
const WIN_SHARING_VIOLATION: i32 = 32;

/// `ERROR_UNABLE_TO_REMOVE_REPLACED`: the replaced file could not be replaced.
const WIN_UNABLE_TO_REMOVE_REPLACED: i32 = 1175;

/// Whether `ReplaceFileW` that failed with the system error `code` (the file is replaced by it with no backup made) left
/// the replaced file as it was under its name, so that the save may go on writing into it. That is the case for a sharing
/// violation and for 1175, for which the documentation says the replaced and the replacement files keep their names. It
/// is not for `ERROR_UNABLE_TO_MOVE_REPLACEMENT` (1176), where without a backup file "the replaced file no longer exists
/// under its original name", nor for 1177 (the replacement no longer exists under its name): there the original may be
/// gone, so the save fails and is not retried on a file that may not be there. Any other code is a failure as well. A pure
/// function of the code so that it is tested on every system.
fn original_left_alone(code: i32) -> bool {
    matches!(code, WIN_SHARING_VIOLATION | WIN_UNABLE_TO_REMOVE_REPLACED)
}

/// Whether the failure `error` of [`replace_with`] left the target as it was, so that the save may write into it in place
/// (Windows only; a rename on another system has no such failure).
pub(super) fn replace_left_original(error: &io::Error) -> bool {
    cfg!(windows) && error.raw_os_error().is_some_and(original_left_alone)
}

/// Whether `now`, the file opened for writing, is `seen`, the file that was checked (Unix: the same device and inode;
/// Windows, which has no identity here: the same size, last write and creation time).
#[cfg(unix)]
pub(super) fn is_seen(now: &Metadata, seen: &Metadata) -> bool {
    platform::identity(now) == platform::identity(seen)
}

#[cfg(windows)]
pub(super) fn is_seen(now: &Metadata, seen: &Metadata) -> bool {
    revision(now) == revision(seen)
}

/// Whether the paths `a` and `b` are one file (a hard link or a link to it counts), by where they are on disk. A path
/// that is not there is another file than any; one that cannot be looked at is told to be the same, which is the
/// answer that refuses.
#[cfg(unix)]
pub(super) fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => platform::identity(&a) == platform::identity(&b),
        (Err(error), _) | (_, Err(error)) => error.kind() != io::ErrorKind::NotFound,
    }
}

#[cfg(windows)]
pub(super) fn same_file(a: &Path, b: &Path) -> bool {
    match same_file::is_same_file(a, b) {
        Ok(same) => same,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_failure_that_leaves_the_replaced_file_where_it_was_is_written_into_in_place() {
        // ERROR_SHARING_VIOLATION and ERROR_UNABLE_TO_REMOVE_REPLACED: both files keep their names.
        assert!(original_left_alone(32));
        assert!(original_left_alone(1175));

        // ERROR_UNABLE_TO_MOVE_REPLACEMENT and _2: the original may be gone, so the save fails.
        assert!(!original_left_alone(1176));
        assert!(!original_left_alone(1177));

        // Access denied, a missing file, a full disk and what is not an error code.
        for code in [0, 2, 5, 112, 1178] {
            assert!(!original_left_alone(code), "{code}");
        }
    }

    #[test]
    fn the_fallback_is_for_windows_errors_only() {
        let sharing = io::Error::from_raw_os_error(32);

        assert_eq!(replace_left_original(&sharing), cfg!(windows));
        assert!(!replace_left_original(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
    }
}

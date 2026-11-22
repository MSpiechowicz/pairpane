//! Renaming an item of a folder held open, without replacing anything: on Unix by the handle of the folder (`renameat`),
//! so the names are looked up in that folder whatever happens to the path it was opened by; on Windows by path, as the
//! other Windows jobs go.
//!
//! Races that remain, as in the other jobs of this crate: where the filesystem cannot refuse a replacing rename itself,
//! the check for something at the new name and the rename are two steps, and someone who can write to the folder in between
//! can win; and on Windows the folder is a path, so a folder swapped for a link between two renames is followed.

use std::io;

use super::held::Held;

#[cfg(unix)]
impl Held {
    /// Renames the item `from` of this folder to `to`, unless something (a link too) is at `to`: then `AlreadyExists`,
    /// and nothing changes. Both are plain names.
    ///
    /// Linux and macOS ask the system to refuse (`renameat2` `NOREPLACE`, `renameatx_np` `EXCL`), so nothing can appear
    /// at `to` between the check and the rename. Where the filesystem lacks that flag, and on the other systems, it is
    /// check-then-rename: someone who can write to the folder at that very moment can still win the race.
    pub(super) fn rename_noreplace(&self, from: &str, to: &str) -> io::Result<()> {
        system_noreplace(&self.dir, from, to)
    }
}

#[cfg(windows)]
impl Held {
    /// Renames the item `from` of this folder to `to`, unless something is at `to` (see `rename::noreplace`).
    pub(super) fn rename_noreplace(&self, from: &str, to: &str) -> io::Result<()> {
        super::rename::noreplace(&self.path.join(from), &self.path.join(to))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn system_noreplace(dir: &std::fs::File, from: &str, to: &str) -> io::Result<()> {
    use rustix::fs::{RenameFlags, renameat_with};
    use rustix::io::Errno;

    match renameat_with(dir, from, dir, to, RenameFlags::NOREPLACE) {
        // The system or the filesystem does not know the flag (macOS answers `ENOTSUP` for a filesystem without `EXCL`;
        // on Linux the two codes are one).
        Err(errno) if [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP, Errno::NOTSUP].contains(&errno) => {
            check_then_rename(dir, from, to)
        }
        result => result.map_err(io::Error::from),
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn system_noreplace(dir: &std::fs::File, from: &str, to: &str) -> io::Result<()> {
    check_then_rename(dir, from, to)
}

#[cfg(unix)]
fn check_then_rename(dir: &std::fs::File, from: &str, to: &str) -> io::Result<()> {
    use rustix::fs::{AtFlags, renameat, statat};
    use rustix::io::Errno;

    match statat(dir, to, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(Errno::NOENT) => renameat(dir, from, dir, to).map_err(io::Error::from),
        Err(errno) => Err(io::Error::from(errno)),
    }
}

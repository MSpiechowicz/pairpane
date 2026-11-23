//! Removing one item for good, without reaching into a drive that is attached inside it.
//!
//! `fs::remove_dir_all` goes down into a mounted drive like into any folder and deletes what is on it, so a
//! folder that holds a mount (a USB drive, a bind mount, a network share) must be refused before anything of it is
//! removed. On Windows std's `remove_dir_all` opens every folder with `FILE_FLAG_OPEN_REPARSE_POINT` and
//! `OBJ_DONT_REPARSE` (library/std/src/sys/fs/windows/remove_dir_all.rs, `open_link_no_reparse`), so a mounted
//! volume, which is a reparse point, is deleted as the entry it is and never entered: nothing more is needed there.

use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(unix)]
use super::report::refusal;

/// What the refusal says. It holds no path.
#[cfg(unix)]
const HOLDS_A_MOUNT: &str = "contains a mounted drive or volume; unmount or delete it separately";

/// Removes a folder with its contents, or a file, or a link (the link only, never what it leads to).
///
/// A folder that is a mount point, or holds one anywhere below it, is refused with [`io::ErrorKind::InvalidInput`]
/// and nothing of it is removed (on Unix; see the module text for Windows). A mount that is made after the check
/// is not caught: the check and the removal are two steps.
pub(super) fn remove(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        #[cfg(unix)]
        refuse_mounts(path, &is_mount)?;

        return fs::remove_dir_all(path);
    }

    match fs::remove_file(path) {
        // A link to a folder is removed like a folder on Windows.
        Err(_) if cfg!(windows) && metadata.file_type().is_symlink() => fs::remove_dir(path),
        result => result,
    }
}

/// Fails when the folder `root`, or a folder below it, is the top of another mounted drive.
///
/// `is_mount(folder, outer)` tells whether `folder` is a mount point, where `outer` is the device of the folder
/// that holds `root`. A folder on the same device as that one, and not marked as a mount, is a plain folder.
/// Links are never followed, and a folder that cannot be read fails the check: it could not be removed either.
#[cfg(unix)]
pub(super) fn refuse_mounts(root: &Path, is_mount: &dyn Fn(&Path, u64) -> io::Result<bool>) -> io::Result<()> {
    let outer = root
        .parent()
        .map(fs::metadata)
        .transpose()?
        .map(|metadata| metadata.dev());
    let Some(outer) = outer else {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the root of a disk cannot be deleted",
        ));
    };

    let mut pending = vec![root.to_path_buf()];

    while let Some(folder) = pending.pop() {
        // A folder is looked at before it is read, so a mount is never entered.
        if is_mount(&folder, outer)? {
            return Err(refusal(io::ErrorKind::InvalidInput, HOLDS_A_MOUNT));
        }

        for entry in fs::read_dir(&folder)? {
            let entry = entry?;

            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }

    Ok(())
}

/// Whether `folder` is a mount point: on another device than `outer`, or (Linux 5.8 and later, which tell it) the
/// root of a mount on the same device, as a bind mount is.
#[cfg(unix)]
pub(super) fn is_mount(folder: &Path, outer: u64) -> io::Result<bool> {
    if fs::symlink_metadata(folder)?.dev() != outer {
        return Ok(true);
    }

    is_mount_root(folder)
}

#[cfg(target_os = "linux")]
fn is_mount_root(folder: &Path) -> io::Result<bool> {
    use rustix::fs::{AtFlags, CWD, StatxFlags, statx};

    is_mount_root_stat(statx(CWD, folder, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::empty()))
}

/// Whether what `statx` answered says a mount root.
#[cfg(target_os = "linux")]
pub(super) fn is_mount_root_stat(answer: rustix::io::Result<rustix::fs::Statx>) -> io::Result<bool> {
    use rustix::fs::StatxAttributes;
    use rustix::io::Errno;

    match answer {
        // An older kernel does not tell, and its answer is not "no": the device alone has to do.
        Ok(stat) => Ok(stat.stx_attributes_mask.contains(StatxAttributes::MOUNT_ROOT)
            && stat.stx_attributes.contains(StatxAttributes::MOUNT_ROOT)),
        Err(Errno::NOSYS | Errno::PERM | Errno::INVAL | Errno::OPNOTSUPP) => Ok(false),
        Err(errno) => Err(io::Error::from(errno)),
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn is_mount_root(_folder: &Path) -> io::Result<bool> {
    Ok(false)
}

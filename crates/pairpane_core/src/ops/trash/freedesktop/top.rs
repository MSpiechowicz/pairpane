//! The top folder of the drive a folder is on, found by walking up from a handle (no list of mount points is read).

use std::io;
use std::os::fd::OwnedFd;

use super::handle::{Facts, open_parent};
use crate::ops::report::refusal;

/// More folders than any path holds; a walk that goes on longer is not going up.
const MOST_STEPS: usize = 4096;

/// The top folder of a drive, and how many folders up from the start it is.
#[derive(Debug)]
pub(super) struct Top {
    pub fd: OwnedFd,
    pub steps: usize,
}

/// Finds the top folder of the drive that holds the folder `start` is a handle of: the first folder, going up, that is
/// the root of a mount (where the kernel tells), whose parent is on another device, or that is the root of all.
pub(super) fn top_of(start: &OwnedFd) -> io::Result<Top> {
    let mut current = start.try_clone()?;

    for steps in 0..MOST_STEPS {
        let here = Facts::of_handle(&current)?;
        if is_mount_root(&current) {
            return Ok(Top { fd: current, steps });
        }

        let parent = open_parent(&current)?;
        let above = Facts::of_handle(&parent)?;

        // The parent of the root of all is the root itself.
        let is_root = above.device == here.device && above.inode == here.inode;
        if is_root || above.device != here.device {
            return Ok(Top { fd: current, steps });
        }

        current = parent;
    }

    Err(refusal(
        io::ErrorKind::Other,
        "the folders above an item cannot be followed",
    ))
}

/// Whether the folder is the root of a mount, as Linux 5.8 and later tell. An older kernel does not say, and its answer
/// is not "no": the device of the parent has to do.
#[cfg(target_os = "linux")]
fn is_mount_root(fd: &OwnedFd) -> bool {
    use rustix::fs::{AtFlags, StatxAttributes, StatxFlags, statx};

    match statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::empty()) {
        Ok(stat) => {
            stat.stx_attributes_mask.contains(StatxAttributes::MOUNT_ROOT)
                && stat.stx_attributes.contains(StatxAttributes::MOUNT_ROOT)
        }
        Err(_) => false,
    }
}

#[cfg(not(target_os = "linux"))]
fn is_mount_root(_fd: &OwnedFd) -> bool {
    false
}

#[cfg(test)]
mod tests;

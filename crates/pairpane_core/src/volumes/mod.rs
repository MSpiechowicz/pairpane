//! The drive list: the places the drive menu offers (the system disk, the home folder, mounted and removable
//! volumes, network mounts) and the free space of each.
//!
//! Listing must never reach out to the network or wait on a device, so it only reads what the system already
//! has: the kernel's mount table on Linux (`/proc/self/mounts`), the names in `/Volumes` on macOS, the volume
//! list of the OS on Windows (local fixed and removable drives only; a mapped network drive is never listed or
//! touched). Free space is the part that can block (it asks the volume), so it is measured separately, by
//! [`measure`], on a thread the caller can afford to lose, and never on a network volume. A path with a link
//! anywhere on it is not measured either (the home folder may be a link to a share, or into a network mount), so
//! the home folder is measured only when it holds no links. On macOS only the system disk and such a home folder
//! are measured: the volumes in `/Volumes` are listed without free space, as the system offers no way to tell a
//! network share from a local drive without asking the volume.
//!
//! The parsing and filtering rules are plain functions of their input (see [`linux`], [`macos`], [`windows`]),
//! tested on every system.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::listing::platform::{self, check_folder};
use crate::model::{Volume, VolumeKind};

pub mod linux;
pub mod macos;
pub mod windows;

/// Volumes listed at most; a mount table is not trusted to be small.
const MAX_VOLUMES: usize = 64;

/// Whether [`list`] may block (Windows asks the OS about every drive), so the caller must run it on a thread it can
/// afford to lose. Elsewhere it reads a table the system keeps in memory, or the names in one folder.
pub const LISTING_MAY_BLOCK: bool = cfg!(windows);

/// A listed volume and whether [`measure`] may ask it for its free space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub volume: Volume,
    /// False for a network mount and for anything that could wait on something remote or dead.
    pub measure: bool,
}

/// Every place to offer, in menu order: system disk, home, drives, removable volumes, network mounts. Every path is
/// one a pane may open (see [`check_folder`]). `home` is the user's home folder.
pub fn list(home: Option<&Path>) -> Vec<Found> {
    let mut found = os_list(home.and_then(Path::to_str));
    found.retain(|found| check_folder(Path::new(&found.volume.path)).is_ok());

    order(found)
}

/// Asks each volume that may be asked (and whose free space is not known yet) for its free space, one after the
/// other in list order, and calls `emit` with its path and the bytes. A volume that does not answer holds the thread
/// for good, so the caller runs this on a capped thread. Stops when `emit` returns false.
pub fn measure(found: &[Found], mut emit: impl FnMut(&str, u64) -> bool) {
    for found in found {
        let askable = found.measure && found.volume.kind != VolumeKind::Network && found.volume.free.is_none();
        if !askable {
            continue;
        }

        let path = Path::new(&found.volume.path);
        if !is_link_free(path) {
            continue;
        }

        let Some(bytes) = platform::free_space(path) else {
            continue;
        };
        if !emit(&found.volume.path, bytes) {
            return;
        }
    }
}

/// Whether the absolute `path` holds no link. Asking free space follows links, so one to a network share or into a
/// network mount would contact that host. Each folder is checked from the root down with `symlink_metadata`, which
/// never follows the last component, and the walk stops at the first link: nothing beyond the path's own folders
/// (which the listing already found to be on a local volume) is ever touched, not even a link's target.
fn is_link_free(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }

    // Rebuilt from its components: a trailing separator (`link/`) would make `symlink_metadata` follow the link.
    let path: PathBuf = path.components().collect();
    let mut folders: Vec<&Path> = path.ancestors().collect();
    folders.reverse();

    folders
        .into_iter()
        .all(|folder| fs::symlink_metadata(folder).is_ok_and(|metadata| !metadata.file_type().is_symlink()))
}

/// The home folder as a volume. `measure` says whether its disk may be asked for free space.
pub fn home_volume(path: &str, measure: bool) -> Found {
    Found {
        volume: Volume {
            name: "Home".to_owned(),
            label: None,
            path: path.to_owned(),
            kind: VolumeKind::Home,
            free: None,
        },
        measure,
    }
}

/// `found` with the home folder added, for the systems whose own list has no entry for it.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn with_home(mut found: Vec<Found>, home: Option<&str>) -> Vec<Found> {
    found.extend(home.map(|home| home_volume(home, true)));

    found
}

/// Puts the kinds in menu order (keeping the order within a kind), drops a second volume at the same path and
/// limits the list to [`MAX_VOLUMES`].
fn order(mut found: Vec<Found>) -> Vec<Found> {
    found.sort_by_key(|found| rank(found.volume.kind));

    let mut seen = HashSet::new();
    found.retain(|found| seen.insert(found.volume.path.clone()));
    found.truncate(MAX_VOLUMES);

    found
}

fn rank(kind: VolumeKind) -> u8 {
    match kind {
        VolumeKind::System => 0,
        VolumeKind::Home => 1,
        VolumeKind::Drive => 2,
        VolumeKind::Removable => 3,
        VolumeKind::Network => 4,
    }
}

#[cfg(target_os = "linux")]
fn os_list(home: Option<&str>) -> Vec<Found> {
    linux::list(home)
}

#[cfg(target_os = "macos")]
fn os_list(home: Option<&str>) -> Vec<Found> {
    with_home(macos::list(), home)
}

#[cfg(windows)]
fn os_list(home: Option<&str>) -> Vec<Found> {
    with_home(windows::list(), home)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn os_list(home: Option<&str>) -> Vec<Found> {
    with_home(Vec::new(), home)
}

#[cfg(test)]
mod tests;

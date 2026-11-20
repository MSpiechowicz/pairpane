//! macOS volumes from the names in `/Volumes`.
//!
//! The kind of each entry comes from the directory listing itself, so nothing inside a mounted volume is read and a
//! dead network share cannot stall the list. The boot volume is a link to `/` there (System is listed already).
//!
//! Network shares (SMB, AFP, NFS, WebDAV), FUSE volumes (macFUSE) and automounts cannot be told from local drives by
//! name, and the way to tell by type needs a system call that has no safe binding here (`getfsstat`; `sysinfo`
//! would also stat every mount and ask the volume for its properties, which is what must not happen). So every
//! volume is listed as a drive and none is measured: asking a share for its free space contacts the server and
//! waits for it. Only the system disk (`/`) is measured, and the home folder when it holds no links (see
//! [`measure`](super::measure)).

use std::fs;

use super::Found;
use crate::model::{Volume, VolumeKind};

/// Where macOS mounts every volume.
const VOLUMES: &str = "/Volumes";

/// The most entries of `/Volumes` looked at.
const MAX_ENTRIES: usize = 256;

/// The system disk and the volumes in `/Volumes`.
pub fn list() -> Vec<Found> {
    let entries: Vec<(String, bool)> = fs::read_dir(VOLUMES)
        .map(|dir| {
            dir.filter_map(Result::ok)
                .take(MAX_ENTRIES)
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());

                    Some((name, is_folder))
                })
                .collect()
        })
        .unwrap_or_default();

    volumes(&entries)
}

/// The system disk (the only one to measure) and the volumes among `entries` (name, whether it is a folder), by name,
/// none of them to measure. A link (the boot volume is one, to `/`) is not a volume.
pub fn volumes(entries: &[(String, bool)]) -> Vec<Found> {
    let mut names: Vec<&str> = entries
        .iter()
        .filter(|(name, is_folder)| *is_folder && is_listed(name))
        .map(|(name, _)| name.as_str())
        .collect();
    names.sort_unstable();

    let system = Found {
        volume: Volume {
            name: "System".to_owned(),
            label: None,
            path: "/".to_owned(),
            kind: VolumeKind::System,
            free: None,
        },
        measure: true,
    };
    // Never measured: it may be a network share, a FUSE volume or an automount (see the module docs).
    let drives = names.into_iter().map(|name| Found {
        volume: Volume {
            name: name.to_owned(),
            label: None,
            path: format!("{VOLUMES}/{name}"),
            kind: VolumeKind::Drive,
            free: None,
        },
        measure: false,
    });

    std::iter::once(system).chain(drives).collect()
}

/// Whether a name in `/Volumes` is for people: not hidden and not one of the system's own (`com.apple.*`).
pub fn is_listed(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.starts_with("com.apple.")
}

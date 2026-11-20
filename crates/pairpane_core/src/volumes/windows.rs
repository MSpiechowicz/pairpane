//! Windows volumes from the OS volume list (`sysinfo`): the local fixed and removable drives only. A mapped network
//! drive is neither listed nor touched. The list comes with each drive's free space, so a volume found here is
//! never measured again; asking the OS for it runs where the caller can afford to lose the thread (see
//! [`LISTING_MAY_BLOCK`](super::LISTING_MAY_BLOCK)).

use super::Found;
use crate::model::{Volume, VolumeKind};

/// The local fixed and removable volumes, with their free space.
#[cfg(windows)]
pub fn list() -> Vec<Found> {
    use sysinfo::{DiskRefreshKind, Disks};

    // Storage only: the volume list and its sizes, no I/O statistics or drive kinds.
    let Ok(disks) = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage()) else {
        return Vec::new();
    };
    let system_drive = std::env::var("SystemDrive").ok();

    disks
        .iter()
        .filter_map(|disk| {
            let mount = disk.mount_point().to_str()?;
            let label = disk.name().to_string_lossy();

            Some(volume(
                mount,
                &label,
                disk.is_removable(),
                disk.available_space(),
                system_drive.as_deref(),
            ))
        })
        .collect()
}

/// A volume found by the OS: where it is mounted (`C:\`, or a folder such as `C:\Mounts\Data\`), its label (may be
/// empty), whether it is removable, its free space and the drive Windows runs from (`C:`). The drive's name is its
/// letter (`C:`) or, for a folder mount, the folder's name; the label goes to the dimmed second line.
pub fn volume(mount: &str, label: &str, removable: bool, free: u64, system_drive: Option<&str>) -> Found {
    let trimmed = mount.trim_end_matches(['\\', '/']);
    let name = if trimmed.ends_with(':') {
        trimmed
    } else {
        trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed)
    };
    let is_system = system_drive.is_some_and(|drive| trimmed.eq_ignore_ascii_case(drive.trim_end_matches(['\\', '/'])));

    let kind = if is_system {
        VolumeKind::System
    } else if removable {
        VolumeKind::Removable
    } else {
        VolumeKind::Drive
    };

    Found {
        volume: Volume {
            name: name.to_owned(),
            label: Some(label.trim().to_owned()).filter(|label| !label.is_empty()),
            path: mount.to_owned(),
            kind,
            free: Some(free),
        },
        measure: false,
    }
}

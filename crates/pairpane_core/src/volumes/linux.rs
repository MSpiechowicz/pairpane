//! Linux volumes from the kernel's mount table.
//!
//! `/proc/self/mounts` is produced by the kernel from its own records; reading it touches no mounted filesystem,
//! so a dead network mount cannot stall it. Which mounts are worth showing is decided from the table alone: the
//! filesystem type, the source and the mount point. Everything here works on text, so the rules are tested on
//! every system.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

use super::{Found, home_volume};
use crate::model::{Volume, VolumeKind};

/// The mount table of the process.
const MOUNTS_FILE: &str = "/proc/self/mounts";

/// The most of the table that is read, in bytes.
const MAX_TABLE_BYTES: u64 = 4 * 1_048_576;

/// Filesystems that reach over the network. Listed as network volumes wherever they are mounted, and never asked for
/// free space (that would contact the server, and wait for it for as long as it takes).
const NETWORK_TYPES: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "9p",
    "ceph",
    "glusterfs",
    "davfs",
    "fuse.sshfs",
    "fuse.rclone",
    "afs",
    "lustre",
    "fuse.s3fs",
    "smb",
    "ncpfs",
    "coda",
    "orangefs",
    "beegfs",
    "gpfs",
    "virtiofs",
];

/// Filesystems that are never places to go: kernel interfaces, memory, read-only images (snap packages), container
/// layers, mounts that wait for a first visit before mounting what they stand for (`autofs`; looking inside one
/// mounts it, which may mean contacting a server) and desktop services (portals, the GNOME virtual filesystem).
const HIDDEN_TYPES: &[&str] = &[
    "autofs",
    "bpf",
    "binfmt_misc",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fusectl",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "hugetlbfs",
    "mqueue",
    "nsfs",
    "overlay",
    "proc",
    "pstore",
    "ramfs",
    "rpc_pipefs",
    "securityfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
];

/// Mount points that belong to the system, not the user: a block device mounted here is not offered as a drive.
const SYSTEM_PATHS: &[&str] = &[
    "/boot", "/dev", "/efi", "/etc", "/proc", "/run", "/snap", "/sys", "/usr", "/var",
];

/// Where desktops mount removable media.
const REMOVABLE_PATHS: &[&str] = &["/run/media", "/media"];

/// One line of the mount table, with its escapes decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub point: String,
    pub fstype: String,
}

/// The volumes of this machine: the system disk, `home` and what the mount table shows.
pub fn list(home: Option<&str>) -> Vec<Found> {
    let mounts = read_table().map(|table| parse(&table)).unwrap_or_default();

    classify(&mounts, home)
}

/// The mount table as text; `None` when it cannot be read.
fn read_table() -> Option<String> {
    let mut bytes = Vec::new();
    File::open(MOUNTS_FILE)
        .and_then(|file| file.take(MAX_TABLE_BYTES).read_to_end(&mut bytes))
        .ok()?;

    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The mounts in a mount table (`source point type options dump pass`, one per line). Lines that are not that, and
/// mounts whose point is not a usable path, are skipped.
pub fn parse(table: &str) -> Vec<Mount> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            let (source, point, fstype) = (fields.next()?, fields.next()?, fields.next()?);
            let point = unescape(point);
            if !point.starts_with('/') || point.contains(['\0', '\u{FFFD}']) {
                return None;
            }

            Some(Mount {
                source: unescape(source),
                point,
                fstype: fstype.to_owned(),
            })
        })
        .collect()
}

/// A mount table field with the kernel's escapes undone: a space, tab, newline or backslash in a name is written as a
/// backslash and its three-digit octal code (`\040`).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());

    let mut at = 0;
    while at < bytes.len() {
        let code = match bytes[at..] {
            [b'\\', a @ b'0'..=b'3', b @ b'0'..=b'7', c @ b'0'..=b'7', ..] => {
                Some((a - b'0') * 64 + (b - b'0') * 8 + (c - b'0'))
            }
            _ => None,
        };

        match code {
            Some(code) => {
                decoded.push(code);
                at += 4;
            }
            None => {
                decoded.push(bytes[at]);
                at += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// What to offer for these mounts: `/` as System, `home` as Home, and every other mount that is a place to go (see
/// [`volume_of`]). Where several mounts are stacked on one point only the last shows, as it does on disk.
pub fn classify(mounts: &[Mount], home: Option<&str>) -> Vec<Found> {
    let mounts = visible(mounts);
    let mut found = Vec::new();

    let root = mounts.iter().find(|mount| mount.point == "/");
    found.push(Found {
        volume: Volume {
            name: "System".to_owned(),
            label: None,
            path: "/".to_owned(),
            kind: VolumeKind::System,
            free: None,
        },
        measure: root.is_some_and(|root| may_ask(&root.fstype)),
    });

    if let Some(home) = home {
        found.push(home_volume(home, home_may_be_asked(&mounts, home)));
    }

    found.extend(
        mounts
            .iter()
            .filter(|mount| mount.point != "/")
            .filter_map(|mount| volume_of(mount)),
    );

    found
}

/// The mounts that are not covered by a later mount on the same point.
fn visible(mounts: &[Mount]) -> Vec<&Mount> {
    let last: HashMap<&str, usize> = mounts
        .iter()
        .enumerate()
        .map(|(index, mount)| (mount.point.as_str(), index))
        .collect();

    mounts
        .iter()
        .enumerate()
        .filter(|(index, mount)| last[mount.point.as_str()] == *index)
        .map(|(_, mount)| mount)
        .collect()
}

/// The volume to show for a mount that is not `/`, or `None` for one that is no place to go.
///
/// A network filesystem is shown wherever it is. A local one is shown when it is mounted where desktops and users
/// mount media (`/run/media`, `/media`: removable; `/mnt`: a drive), or is a block device outside the system's own
/// folders.
fn volume_of(mount: &Mount) -> Option<Found> {
    let name = mount.point.rsplit('/').next().filter(|name| !name.is_empty())?;
    let volume = |kind, label| Volume {
        name: name.to_owned(),
        label,
        path: mount.point.clone(),
        kind,
        free: None,
    };

    if is_network(&mount.fstype) {
        let label = Some(mount.source.clone()).filter(|source| !source.is_empty());

        return Some(Found {
            volume: volume(VolumeKind::Network, label),
            measure: false,
        });
    }
    if HIDDEN_TYPES.contains(&mount.fstype.as_str()) {
        return None;
    }

    let kind = if is_under_any(&mount.point, REMOVABLE_PATHS) {
        VolumeKind::Removable
    } else if is_under(&mount.point, "/mnt") || is_user_block_device(mount) {
        VolumeKind::Drive
    } else {
        return None;
    };

    Some(Found {
        volume: volume(kind, None),
        measure: may_ask(&mount.fstype),
    })
}

/// A block device mounted outside the system's folders. Never a user-space filesystem: its `source` is whatever the
/// driver says (`fsname=/dev/sda1`), so it proves nothing about a device.
fn is_user_block_device(mount: &Mount) -> bool {
    !is_fuse(&mount.fstype)
        && mount.source.starts_with("/dev/")
        && !SYSTEM_PATHS.iter().any(|system| is_at_or_under(&mount.point, system))
}

fn is_network(fstype: &str) -> bool {
    NETWORK_TYPES.contains(&fstype)
}

/// A user-space filesystem: `fuse` or `fuse.<name>`. Not `fuseblk`, the kernel name of a block device filesystem such
/// as NTFS or exFAT, which is local.
fn is_fuse(fstype: &str) -> bool {
    fstype == "fuse" || fstype.starts_with("fuse.")
}

/// Whether the filesystem may be asked for free space: a local one. Not a network one, not `autofs` (asking mounts
/// what it stands for) and not a user-space one (see [`is_fuse`]): a driver that hangs, like a stalled sshfs, leaves
/// the asking call stuck with no way to stop it.
fn may_ask(fstype: &str) -> bool {
    !is_network(fstype) && fstype != "autofs" && !is_fuse(fstype)
}

/// Whether the home folder's disk may be asked: the mount that holds it (the deepest one containing it) is local. A
/// home folder on a network or `autofs` mount, or on one the table does not show, is never asked.
fn home_may_be_asked(mounts: &[&Mount], home: &str) -> bool {
    mounts
        .iter()
        .filter(|mount| is_at_or_under(home, &mount.point))
        .max_by_key(|mount| mount.point.len())
        .is_some_and(|mount| may_ask(&mount.fstype))
}

/// Whether `path` is the folder `base` or inside it (`/mnt` is a base of `/mnt/x`, not of `/mnt2`).
fn is_at_or_under(path: &str, base: &str) -> bool {
    base == "/" || path == base || is_under(path, base)
}

/// Whether `path` is inside `base`, not `base` itself.
fn is_under(path: &str, base: &str) -> bool {
    path.strip_prefix(base).is_some_and(|rest| rest.starts_with('/'))
}

fn is_under_any(path: &str, bases: &[&str]) -> bool {
    bases.iter().any(|base| is_under(path, base))
}

#[cfg(test)]
mod tests;

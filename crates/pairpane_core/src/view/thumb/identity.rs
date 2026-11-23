//! What tells one file from another that took its place, for a cache that must not serve the picture of the old one.

use std::fs::{File, Metadata};

/// Which file this is, as the system names it: the device and the file number, and on Unix the time of its last change
/// (it moves on a rename over the file, a rewrite and a change of owner; unlike the modification time it cannot be set
/// back by the program that replaced the file). Two spellings of a path that lead to one file have one identity, and a
/// file replaced by another of the same length and modification time has another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    device: u64,
    file: u64,
    changed: (i64, i64),
}

impl Identity {
    /// The identity of the open `file`, whose metadata is `meta` (read from the same handle). `None` where the system
    /// does not tell (the file is then known by its length and modification time only).
    #[cfg(unix)]
    pub(super) fn of(_file: &File, meta: &Metadata) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;

        Some(Self {
            device: meta.dev(),
            file: meta.ino(),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }

    /// The volume serial number and the file index of the handle.
    #[cfg(windows)]
    pub(super) fn of(file: &File, _meta: &Metadata) -> Option<Self> {
        let info = winapi_util::file::information(file).ok()?;

        Some(Self {
            device: info.volume_serial_number(),
            file: info.file_index(),
            changed: (0, 0),
        })
    }

    #[cfg(not(any(unix, windows)))]
    pub(super) fn of(_file: &File, _meta: &Metadata) -> Option<Self> {
        None
    }
}

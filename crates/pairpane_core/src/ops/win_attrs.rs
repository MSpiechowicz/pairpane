//! The Windows file attributes (read-only, hidden, system, archive) of an item, set through its handle.
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library sets the read-only attribute alone, and the others need `SetFileInformationByHandle`. The call is
//! made on a handle the caller opened without following links, never by path (`SetFileAttributesW` would follow what a
//! path names). What to set is worked out by [`Wanted::apply`], which is safe and is tested on every system.
#![allow(unsafe_code, reason = "the one call of SetFileInformationByHandle; see `write`")]
#![deny(clippy::undocumented_unsafe_blocks)]

use crate::model::AttributeChange;

/// Windows `FILE_ATTRIBUTE_READONLY`.
const READ_ONLY: u32 = 0x1;

/// Windows `FILE_ATTRIBUTE_HIDDEN`.
const HIDDEN: u32 = 0x2;

/// Windows `FILE_ATTRIBUTE_SYSTEM`.
const SYSTEM: u32 = 0x4;

/// Windows `FILE_ATTRIBUTE_ARCHIVE`.
const ARCHIVE: u32 = 0x20;

/// Windows `FILE_ATTRIBUTE_NORMAL`: no other attribute is set. Writing zero would mean "leave the attributes alone".
#[cfg(any(windows, test))]
const NORMAL: u32 = 0x80;

/// Windows `FILE_ATTRIBUTE_TEMPORARY`.
const TEMPORARY: u32 = 0x100;

/// Windows `FILE_ATTRIBUTE_OFFLINE`.
const OFFLINE: u32 = 0x1000;

/// Windows `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED`.
const NOT_CONTENT_INDEXED: u32 = 0x2000;

/// Windows `FILE_ATTRIBUTE_NO_SCRUB_DATA` (ReFS and Storage Spaces).
const NO_SCRUB_DATA: u32 = 0x2_0000;

/// Windows `FILE_ATTRIBUTE_PINNED`: a cloud file (OneDrive) kept on this device.
const PINNED: u32 = 0x8_0000;

/// Windows `FILE_ATTRIBUTE_UNPINNED`: a cloud file the system may free up.
const UNPINNED: u32 = 0x10_0000;

/// The attributes a handle can set, all written back as they are unless the change names them (a missing one would be
/// cleared: a OneDrive pin would be lost). The others (folder, link, compressed, encrypted, sparse and so on) are facts
/// of the item that this call does not change, so they are never written back.
const SETTABLE: u32 = READ_ONLY
    | HIDDEN
    | SYSTEM
    | ARCHIVE
    | TEMPORARY
    | OFFLINE
    | NOT_CONTENT_INDEXED
    | NO_SCRUB_DATA
    | PINNED
    | UNPINNED;

/// The attributes a change asks for: `None` leaves one as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Wanted {
    read_only: Option<bool>,
    hidden: Option<bool>,
    system: Option<bool>,
    archive: Option<bool>,
}

impl Wanted {
    /// What `change` asks of an item. Read-only is for files only; a folder takes the other three.
    pub(super) fn of(change: &AttributeChange, is_file: bool) -> Self {
        Self {
            read_only: if is_file { change.read_only } else { None },
            hidden: change.hidden,
            system: change.system,
            archive: change.archive,
        }
    }

    /// Whether nothing is asked.
    pub(super) fn is_empty(self) -> bool {
        self.read_only.is_none() && self.hidden.is_none() && self.system.is_none() && self.archive.is_none()
    }

    /// The attributes to write to an item that has `current` now, or `None` when it has what is asked already. Only the
    /// attributes of [`SETTABLE`] are kept, and no attribute at all is written as `NORMAL`.
    pub(super) fn apply(self, current: u32) -> Option<u32> {
        let mut attributes = current & SETTABLE;

        for (bit, wanted) in [
            (READ_ONLY, self.read_only),
            (HIDDEN, self.hidden),
            (SYSTEM, self.system),
            (ARCHIVE, self.archive),
        ] {
            match wanted {
                Some(true) => attributes |= bit,
                Some(false) => attributes &= !bit,
                None => {}
            }
        }

        if attributes == current & SETTABLE {
            return None;
        }

        Some(if attributes == 0 { NORMAL } else { attributes })
    }
}

/// Gives the item behind `file` the attributes `wanted` asks for, if it has others now. `file` is a handle opened for
/// attributes without following links (see `open::attributes`); `current` is what it has.
#[cfg(windows)]
pub(super) fn set(file: &std::fs::File, current: u32, wanted: Wanted) -> std::io::Result<()> {
    match wanted.apply(current) {
        Some(attributes) => write(file, attributes),
        None => Ok(()),
    }
}

/// Writes `attributes` to the item behind `file`; its times are left alone (zero means unchanged).
#[cfg(windows)]
fn write(file: &std::fs::File, attributes: u32) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{FILE_BASIC_INFO, FileBasicInfo, SetFileInformationByHandle};

    let info = FILE_BASIC_INFO {
        CreationTime: 0,
        LastAccessTime: 0,
        LastWriteTime: 0,
        ChangeTime: 0,
        FileAttributes: attributes,
    };
    let size = size_of::<FILE_BASIC_INFO>() as u32;

    // SAFETY: `file` is an open handle for the whole call, and `AsRawHandle` gives the same kind of handle that
    // `HANDLE` is. `info` is a fully initialised `FILE_BASIC_INFO` (the `repr(C)` struct that class `FileBasicInfo`
    // reads) on this stack frame, and `size` is its exact size, so the call reads only inside it and keeps no pointer
    // after it returns. A failure is reported by a zero result and nothing else is touched.
    let done =
        unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileBasicInfo, (&raw const info).cast(), size) };

    if done == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn wanted(read_only: Option<bool>, hidden: Option<bool>, system: Option<bool>, archive: Option<bool>) -> Wanted {
        Wanted {
            read_only,
            hidden,
            system,
            archive,
        }
    }

    #[test]
    fn each_attribute_is_set_and_cleared_alone() {
        assert_eq!(
            wanted(None, Some(true), None, None).apply(ARCHIVE),
            Some(ARCHIVE | HIDDEN)
        );
        assert_eq!(
            wanted(None, None, Some(true), None).apply(HIDDEN),
            Some(HIDDEN | SYSTEM)
        );
        assert_eq!(
            wanted(Some(true), None, None, None).apply(HIDDEN),
            Some(HIDDEN | READ_ONLY)
        );
        assert_eq!(
            wanted(None, Some(false), None, None).apply(HIDDEN | ARCHIVE),
            Some(ARCHIVE)
        );
        assert_eq!(
            wanted(Some(true), Some(false), Some(true), Some(false)).apply(HIDDEN | ARCHIVE),
            Some(READ_ONLY | SYSTEM)
        );
    }

    #[test]
    fn what_the_item_has_already_is_not_written() {
        assert_eq!(wanted(None, None, None, None).apply(HIDDEN), None);
        assert_eq!(wanted(None, Some(true), None, None).apply(HIDDEN | ARCHIVE), None);
        assert_eq!(wanted(Some(false), Some(false), None, None).apply(ARCHIVE), None);
    }

    #[test]
    fn a_cloud_pin_and_no_scrub_are_kept() {
        assert_eq!(
            wanted(None, Some(true), None, None).apply(PINNED | ARCHIVE),
            Some(PINNED | ARCHIVE | HIDDEN)
        );
        assert_eq!(
            wanted(None, None, None, Some(false)).apply(UNPINNED | NO_SCRUB_DATA | ARCHIVE),
            Some(UNPINNED | NO_SCRUB_DATA)
        );
    }

    #[test]
    fn no_attribute_at_all_is_written_as_normal() {
        assert_eq!(wanted(None, None, None, Some(false)).apply(ARCHIVE), Some(NORMAL));
        assert_eq!(wanted(None, Some(false), None, None).apply(HIDDEN), Some(NORMAL));
    }

    #[test]
    fn the_facts_of_an_item_are_never_written_back() {
        // Folder (0x10), compressed (0x800) and reparse point (0x400) are not for this call to set.
        let folder = 0x10 | 0x800 | 0x400;

        assert_eq!(
            wanted(None, Some(true), None, None).apply(folder | ARCHIVE),
            Some(ARCHIVE | HIDDEN)
        );
        assert_eq!(
            wanted(None, Some(false), None, None).apply(folder | HIDDEN),
            Some(NORMAL)
        );
        assert_eq!(wanted(None, Some(true), None, None).apply(folder | HIDDEN), None);
        assert_eq!(
            wanted(None, Some(true), None, None).apply(OFFLINE | NOT_CONTENT_INDEXED | TEMPORARY),
            Some(OFFLINE | NOT_CONTENT_INDEXED | TEMPORARY | HIDDEN)
        );
    }

    #[test]
    fn a_folder_is_not_asked_for_read_only() {
        let change = AttributeChange {
            mode: None,
            read_only: Some(true),
            hidden: Some(true),
            system: None,
            archive: None,
            modified: None,
            accessed: None,
            recursive: false,
        };

        assert_eq!(Wanted::of(&change, true), wanted(Some(true), Some(true), None, None));
        assert_eq!(Wanted::of(&change, false), wanted(None, Some(true), None, None));
        assert!(Wanted::of(&AttributeChange { hidden: None, ..change }, false).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn the_bits_are_the_ones_windows_names() {
        use windows_sys::Win32::Storage::FileSystem as fs;

        assert_eq!(READ_ONLY, fs::FILE_ATTRIBUTE_READONLY);
        assert_eq!(HIDDEN, fs::FILE_ATTRIBUTE_HIDDEN);
        assert_eq!(SYSTEM, fs::FILE_ATTRIBUTE_SYSTEM);
        assert_eq!(ARCHIVE, fs::FILE_ATTRIBUTE_ARCHIVE);
        assert_eq!(NORMAL, fs::FILE_ATTRIBUTE_NORMAL);
        assert_eq!(TEMPORARY, fs::FILE_ATTRIBUTE_TEMPORARY);
        assert_eq!(OFFLINE, fs::FILE_ATTRIBUTE_OFFLINE);
        assert_eq!(NOT_CONTENT_INDEXED, fs::FILE_ATTRIBUTE_NOT_CONTENT_INDEXED);
        assert_eq!(NO_SCRUB_DATA, fs::FILE_ATTRIBUTE_NO_SCRUB_DATA);
        assert_eq!(PINNED, fs::FILE_ATTRIBUTE_PINNED);
        assert_eq!(UNPINNED, fs::FILE_ATTRIBUTE_UNPINNED);
    }
}

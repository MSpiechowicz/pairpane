//! Files that are not on this computer: a cloud placeholder (OneDrive on Windows, iCloud on macOS) holds a name and a
//! size, and reading it, even its first bytes, makes the system download the whole file. Scrolling a folder must not do
//! that, so such a file is neither read for a size nor hashed.
//!
//! The decision is a pure function of the attribute word of the system, so it is tested on every system; only the read
//! of that word (the handle's own metadata, nothing is read of the file) is specific to Windows and macOS. On any other
//! system there are no such files and the words are zero.

use std::fs::Metadata;

/// Windows `FILE_ATTRIBUTE_OFFLINE`: the data is not available right away (moved to remote storage).
const WINDOWS_OFFLINE: u32 = 0x0000_1000;

/// Windows `FILE_ATTRIBUTE_RECALL_ON_OPEN`: opening the file brings its data.
const WINDOWS_RECALL_ON_OPEN: u32 = 0x0004_0000;

/// Windows `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: reading the file brings its data (a OneDrive "online only" file).
const WINDOWS_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// macOS `SF_DATALESS`: the file is a placeholder whose data is in iCloud.
const MACOS_DATALESS: u32 = 0x4000_0000;

/// Whether the Windows attributes `attributes` are those of a file whose data is not on this computer.
pub(super) fn is_offline(attributes: u32) -> bool {
    attributes & (WINDOWS_OFFLINE | WINDOWS_RECALL_ON_OPEN | WINDOWS_RECALL_ON_DATA_ACCESS) != 0
}

/// Whether the macOS file flags `flags` (`st_flags`) are those of a dataless file.
pub(super) fn is_dataless(flags: u32) -> bool {
    flags & MACOS_DATALESS != 0
}

/// Whether the file `meta` describes is a placeholder of the system's cloud storage. Only the metadata is looked at.
pub(super) fn is_placeholder(meta: &Metadata) -> bool {
    is_offline(windows_attributes(meta)) || is_dataless(macos_flags(meta))
}

#[cfg(windows)]
fn windows_attributes(meta: &Metadata) -> u32 {
    std::os::windows::fs::MetadataExt::file_attributes(meta)
}

#[cfg(not(windows))]
fn windows_attributes(_meta: &Metadata) -> u32 {
    0
}

#[cfg(target_os = "macos")]
fn macos_flags(meta: &Metadata) -> u32 {
    std::os::macos::fs::MetadataExt::st_flags(meta)
}

#[cfg(not(target_os = "macos"))]
fn macos_flags(_meta: &Metadata) -> u32 {
    0
}

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{ArchiveSpot, FindId, IpcError};

/// Which of the two panes a command is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PaneId {
    Left,
    Right,
}

/// What an entry is. For a symbolic link: what it points to (`other` when the target is missing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntryKind {
    Dir,
    File,
    /// A device, socket, pipe, or a symbolic link whose target does not exist.
    Other,
}

/// One folder entry as a pane shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Entry {
    /// File name. A name that is not valid Unicode (possible on Linux) shows `�` in place of the invalid bytes.
    pub name: String,
    pub kind: EntryKind,
    /// Size in bytes; 0 for folders.
    #[ts(type = "number")]
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch; `null` when the filesystem does not report it.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    /// Hidden by the platform's rule (dot prefix on Unix, the hidden attribute on Windows).
    pub hidden: bool,
    /// Not writable by its permissions or attributes.
    pub readonly: bool,
    /// Unix permission bits (`0o755`); `null` on Windows.
    pub mode: Option<u32>,
    /// Where a symbolic link points (as written in the link); `null` for anything else.
    pub link_target: Option<String>,
}

/// One step of the path bar: a folder name and the full path to open it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Crumb {
    pub name: String,
    pub path: String,
    /// For a step inside an archive: the folder path inside it (`""` for the archive's root, which is the crumb with the
    /// archive's name); `path` is then the archive's path. Absent for a real folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub inside: Option<String>,
}

/// What a pane's listing channel delivers, in this order: `start`, any number of `batch`es in entry order,
/// then `done` (or `error`). After `done`, `changed` reports that the folder changed on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ListEvent {
    /// The folder was opened; `total` entries follow, sorted (folders first, then by name).
    Start {
        path: String,
        /// The folder above, or `null` at a root.
        parent: Option<String>,
        /// The path bar, from the root down to this folder.
        crumbs: Vec<Crumb>,
        total: u32,
        /// Set when the listing is a folder inside an archive: then `path` is the folder that holds the archive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        archive: Option<ArchiveSpot>,
        /// Set when the listing is the results of a search: then `path` is the folder searched and every entry's name is
        /// its path below it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        results: Option<FindId>,
    },
    /// Entries `offset..offset + entries.len()`.
    Batch { offset: u32, entries: Vec<Entry> },
    /// Every entry was sent.
    Done {
        dirs: u32,
        files: u32,
        /// Sum of the file sizes.
        #[ts(type = "number")]
        bytes: u64,
        /// Space available to the user on the folder's volume, in bytes; `null` when it cannot be read.
        #[ts(type = "number | null")]
        free: Option<u64>,
    },
    /// The folder changed on disk; open it again to see the change.
    Changed,
    /// The folder could not be listed.
    Error { error: IpcError },
}

/// What a folder-size job delivers: any number of `progress` events, then exactly one of `done`, `error` or
/// `cancelled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum SizeEvent {
    /// The bytes counted so far.
    Progress {
        #[ts(type = "number")]
        bytes: u64,
    },
    /// The folder, with everything below it, holds this many bytes of files.
    Done {
        #[ts(type = "number")]
        bytes: u64,
    },
    /// The size could not be calculated.
    Error { error: IpcError },
    /// The job was stopped (the pane moved to another folder, or a newer job replaced it); every job ends with
    /// `done`, `error` or this.
    Cancelled,
}

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::IpcError;

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
    },
    /// The folder changed on disk; open it again to see the change.
    Changed,
    /// The folder could not be listed.
    Error { error: IpcError },
}

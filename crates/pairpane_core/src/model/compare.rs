use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{EntryKind, IpcError, PaneId};

/// Identifies one folder comparison. Monotonic within a run and never reused; a `number` in TypeScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompareId(#[ts(type = "number")] pub u64);

/// One side of a comparison: a folder on disk, or a folder inside an archive (as [`super::ArchiveSpot`] names it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CompareRoot {
    /// A folder, absolute.
    Folder { path: String },
    /// A folder inside an archive: `dir` is `""` for the archive's root, otherwise `a/b`.
    Archive { archive: String, dir: String },
}

/// What a comparison looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompareOptions {
    /// Go into the folders both sides have.
    pub subfolders: bool,
    /// Files of the same size are read and compared by content: the same bytes make them the same, whatever their times say (not with an
    /// archive side).
    pub content: bool,
    /// Look at hidden entries too.
    pub hidden: bool,
}

/// How the two sides of one row stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CompareStatus {
    /// Only the left side has it.
    LeftOnly,
    /// Only the right side has it.
    RightOnly,
    /// Both have it; the left one is newer.
    LeftNewer,
    /// Both have it; the right one is newer.
    RightNewer,
    /// Both have it and they differ, with nothing to say which is newer.
    Differs,
    /// Both have it and they are the same.
    Equal,
}

/// What one side holds at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompareFacts {
    /// For a link: `other` (a link is never followed).
    pub kind: EntryKind,
    /// A symbolic link.
    pub link: bool,
    /// Size in bytes; 0 for folders.
    #[ts(type = "number")]
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    /// For a folder only one side has: the files in its whole subtree; otherwise 0.
    #[ts(type = "number")]
    pub files: u64,
    /// For a folder only one side has: the bytes of those files; otherwise 0.
    #[ts(type = "number")]
    pub bytes: u64,
}

/// One path of the comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompareRow {
    /// The path below the compared folders, names joined with `/`.
    pub path: String,
    pub left: Option<CompareFacts>,
    pub right: Option<CompareFacts>,
    pub status: CompareStatus,
}

/// What a comparison delivers: any number of `rows` and `progress` events, then exactly one of `done` and `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CompareEvent {
    /// More rows, in no particular order.
    Rows { rows: Vec<CompareRow> },
    /// What was looked at so far. While files are compared by content, `bytes` counts the bytes read.
    Progress {
        #[ts(type = "number")]
        folders: u64,
        #[ts(type = "number")]
        files: u64,
        #[ts(type = "number")]
        bytes: u64,
    },
    /// The comparison ended.
    Done {
        /// The rows delivered.
        #[ts(type = "number")]
        rows: u64,
        /// What could not be compared: unreadable entries, names that cannot be used, names that fold together.
        #[ts(type = "number")]
        skipped: u64,
        /// It stopped at a limit and there is more.
        truncated: bool,
        /// It was stopped.
        cancelled: bool,
    },
    /// The comparison could not run.
    Error { error: IpcError },
}

/// Which way a synchronized item is copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SyncDirection {
    /// From the left side to the right one.
    ToRight,
    /// From the right side to the left one.
    ToLeft,
}

/// What the scan saw of one side of an item; the job copies only while it still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SyncFacts {
    pub kind: EntryKind,
    /// Size in bytes; 0 for folders.
    #[ts(type = "number")]
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}

/// One copy of a synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SyncItem {
    /// The path below the two folders, names joined with `/`.
    pub path: String,
    pub direction: SyncDirection,
    /// The side that is copied from, as the scan saw it.
    pub source: SyncFacts,
    /// The side that is copied onto, as the scan saw it; `null` when it was not there.
    pub target: Option<SyncFacts>,
}

/// One removal of a synchronization (a mirror): an item only one side has, removed from that side so that it matches the
/// other. The job removes it only while it is still what the scan saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SyncRemoval {
    /// The path below the two folders, names joined with `/`.
    pub path: String,
    /// The side the item is removed from.
    pub side: PaneId,
    /// The item as the scan saw it.
    pub seen: SyncFacts,
    /// For a folder: the files in its whole subtree, as the scan counted them; 0 for a file.
    #[ts(type = "number")]
    pub files: u64,
    /// For a folder: the bytes of those files; 0 for a file.
    #[ts(type = "number")]
    pub bytes: u64,
    /// When the scan began, in milliseconds since the Unix epoch: a folder with anything newer in it is kept.
    #[ts(type = "number")]
    pub scanned: i64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn facts(kind: EntryKind, size: u64, modified: Option<i64>) -> CompareFacts {
        CompareFacts {
            kind,
            link: false,
            size,
            modified,
            files: 0,
            bytes: 0,
        }
    }

    #[test]
    fn roots_are_tagged_by_kind() {
        let folder = CompareRoot::Folder { path: "/a".into() };
        let archive = CompareRoot::Archive {
            archive: "/a.zip".into(),
            dir: "d".into(),
        };

        assert_eq!(
            serde_json::to_value(&folder).expect("serializable"),
            json!({ "kind": "folder", "path": "/a" })
        );
        assert_eq!(
            serde_json::to_value(&archive).expect("serializable"),
            json!({ "kind": "archive", "archive": "/a.zip", "dir": "d" })
        );
        assert_eq!(
            serde_json::from_value::<CompareRoot>(json!({ "kind": "archive", "archive": "/a.zip", "dir": "" }))
                .expect("deserializable"),
            CompareRoot::Archive {
                archive: "/a.zip".into(),
                dir: String::new()
            }
        );
    }

    #[test]
    fn options_and_statuses_use_camel_case() {
        let options = CompareOptions {
            subfolders: true,
            content: false,
            hidden: true,
        };

        assert_eq!(
            serde_json::to_value(options).expect("serializable"),
            json!({ "subfolders": true, "content": false, "hidden": true })
        );
        assert_eq!(
            serde_json::to_value(CompareStatus::LeftNewer).expect("serializable"),
            json!("leftNewer")
        );
        assert_eq!(
            serde_json::from_value::<CompareStatus>(json!("rightOnly")).expect("deserializable"),
            CompareStatus::RightOnly
        );
    }

    #[test]
    fn rows_round_trip_with_one_side_missing() {
        let row = CompareRow {
            path: "a/b.txt".into(),
            left: Some(facts(EntryKind::File, 12, Some(1_700_000_000_000))),
            right: None,
            status: CompareStatus::LeftOnly,
        };

        let value = serde_json::to_value(&row).expect("serializable");

        assert_eq!(
            value,
            json!({
                "path": "a/b.txt",
                "left": { "kind": "file", "link": false, "size": 12, "modified": 1_700_000_000_000_i64, "files": 0, "bytes": 0 },
                "right": null,
                "status": "leftOnly",
            })
        );
        assert_eq!(
            serde_json::from_value::<CompareRow>(value).expect("deserializable"),
            row
        );
    }

    #[test]
    fn events_are_tagged_and_camel_case() {
        let done = CompareEvent::Done {
            rows: 3,
            skipped: 1,
            truncated: false,
            cancelled: true,
        };
        let progress = CompareEvent::Progress {
            folders: 2,
            files: 5,
            bytes: 9,
        };

        assert_eq!(
            serde_json::to_value(&done).expect("serializable"),
            json!({ "kind": "done", "rows": 3, "skipped": 1, "truncated": false, "cancelled": true })
        );
        assert_eq!(
            serde_json::to_value(&progress).expect("serializable"),
            json!({ "kind": "progress", "folders": 2, "files": 5, "bytes": 9 })
        );
        assert_eq!(
            serde_json::from_value::<CompareEvent>(json!({ "kind": "rows", "rows": [] })).expect("deserializable"),
            CompareEvent::Rows { rows: Vec::new() }
        );
    }

    #[test]
    fn sync_items_round_trip() {
        let item = SyncItem {
            path: "dir/new.txt".into(),
            direction: SyncDirection::ToLeft,
            source: SyncFacts {
                kind: EntryKind::File,
                size: 4,
                modified: Some(10),
            },
            target: None,
        };

        let value = serde_json::to_value(&item).expect("serializable");

        assert_eq!(
            value,
            json!({
                "path": "dir/new.txt",
                "direction": "toLeft",
                "source": { "kind": "file", "size": 4, "modified": 10 },
                "target": null,
            })
        );
        assert_eq!(serde_json::from_value::<SyncItem>(value).expect("deserializable"), item);
        assert_eq!(
            serde_json::to_value(SyncDirection::ToRight).expect("serializable"),
            json!("toRight")
        );
    }

    #[test]
    fn the_sync_job_kind_is_named_sync() {
        assert_eq!(
            serde_json::to_value(super::super::OpKind::Sync).expect("serializable"),
            json!("sync")
        );
    }
}

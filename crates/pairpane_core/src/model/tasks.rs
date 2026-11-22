use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::EntryKind;

/// Which checksum a checksum file holds. The names are the file extensions the checksum files use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HashAlgorithm {
    Sha256,
    Blake3,
    Md5,
}

/// How a change of permissions treats the Unix permission bits (`0o755`; the masks stay within `0o7777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ModeChange {
    /// Every item gets exactly these bits.
    Set { mode: u32 },
    /// Every item keeps its bits except those of `remove` (cleared) and `add` (set); `add` wins over `remove`.
    Adjust { add: u32, remove: u32 },
}

/// What the "change attributes" job does to each item. A `null` field leaves that attribute alone; at least one
/// must be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttributeChange {
    /// The Unix permission bits; refused on Windows.
    pub mode: Option<ModeChange>,
    /// The Windows read-only attribute of files; refused on Unix.
    pub read_only: Option<bool>,
    /// The Windows hidden attribute of files and folders; refused on Unix.
    pub hidden: Option<bool>,
    /// The Windows system attribute of files and folders; refused on Unix.
    pub system: Option<bool>,
    /// The Windows archive attribute of files and folders; refused on Unix.
    pub archive: Option<bool>,
    /// The new modification time, in milliseconds since the Unix epoch.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    /// The new access time, in milliseconds since the Unix epoch.
    #[ts(type = "number | null")]
    pub accessed: Option<i64>,
    /// Also change what is inside the selected folders.
    pub recursive: bool,
}

impl AttributeChange {
    /// Whether any Windows attribute (read-only, hidden, system or archive) is to be changed.
    #[must_use]
    pub fn sets_windows_attributes(&self) -> bool {
        self.read_only.is_some() || self.hidden.is_some() || self.system.is_some() || self.archive.is_some()
    }

    /// Whether nothing at all is to be changed (`recursive` alone changes nothing).
    #[must_use]
    pub fn changes_nothing(&self) -> bool {
        self.mode.is_none() && !self.sets_windows_attributes() && self.modified.is_none() && self.accessed.is_none()
    }
}

/// A job other than a copy or a move, as the `ops_task` command takes it. The folder the items are in and their
/// names come with the command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum JobTask {
    /// Writes the checksum file `output` (a plain file name) into the folder of the items, listing every file in the
    /// items, folders included.
    Checksum { algorithm: HashAlgorithm, output: String },
    /// Checks the files a checksum file lists (the one item) against it.
    VerifyChecksum,
    /// Cuts the file (the one item) into parts of `partSize` bytes in the folder `to`, and writes its `.crc` file.
    #[serde(rename_all = "camelCase")]
    Split {
        to: String,
        #[ts(type = "number")]
        part_size: u64,
    },
    /// Joins the parts of a file (the one item, its `.001` part) into the folder `to`.
    Combine { to: String },
    /// Changes permissions (or the Windows attributes) and times.
    Attributes { change: AttributeChange },
    /// Renames the items (the names of the command, the sources): `targets[i]` is the new name of the item `i`.
    Rename { targets: Vec<String> },
}

/// What the Windows file attributes of an item say (`null` for the whole group on other systems).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WindowsAttributes {
    pub read_only: bool,
    pub hidden: bool,
    pub system: bool,
    pub archive: bool,
}

/// What the Properties dialog shows about one item. A link shows what it leads to (as the panes do).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ItemProperties {
    pub name: String,
    pub kind: EntryKind,
    /// Size in bytes; 0 for folders (the pane measures those).
    #[ts(type = "number")]
    pub size: u64,
    /// Milliseconds since the Unix epoch; `null` when the filesystem does not report it.
    #[ts(type = "number | null")]
    pub created: Option<i64>,
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    #[ts(type = "number | null")]
    pub accessed: Option<i64>,
    /// Unix permission bits (`0o755`); `null` on Windows.
    pub mode: Option<u32>,
    /// The owner and the group, by name, or by number when the system knows no name; `null` on Windows.
    pub owner: Option<String>,
    pub group: Option<String>,
    /// `null` on systems other than Windows.
    pub windows: Option<WindowsAttributes>,
    /// Where a symbolic link points, as written in the link; `null` for anything else.
    pub link_target: Option<String>,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn a_task_is_tagged_by_kind_with_camel_case_fields() {
        let split = JobTask::Split {
            to: "/out".to_owned(),
            part_size: 1_457_664,
        };
        let checksum = JobTask::Checksum {
            algorithm: HashAlgorithm::Blake3,
            output: "a.blake3".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(split).expect("serializable"),
            json!({ "kind": "split", "to": "/out", "partSize": 1_457_664 })
        );
        assert_eq!(
            serde_json::to_value(checksum).expect("serializable"),
            json!({ "kind": "checksum", "algorithm": "blake3", "output": "a.blake3" })
        );
        assert_eq!(
            serde_json::to_value(JobTask::VerifyChecksum).expect("serializable"),
            json!({ "kind": "verifyChecksum" })
        );
    }

    #[test]
    fn attribute_changes_read_what_the_webview_sends() {
        let sent = json!({
            "kind": "attributes",
            "change": {
                "mode": { "kind": "adjust", "add": 8, "remove": 2 },
                "readOnly": null, "hidden": true, "system": null, "archive": false,
                "modified": -5, "accessed": null, "recursive": true
            }
        });

        let task: JobTask = serde_json::from_value(sent).expect("a task");

        assert_eq!(
            task,
            JobTask::Attributes {
                change: AttributeChange {
                    mode: Some(ModeChange::Adjust { add: 8, remove: 2 }),
                    read_only: None,
                    hidden: Some(true),
                    system: None,
                    archive: Some(false),
                    modified: Some(-5),
                    accessed: None,
                    recursive: true,
                }
            }
        );
    }

    #[test]
    fn properties_are_camel_case() {
        let properties = ItemProperties {
            name: "a".to_owned(),
            kind: EntryKind::File,
            size: 3,
            created: None,
            modified: Some(7),
            accessed: None,
            mode: Some(0o644),
            owner: Some("me".to_owned()),
            group: None,
            windows: Some(WindowsAttributes {
                read_only: true,
                hidden: false,
                system: false,
                archive: true,
            }),
            link_target: None,
        };

        assert_eq!(
            serde_json::to_value(properties).expect("serializable"),
            json!({
                "name": "a", "kind": "file", "size": 3, "created": null, "modified": 7, "accessed": null,
                "mode": 420, "owner": "me", "group": null,
                "windows": { "readOnly": true, "hidden": false, "system": false, "archive": true },
                "linkTarget": null
            })
        );
    }

    #[test]
    fn algorithms_and_mode_changes_are_camel_case() {
        assert_eq!(
            serde_json::to_value(HashAlgorithm::Sha256).expect("serializable"),
            json!("sha256")
        );
        assert_eq!(
            serde_json::to_value(HashAlgorithm::Md5).expect("serializable"),
            json!("md5")
        );
        assert_eq!(
            serde_json::to_value(ModeChange::Set { mode: 0o755 }).expect("serializable"),
            json!({ "kind": "set", "mode": 493 })
        );
    }
}

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{IpcError, JobRemote};

/// Identifies one copy or move job. Monotonic within a run and never reused; a `number` in TypeScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobId(#[ts(type = "number")] pub u64);

/// What a job does with its items. A move deletes a source item only after its copy is committed. The kinds after
/// `move` work in place (the job's `to` is its own folder) or, for `split` and `combine`, write into the target folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OpKind {
    Copy,
    Move,
    /// Writes a checksum file for the items.
    Checksum,
    /// Checks the files a checksum file lists.
    VerifyChecksum,
    /// Cuts a file into parts.
    Split,
    /// Joins the parts of a file.
    Combine,
    /// Changes permissions and times.
    Attributes,
    /// Extracts entries of an archive into the target folder.
    Extract,
    /// Writes the items into a new archive.
    Pack,
    /// Writes the items into an archive that exists, by writing a new one beside it and replacing the old.
    AddToArchive,
    /// Renames items of one folder, each to a new name.
    Rename,
    /// Copies what differs between two folders, each item the way the synchronization chose.
    Sync,
    /// Puts back what a synchronization did to its folders.
    SyncUndo,
}

/// Where a running job is. Runs in this order; `waiting` and `cancelling` can interrupt the later steps.
/// Pausing is not a phase: it is [`JobView::paused`], and the phase stays where it was.
///
/// A job that finished has no phase; it ends with [`OpsEvent::Ended`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobPhase {
    /// Waiting for the worker of its destination volume to become free.
    Queued,
    /// Resolving the paths and checking that nothing is copied into itself.
    Preparing,
    /// A move trying a plain rename of each item (no bytes are copied when it works).
    Renaming,
    /// Counting the files and bytes that still need copying.
    Scanning,
    Copying,
    /// Re-reading both files to compare their hashes (only when the job asked to verify).
    Verifying,
    /// Reading the files to compute their checksums (checksum and verify jobs).
    Hashing,
    /// Writing the parts of a split file.
    Splitting,
    /// Writing the file a combine joins from its parts.
    Combining,
    /// Setting permissions and times.
    Changing,
    /// Reading an archive and writing the entries it holds (extract jobs).
    Extracting,
    /// Reading the items and writing them into a new archive (pack jobs) or into the rewritten one (add jobs).
    Packing,
    /// Removing the items a synchronization mirrors away (to the Trash, or for good).
    Removing,
    /// Stopped on a conflict question until [`Answer`] arrives (see [`OpsEvent::Conflict`]).
    Waiting,
    /// Cancel was requested; the job is stopping and removing what it created.
    Cancelling,
}

/// One job as the Transfers panel shows it, and as a snapshot carries it. Counters are zero until the job is
/// `scanning`, and `totalBytes` and `totalFiles` are final once it is `copying`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobView {
    pub id: JobId,
    pub kind: OpKind,
    /// What the row calls the source: the item's name for a single item, otherwise `3 items`.
    pub label: String,
    /// The folder the items come from; for one on a server `label: dir` (see `remote`).
    pub from: String,
    /// The folder the items go into; for one on a server `label: dir` (see `remote`).
    pub to: String,
    /// How many top-level items the job was started with.
    pub items: u32,
    /// The job re-reads and hashes each file before committing it.
    pub verify: bool,
    pub phase: JobPhase,
    /// Held by the user. The job keeps its place, and its volume's queue waits behind it.
    pub paused: bool,
    #[ts(type = "number")]
    pub done_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub done_files: u32,
    pub total_files: u32,
    /// The path, relative to the source folder, of the file being copied; `null` when there is none.
    pub current: Option<String>,
    /// Smoothed speed in bytes per second; 0 while paused or before the first sample.
    #[ts(type = "number")]
    pub bytes_per_sec: u64,
    /// Seconds left; `null` while it cannot be estimated (the first second, paused, or no total yet).
    #[ts(type = "number | null")]
    pub eta_secs: Option<u64>,
    /// The server side of a transfer with one (`from` is then the server's folder shown as `label: dir`, or `to` is);
    /// absent for a job between folders of this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remote: Option<JobRemote>,
}

/// What a conflict dialog shows about one side: a file, or a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileFacts {
    /// Size in bytes; 0 for folders.
    #[ts(type = "number")]
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch; `null` when the filesystem does not report it.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
    pub dir: bool,
}

/// A question: the target of `name` already exists. The job waits, with no timeout, until it is answered or
/// cancelled. Answer with the same `job` and `question`; any other number is refused as stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConflictInfo {
    pub job: JobId,
    /// Counts up within the job, from 1.
    pub question: u32,
    /// The item's path relative to the source folder.
    pub name: String,
    pub source: FileFacts,
    pub target: FileFacts,
    /// The first free name for `rename` (`a (2).txt`), the file name only. It is what `rename` will use; the
    /// webview never sends a name of its own.
    pub suggested: String,
    /// Both sides are files on disk, so `compare` can hash them. `false` when the source is an entry of an archive.
    pub comparable: bool,
}

/// The answer to a [`ConflictInfo`]. `compare` is not final: it hashes both files, reports
/// [`OpsEvent::Compared`] and the same question stays open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Answer {
    /// Replace the target. When both are the same file, this skips instead.
    Overwrite,
    Skip,
    /// Copy under the suggested name.
    Rename,
    /// Replace the target only when it was modified earlier than the source, otherwise skip.
    OverwriteOlder,
    Compare,
}

/// What the user asks of a running job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ControlAction {
    /// Stops moving bytes within one chunk.
    Pause,
    Resume,
    /// Stops within one chunk, removes the partial file and the folders the job created; finished files stay.
    Cancel,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Outcome {
    /// Every item was handled; some may have been skipped or failed (see [`OpsEvent::Ended`]).
    Done,
    Cancelled,
    /// The job could not run at all (a path that does not resolve, a folder into itself).
    Failed {
        error: IpcError,
    },
}

/// An item that failed while the job went on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ItemError {
    /// The item's path relative to the source folder.
    pub path: String,
    pub error: IpcError,
}

/// What the transfers channel delivers. `snapshot` comes first, on every subscribe. After that, a job produces
/// `added`, any number of `phase`, `paused`, `progress`, `conflict` and `compared` events, then exactly one
/// `ended`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum OpsEvent {
    /// Every job that has not ended, and every question still waiting for an answer, so a reloaded page shows
    /// them again. The listener replaces what it knew.
    Snapshot {
        jobs: Vec<JobView>,
        questions: Vec<ConflictInfo>,
    },
    /// A job was accepted, queued.
    Added { job: JobView },
    /// The job moved to another phase.
    Phase { id: JobId, phase: JobPhase },
    /// The job was paused or resumed.
    Paused { id: JobId, paused: bool },
    /// At most every 100 ms per job.
    #[serde(rename_all = "camelCase")]
    Progress {
        id: JobId,
        #[ts(type = "number")]
        done_bytes: u64,
        #[ts(type = "number")]
        total_bytes: u64,
        done_files: u32,
        total_files: u32,
        /// The path, relative to the source folder, of the file being copied; `null` when there is none.
        current: Option<String>,
        #[ts(type = "number")]
        bytes_per_sec: u64,
        #[ts(type = "number | null")]
        eta_secs: Option<u64>,
    },
    /// The job waits for an answer to this question.
    Conflict { conflict: ConflictInfo },
    /// The result of `compare` for an open question: whether both files hold the same bytes.
    Compared { id: JobId, question: u32, identical: bool },
    /// The job is over.
    Ended {
        id: JobId,
        outcome: Outcome,
        /// Items left alone: skipped by the user's answer, or not copyable (a pipe, a socket, a device).
        skipped: u32,
        /// The first 100 items that failed.
        errors: Vec<ItemError>,
        /// Failures beyond those in `errors`.
        more_errors: u32,
    },
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::model::SiteId;

    #[test]
    fn ids_are_plain_numbers_and_answers_are_camel_case() {
        assert_eq!(serde_json::to_value(JobId(7)).expect("serializable"), json!(7));
        assert_eq!(
            serde_json::to_value(Answer::OverwriteOlder).expect("serializable"),
            json!("overwriteOlder")
        );
    }

    fn view(remote: Option<JobRemote>) -> JobView {
        JobView {
            id: JobId(5),
            kind: OpKind::Copy,
            label: "a.txt".to_owned(),
            from: "me@host: /srv".to_owned(),
            to: "/home/me".to_owned(),
            items: 1,
            verify: false,
            phase: JobPhase::Queued,
            paused: false,
            done_bytes: 0,
            total_bytes: 0,
            done_files: 0,
            total_files: 0,
            current: None,
            bytes_per_sec: 0,
            eta_secs: None,
            remote,
        }
    }

    #[test]
    fn a_job_view_names_its_server_only_when_it_has_one() {
        let local = json!({
            "id": 5, "kind": "copy", "label": "a.txt", "from": "me@host: /srv", "to": "/home/me", "items": 1,
            "verify": false, "phase": "queued", "paused": false, "doneBytes": 0, "totalBytes": 0, "doneFiles": 0,
            "totalFiles": 0, "current": null, "bytesPerSec": 0, "etaSecs": null
        });
        let mut remote = local.clone();
        remote["remote"] = json!({ "site": 3, "dir": "/srv" });
        let placed = view(Some(JobRemote {
            site: SiteId(3),
            dir: "/srv".to_owned(),
        }));

        assert_eq!(serde_json::to_value(view(None)).expect("serializable"), local);
        assert_eq!(serde_json::to_value(&placed).expect("serializable"), remote);
        assert_eq!(
            serde_json::from_value::<JobView>(local).expect("deserializable"),
            view(None)
        );
        assert_eq!(
            serde_json::from_value::<JobView>(remote).expect("deserializable"),
            placed
        );
    }

    #[test]
    fn events_are_tagged_by_kind_with_camel_case_fields() {
        let progress = OpsEvent::Progress {
            id: JobId(3),
            done_bytes: 10,
            total_bytes: 20,
            done_files: 1,
            total_files: 2,
            current: None,
            bytes_per_sec: 5,
            eta_secs: None,
        };
        let compared = OpsEvent::Compared {
            id: JobId(3),
            question: 2,
            identical: true,
        };

        assert_eq!(
            serde_json::to_value(progress).expect("serializable"),
            json!({
                "kind": "progress", "id": 3, "doneBytes": 10, "totalBytes": 20, "doneFiles": 1,
                "totalFiles": 2, "current": null, "bytesPerSec": 5, "etaSecs": null
            })
        );
        assert_eq!(
            serde_json::to_value(compared).expect("serializable"),
            json!({ "kind": "compared", "id": 3, "question": 2, "identical": true })
        );
    }

    #[test]
    fn a_job_ends_with_a_tagged_outcome() {
        let ended = OpsEvent::Ended {
            id: JobId(1),
            outcome: Outcome::Cancelled,
            skipped: 0,
            errors: Vec::new(),
            more_errors: 0,
        };

        assert_eq!(
            serde_json::to_value(ended).expect("serializable"),
            json!({
                "kind": "ended", "id": 1, "outcome": { "kind": "cancelled" }, "skipped": 0,
                "errors": [], "moreErrors": 0
            })
        );
    }
}

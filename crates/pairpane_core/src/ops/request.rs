//! What a caller hands to the engine: the [`Request`] of a job, the [`Task`] it carries, and why a request or an answer
//! was not accepted.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::archive::Password;
use crate::model::{
    AttributeChange, HashAlgorithm, IpcError, IpcErrorCode, OpKind, PackFormat, PackLevel, PaneId, SyncDirection,
    SyncFacts,
};

use super::{ArchiveJob, RenameLog, SyncLog};

/// What a job does. [`Task::Transfer`] is the copy or move that [`Request::kind`] names; every other task has
/// its own [`OpKind`] (see [`Task::kind`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// A copy or a move.
    Transfer,
    /// Writes the checksum file `output` (a plain file name) into `from`, for all the items.
    Checksum { algorithm: HashAlgorithm, output: String },
    /// Checks the files that the checksum file (the one item) lists, relative to the folder it is in.
    VerifyChecksum,
    /// Cuts the file (the one item) into parts of `part_size` bytes ([`super::MIN_PART_SIZE`] or more) in `to`.
    Split { part_size: u64 },
    /// Joins the parts of the file whose first part (`.001`) is the one item, into `to`.
    Combine,
    /// Changes permissions and times of the items in place.
    Attributes(AttributeChange),
    /// Extracts entries of the archive (the one item, a file of `from`) into `to`: the `entries` (plain names; none means
    /// everything) of the archive's folder `inner` (`""` is its root, `a/b` a folder), folders with all that is below.
    Extract {
        inner: String,
        entries: Vec<String>,
        source: ArchiveJob,
    },
    /// Packs the items of `from` into the new archive `name` (a plain file name) in `to`. The one task whose items may be
    /// paths below `from` (`a/b.txt`, see [`super::is_item_path`]; none inside another): each is stored under that
    /// path, and the folders on the way are not entries of their own unless they are items.
    Pack { name: String, options: PackJob },
    /// Adds the items of `from` to the archive (the one item, a file of `to`) under its folder `inner` (`""` is its
    /// root, `a/b` a folder), by writing a new archive and replacing the old one when it is complete. `source` is
    /// what the caller knows of the archive (its index, its state and its password).
    AddToArchive {
        name: String,
        inner: String,
        source: ArchiveJob,
    },
    /// Renames the items (in `from`; `to` is `from` too) to `targets`, plain names in the order of the items. A swap, a
    /// cycle or a chain of names works (see `batch_rename`) and nothing is ever replaced. `undo` is a job that puts an
    /// earlier one back: an item that is gone, or whose old name is taken now, is skipped and counted, not an error.
    /// `log` receives the pairs that were renamed.
    Rename {
        targets: Vec<String>,
        undo: bool,
        log: RenameLog,
    },
    /// Copies the items between two folders that are compared, each the way its [`SyncPlan`] says, then removes the items
    /// of `removals` (a mirror). The names are paths below the two roots (see [`super::is_item_path`], none inside
    /// another): first one for each plan, in order, then one for each removal. An item that is not as the scan saw it is
    /// skipped, and a removal from a folder goes to the Trash unless `permanent`.
    ///
    /// Where the folders are: for two folders `from` is the left and `to` the right one. When one side is an archive, `from`
    /// is the folder that holds the archive file and `to` the other folder. An archive of a format that is written takes
    /// copies and removals too, all in one rewrite of it (see `sync_into`).
    Sync {
        left: SyncRoot,
        right: SyncRoot,
        plans: Vec<SyncPlan>,
        removals: Vec<SyncRemove>,
        permanent: bool,
        /// Receives what the job did to its folders, for its undo (see `sync_log`).
        log: SyncLog,
    },
    /// Puts back what the synchronization `log` describes (see `sync_undo`). The names are the paths it looks at, as
    /// [`SyncLog::names`] gives them; `from` and `to` are a folder the synchronization wrote to.
    SyncUndo { log: SyncLog },
}

/// One side of a synchronization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncRoot {
    /// A folder on disk (see [`Task::Sync`] for which one).
    Folder,
    /// The folder `inner` (`""` is the root, `a/b` a folder) of the archive `name`, a file of the request's `from`.
    Archive {
        name: String,
        inner: String,
        source: ArchiveJob,
    },
}

/// What one item of a synchronization does, and what the scan saw of its two sides (the item is copied only while both are
/// still so).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPlan {
    pub direction: SyncDirection,
    pub source: SyncFacts,
    pub target: Option<SyncFacts>,
}

/// One removal of a synchronization: the item only `side` has, as the scan saw it. It is removed only while it still is
/// (for a folder: the same files and bytes in its whole subtree too).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncRemove {
    pub side: PaneId,
    pub seen: SyncFacts,
    /// For a folder: the files in its subtree; 0 for a file.
    pub files: u64,
    /// For a folder: the bytes of those files; 0 for a file.
    pub bytes: u64,
    /// When the scan began, in milliseconds since the Unix epoch: nothing in a folder that is removed may be newer.
    pub scanned: i64,
}

/// How a pack job writes its archive. The format is the one the archive's `name` must have the extension of.
///
/// Equal when the options are, and the password is the same one (by identity). `Debug` says only whether there is a
/// password.
#[derive(Clone)]
pub struct PackJob {
    pub format: PackFormat,
    pub level: PackLevel,
    /// Encrypts every entry (zip: AES-256; 7z: AES-256). A tar.zst has none.
    pub password: Option<Arc<Password>>,
    /// Encrypts the names too: a 7z with a password only.
    pub encrypt_names: bool,
}

impl PartialEq for PackJob {
    fn eq(&self, other: &Self) -> bool {
        let same_password = match (&self.password, &other.password) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };

        same_password
            && self.format == other.format
            && self.level == other.level
            && self.encrypt_names == other.encrypt_names
    }
}

impl Eq for PackJob {}

impl fmt::Debug for PackJob {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("PackJob")
            .field("format", &self.format)
            .field("level", &self.level)
            .field("password", &self.password.is_some())
            .field("encrypt_names", &self.encrypt_names)
            .finish()
    }
}

impl Task {
    /// The kind of job of this task; `None` for [`Task::Transfer`], which is a copy or a move by [`Request::kind`].
    #[must_use]
    pub fn kind(&self) -> Option<OpKind> {
        match self {
            Self::Transfer => None,
            Self::Checksum { .. } => Some(OpKind::Checksum),
            Self::VerifyChecksum => Some(OpKind::VerifyChecksum),
            Self::Split { .. } => Some(OpKind::Split),
            Self::Combine => Some(OpKind::Combine),
            Self::Attributes(_) => Some(OpKind::Attributes),
            Self::Extract { .. } => Some(OpKind::Extract),
            Self::Pack { .. } => Some(OpKind::Pack),
            Self::AddToArchive { .. } => Some(OpKind::AddToArchive),
            Self::Rename { .. } => Some(OpKind::Rename),
            Self::Sync { .. } => Some(OpKind::Sync),
            Self::SyncUndo { .. } => Some(OpKind::SyncUndo),
        }
    }
}

/// A job to start. Paths are what the webview sent, already checked for syntax by the caller
/// (`check_folder` for the folders, `is_plain_name` for every name, `is_item_path` for those of a pack); the engine resolves them on its worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Copy or move for a [`Task::Transfer`]; the kind of the task otherwise (`submit` refuses a mismatch).
    pub kind: OpKind,
    /// The folder the items are in.
    pub from: PathBuf,
    /// The items, as plain names inside `from` (for a [`Task::Pack`] also relative paths below it). At least 1 and at most
    /// [`super::MAX_NAMES`], without duplicates; exactly 1 for a verify, a split and a combine.
    pub names: Vec<String>,
    /// The folder to put the items into; created when missing. For a task that works in place (checksum, verify,
    /// attributes) it is `from`.
    pub to: PathBuf,
    /// Re-read and hash each copied file before committing it.
    pub verify: bool,
    pub task: Task,
}

impl Request {
    /// A request for `task`, with the kind the task has, no verification and `to` as the target folder. A
    /// [`Task::Transfer`] is made a copy; set `kind` to [`OpKind::Move`] for a move.
    #[must_use]
    pub fn task(from: PathBuf, names: Vec<String>, to: PathBuf, task: Task) -> Self {
        Self {
            kind: task.kind().unwrap_or(OpKind::Copy),
            from,
            names,
            to,
            verify: false,
            task,
        }
    }
}

/// Why `submit` did not accept a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// [`super::MAX_JOBS`] jobs exist already; trying again later may work.
    Busy,
    /// The request has no names, more than [`super::MAX_NAMES`], a name twice, or a name that is not a plain file name (for
    /// a pack: nor a relative path of plain names, or one inside another).
    InvalidNames,
    /// A name is longer than [`super::MAX_NAME_UNITS`], or the names together take more than [`super::MAX_NAMES_BYTES`].
    NamesTooLarge,
    /// The kind and the task do not belong together, a verify, split or combine has other than one name, a part is
    /// shorter than [`super::MIN_PART_SIZE`], or an attribute change is empty, out of range or not for this system.
    InvalidTask,
}

/// A job that is not known: it never existed, or it has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownJob;

/// Why `answer` was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerError {
    /// The job is not known (see [`UnknownJob`]).
    UnknownJob,
    /// The job has no open question with that number: it was answered already, or the number is made up.
    StaleQuestion,
}

impl From<Refused> for IpcError {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::Busy => Self::new(IpcErrorCode::Busy, "too many transfers are running; try again shortly"),
            Refused::InvalidNames => Self::new(
                IpcErrorCode::InvalidInput,
                "choose between 1 and 100000 different items",
            ),
            Refused::NamesTooLarge => Self::new(
                IpcErrorCode::InvalidInput,
                "an item name, or the names together, are too long",
            ),
            Refused::InvalidTask => Self::new(IpcErrorCode::InvalidInput, "this job cannot start with these items"),
        }
    }
}

impl From<UnknownJob> for IpcError {
    fn from(_: UnknownJob) -> Self {
        Self::new(IpcErrorCode::NotFound, "that transfer is not running")
    }
}

impl From<AnswerError> for IpcError {
    fn from(error: AnswerError) -> Self {
        match error {
            AnswerError::UnknownJob => UnknownJob.into(),
            AnswerError::StaleQuestion => Self::new(IpcErrorCode::InvalidInput, "that question is no longer open"),
        }
    }
}

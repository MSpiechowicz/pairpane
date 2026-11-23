//! What a synchronization did to its folders, for the undo that may put it back: the files and folders it made, the files
//! it replaced (each with the old file kept aside, see `sync_aside`), and the folders it made on the way to them.
//!
//! The job writes the log as it goes, an item once it is done and only then, and marks it finished when it ends. Each item
//! carries what was there when the job left it (its identity, and its size and time), so that the undo touches only what
//! is still the job's own. The old files are kept in holding folders (one in each target folder, made when the first file
//! is kept), which the log names so that the caller can remove them once the undo is no longer offered.
//!
//! The holding folders go when nothing needs them any more: the sync is over, no undo of it runs, and either an undo went
//! through to its end or the caller released the log (the undo is no longer offered: a newer sync took its place, or the
//! app quits). An undo that was cancelled, failed or never ran keeps them, so it can be asked for again. The log lives in
//! memory only: nothing of it outlives the app.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::model::SyncFacts;

use super::control::locked;
use super::identity::FolderId;
use super::sync_holding::{Holding, Kept, drop_holding, is_in, path_of};

/// One thing the synchronization did, below the folder `root`, at the path `rel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Done {
    /// A file or a folder was made where there was nothing. A folder holds `totals` (files, bytes) as the job wrote it.
    Created {
        root: PathBuf,
        rel: String,
        id: Option<FolderId>,
        facts: SyncFacts,
        totals: (u64, u64),
    },
    /// A file was replaced by the one `id` and `facts` describe; the old one is kept in the holding folder of `root` as
    /// `aside`, or nowhere when it could not be kept (another drive, a file system without hard links).
    Replaced {
        root: PathBuf,
        rel: String,
        id: Option<FolderId>,
        facts: SyncFacts,
        aside: Option<Kept>,
    },
    /// A folder on the way to an item was made by the job: the folder `id`.
    Made {
        root: PathBuf,
        rel: String,
        id: Option<FolderId>,
    },
}

impl Done {
    pub(super) fn rel(&self) -> &str {
        match self {
            Self::Created { rel, .. } | Self::Replaced { rel, .. } | Self::Made { rel, .. } => rel,
        }
    }
}

#[derive(Default)]
struct Entries {
    done: Vec<Done>,
    /// The holding folders the job made, each in a target folder.
    holdings: Vec<Holding>,
    finished: bool,
    /// When the job ended, in milliseconds since the Unix epoch: what is newer than that was not left by the job.
    ended: i64,
    /// The undo is no longer offered: the holding folders go as soon as nothing uses them.
    released: bool,
    /// An undo job runs.
    running: bool,
    /// The undo jobs queued whose end the caller has not been told of yet.
    queued: u32,
    /// An undo went through to its end.
    undone: bool,
}

impl Entries {
    /// The holding folders to remove now, taken out of the log: nothing needs them any more (the sync is over, no undo
    /// runs or is queued, and the undo is no longer offered or was done).
    fn settle(&mut self) -> Vec<Holding> {
        if self.finished && !self.running && self.queued == 0 && (self.released || self.undone) {
            std::mem::take(&mut self.holdings)
        } else {
            Vec::new()
        }
    }
}

/// Removes `holdings`, with the log no longer locked: a target folder that does not answer holds up no one else.
fn drop_all(holdings: Vec<Holding>) {
    for holding in holdings {
        drop_holding(&holding);
    }
}

/// What a synchronization did. Clones share one log; two logs are equal when they are the same one. `Debug` says how many
/// things it holds.
#[derive(Clone, Default)]
pub struct SyncLog(Arc<Mutex<Entries>>);

impl SyncLog {
    /// Whether the job is over (it is never started again).
    #[must_use]
    pub fn finished(&self) -> bool {
        locked(&self.0).finished
    }

    /// How many things the undo would look at: the files and folders made or replaced (not the folders made on the way).
    #[must_use]
    pub fn undoable(&self) -> usize {
        locked(&self.0)
            .done
            .iter()
            .filter(|done| !matches!(done, Done::Made { .. }))
            .count()
    }

    /// The names an undo job is given: the paths of what it looks at, in the order it does.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let entries = locked(&self.0);
        let mut names: Vec<String> = Vec::new();

        for done in entries.done.iter().rev() {
            if !matches!(done, Done::Made { .. }) {
                names.push(done.rel().to_owned());
            }
        }

        names
    }

    /// The holding folders the job made, which hold the old files until the undo is no longer offered.
    #[must_use]
    pub fn holdings(&self) -> Vec<PathBuf> {
        locked(&self.0).holdings.iter().map(path_of).collect()
    }

    /// Whether an undo of the job runs now, or is queued.
    #[must_use]
    pub fn undoing(&self) -> bool {
        let entries = locked(&self.0);

        entries.running || entries.queued > 0
    }

    /// Whether an undo of the job went through to its end.
    #[must_use]
    pub fn undone(&self) -> bool {
        locked(&self.0).undone
    }

    /// The undo is no longer offered: the holding folders go now, or as soon as the sync or its undo is over.
    pub fn release(&self) {
        let dropping = {
            let mut entries = locked(&self.0);
            entries.released = true;
            entries.settle()
        };

        drop_all(dropping);
    }

    /// An undo job of the job is queued: until the caller says it ended ([`SyncLog::undo_ended`]) nothing it needs goes.
    pub fn undo_queued(&self) {
        let mut entries = locked(&self.0);
        entries.queued = entries.queued.saturating_add(1);
    }

    /// An undo job queued earlier ended, however it ended (also one cancelled before it ran, or one the engine refused).
    pub fn undo_ended(&self) {
        let dropping = {
            let mut entries = locked(&self.0);
            entries.queued = entries.queued.saturating_sub(1);
            entries.settle()
        };

        drop_all(dropping);
    }

    pub(super) fn push(&self, done: Done) {
        let mut entries = locked(&self.0);

        if !entries.done.contains(&done) {
            entries.done.push(done);
        }
    }

    pub(super) fn hold(&self, holding: Holding) {
        locked(&self.0).holdings.push(holding);
    }

    pub(super) fn holding_of(&self, root: &std::path::Path) -> Option<Holding> {
        locked(&self.0)
            .holdings
            .iter()
            .find(|holding| is_in(holding, root))
            .cloned()
    }

    /// The old file `kept` is in the holding folder of `root`.
    pub(super) fn keep(&self, root: &std::path::Path, kept: Kept) {
        let mut entries = locked(&self.0);

        if let Some(holding) = entries.holdings.iter_mut().find(|holding| is_in(holding, root)) {
            holding.kept.push(kept);
        }
    }

    pub(super) fn done(&self) -> Vec<Done> {
        locked(&self.0).done.clone()
    }

    pub(super) fn ended(&self) -> i64 {
        locked(&self.0).ended
    }

    pub(super) fn finish(&self, ended: i64) {
        let dropping = {
            let mut entries = locked(&self.0);
            entries.finished = true;
            entries.ended = ended;
            entries.settle()
        };

        drop_all(dropping);
    }

    /// An undo job starts.
    pub(super) fn begin_undo(&self) {
        locked(&self.0).running = true;
    }

    /// The undo job is over; `through` when it went to its end with nothing failed (what it skipped included), else it can
    /// be asked for again and the old files stay.
    pub(super) fn end_undo(&self, through: bool) {
        let dropping = {
            let mut entries = locked(&self.0);
            entries.running = false;
            entries.undone |= through;
            entries.settle()
        };

        drop_all(dropping);
    }
}

impl PartialEq for SyncLog {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SyncLog {}

impl fmt::Debug for SyncLog {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "SyncLog({} done)", locked(&self.0).done.len())
    }
}

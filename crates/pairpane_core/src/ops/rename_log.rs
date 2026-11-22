//! What a rename job did, for whoever may want to undo it: the pairs it renamed, and whether it is over.
//!
//! The job writes the log as it goes (a pair is in it once the item has its new name, and only then) and marks it
//! finished when it ends, however it ends. The caller that made the log reads it after that.
//!
//! The log also keeps the identity of the folder the job opened (see `identity`), so that the undo can refuse to act on a
//! folder that is not that one any more (the path was replaced by a link to another folder, say).

use std::fmt;
use std::sync::{Arc, Mutex};

use super::control::locked;
use super::identity::FileId;

#[derive(Default)]
struct Entries {
    /// (old name, new name) of each item that was renamed, in the order it happened.
    done: Vec<(String, String)>,
    finished: bool,
    /// The folder the job opened, when its identity could be read. An undo that has none to compare with is refused.
    folder: Option<Arc<FileId>>,
}

/// The renames a job made. Clones share one log; two logs are equal when they are the same one. `Debug` says how many
/// pairs it holds.
#[derive(Clone, Default)]
pub struct RenameLog(Arc<Mutex<Entries>>);

impl RenameLog {
    /// The pairs (old name, new name) of the items renamed so far.
    #[must_use]
    pub fn done(&self) -> Vec<(String, String)> {
        locked(&self.0).done.clone()
    }

    /// Whether the job is over (it is never started again).
    #[must_use]
    pub fn finished(&self) -> bool {
        locked(&self.0).finished
    }

    /// The log of the undo of this job: empty, and expecting the folder that this job opened.
    #[must_use]
    pub fn undoing(&self) -> Self {
        let folder = locked(&self.0).folder.clone();

        Self(Arc::new(Mutex::new(Entries {
            folder,
            ..Entries::default()
        })))
    }

    /// A log that expects the folder `path` is (for the tests of an undo).
    #[cfg(test)]
    pub(super) fn expecting(path: &std::path::Path) -> Self {
        let log = Self::default();
        log.set_folder(FileId::of_path(path));

        log
    }

    pub(super) fn set_folder(&self, id: Option<FileId>) {
        locked(&self.0).folder = id.map(Arc::new);
    }

    pub(super) fn folder(&self) -> Option<Arc<FileId>> {
        locked(&self.0).folder.clone()
    }

    pub(super) fn push(&self, from: &str, to: &str) {
        locked(&self.0).done.push((from.to_owned(), to.to_owned()));
    }

    pub(super) fn finish(&self) {
        locked(&self.0).finished = true;
    }
}

impl PartialEq for RenameLog {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RenameLog {}

impl fmt::Debug for RenameLog {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("RenameLog")
            .field("done", &locked(&self.0).done.len())
            .finish()
    }
}

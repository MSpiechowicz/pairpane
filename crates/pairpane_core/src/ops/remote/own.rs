//! What a download has put on this computer itself: a later item of the same job must never be written over it.
//!
//! A disk may give two names of a server (`A.txt` and `a.txt`) one name here. The second item then finds a file that the
//! job committed a moment ago: asking the user whether to replace it would end with the first item lost, and for a move
//! with its original removed from the server too. What is here is told by its identity (the file index of the disk), and
//! where that cannot be read, by its name in lower case.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use super::super::identity::FolderId;
use super::super::report::refusal;
use super::super::transfer::Run;

/// The files and folders this job made, by identity, and their names in lower case for a disk that gives none.
#[derive(Default)]
pub(super) struct Own {
    ids: HashSet<FolderId>,
    spelled: HashSet<String>,
}

impl Own {
    fn add(&mut self, id: Option<FolderId>, path: &Path) {
        self.ids.extend(id);
        self.spelled.insert(fold(path));
    }

    fn holds(&self, id: Option<FolderId>, path: &Path) -> bool {
        match id {
            Some(id) => self.ids.contains(&id),
            None => self.spelled.contains(&fold(path)),
        }
    }
}

fn fold(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// The error of an item whose name here is that of an item the job wrote before.
pub(super) fn same_name() -> io::Error {
    refusal(
        io::ErrorKind::AlreadyExists,
        "two items on the server have the same name on this computer",
    )
}

impl Run<'_> {
    /// The identity of what is at `path` now; `None` when it cannot be read.
    fn identify(&self, path: &Path) -> Option<FolderId> {
        if self.inner.hooks.unreadable_identity {
            return None;
        }

        FolderId::of_meta(path, &fs::symlink_metadata(path).ok()?)
    }

    /// Notes that the file or folder at `path` was made by this job.
    pub(super) fn mark_own(&self, own: &mut Own, path: &Path) {
        own.add(self.identify(path), path);
    }

    /// Whether the file or folder that exists at `path` is one this job made.
    pub(super) fn is_own(&self, own: &Own, path: &Path) -> bool {
        own.holds(self.identify(path), path)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[cfg(unix)]
    #[test]
    fn what_was_made_is_known_by_its_identity_and_not_by_a_spelling_that_differs() {
        let mut own = Own::default();
        let made = FolderId::of_ids((1, 7));

        own.add(Some(made), &PathBuf::from("/to/A.txt"));

        assert!(
            own.holds(Some(made), &PathBuf::from("/to/a.txt")),
            "the same file under another name"
        );
        assert!(
            !own.holds(Some(FolderId::of_ids((1, 8))), &PathBuf::from("/to/A.txt")),
            "another file"
        );
    }

    #[test]
    fn where_no_identity_is_read_the_name_in_lower_case_decides() {
        let mut own = Own::default();

        own.add(None, &PathBuf::from("/to/A.txt"));

        assert!(own.holds(None, &PathBuf::from("/to/a.TXT")));
        assert!(!own.holds(None, &PathBuf::from("/to/b.txt")));
    }
}

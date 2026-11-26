//! What a download has put on this computer itself: a later item of the same job must never be written over it.
//!
//! A disk may give two names of a server (`A.txt` and `a.txt`) one name here. The second item then finds a file that the
//! job committed a moment ago: asking the user whether to replace it would end with the first item lost, and for a move
//! with its original removed from the server too. What is here is told by its identity (the file index of the disk), and
//! where that cannot be read, by its name in lower case. A name in lower case is kept only for what has no identity: it
//! would take a different file that merely spells alike for one the job made.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use super::super::identity::FolderId;
use super::super::report::refusal;
use super::super::transfer::Run;

/// The files and folders this job made, by identity, and the names in lower case of those that have none to read.
#[derive(Default)]
pub(super) struct Own {
    ids: HashSet<FolderId>,
    spelled: HashSet<String>,
}

impl Own {
    fn add(&mut self, id: Option<FolderId>, path: &Path) {
        match id {
            Some(id) => {
                self.ids.insert(id);
            }
            None => {
                self.spelled.insert(fold(path));
            }
        }
    }

    /// Whether what is at `path`, which has the identity `id` when that could be read, is something the job made. What was
    /// made with no identity is told by its name even when the one at `path` has an identity now.
    fn holds(&self, id: Option<FolderId>, path: &Path) -> bool {
        id.is_some_and(|id| self.ids.contains(&id)) || self.spelled.contains(&fold(path))
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

    /// Whether what is at `path` cannot be told from what this job made: it is one of them, or its identity cannot be read
    /// and a move would lose a file by replacing it (a copy of an item is never lost with its original).
    pub(super) fn may_be_own(&self, own: &Own, path: &Path) -> bool {
        self.is_own(own, path) || (self.moving() && self.identify(path).is_none())
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

    #[cfg(unix)]
    #[test]
    fn a_file_with_an_identity_is_not_taken_for_another_by_its_spelling() {
        let mut own = Own::default();

        own.add(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/A.txt"));

        assert!(
            !own.holds(Some(FolderId::of_ids((1, 8))), &PathBuf::from("/to/a.txt")),
            "another file that is named alike"
        );
        assert!(
            !own.holds(None, &PathBuf::from("/to/a.txt")),
            "nothing was kept of the spelling"
        );
    }

    #[cfg(unix)]
    #[test]
    fn what_was_made_with_no_identity_is_told_by_its_name_when_the_one_found_has_an_identity() {
        let mut own = Own::default();

        own.add(None, &PathBuf::from("/to/A.txt"));

        assert!(own.holds(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/a.txt")));
        assert!(!own.holds(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/b.txt")));
    }

    #[test]
    fn where_no_identity_is_read_the_name_in_lower_case_decides() {
        let mut own = Own::default();

        own.add(None, &PathBuf::from("/to/A.txt"));

        assert!(own.holds(None, &PathBuf::from("/to/a.TXT")));
        assert!(!own.holds(None, &PathBuf::from("/to/b.txt")));
    }
}

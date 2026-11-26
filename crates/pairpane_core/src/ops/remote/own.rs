//! What a download has put on this computer itself: a later item of the same job must never be written over it.
//!
//! A disk may give two names of a server (`A.txt` and `a.txt`) one name here. The second item then finds a file that the
//! job committed a moment ago: asking the user whether to replace it would end with the first item lost, and for a move
//! with its original removed from the server too. What is here is told by its identity (the file index of the disk), and
//! by its name in lower case: an identity can fail to be read, or be reused or unstable on some disks, and then a name
//! alone has to tell. The two only ever refuse: a name that is alike never lets a file be replaced that the identity says
//! is another, and an identity that is not known never lets one through. A move keeps both for what it wrote; a copy needs
//! the name only where there is no identity.
//!
//! The name in lower case also refuses on a disk that tells `A.txt` from `a.txt`, when a move finds an unrelated `a.txt`
//! after writing `A.txt`: a move that keeps both originals is the safe side of that, and is accepted.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use super::super::identity::FolderId;
use super::super::report::refusal;
use super::super::transfer::Run;

/// The files and folders this job made, by identity, and by name in lower case (see the top of this file).
#[derive(Default)]
pub(super) struct Own {
    ids: HashSet<FolderId>,
    spelled: HashSet<String>,
}

impl Own {
    /// Notes what is at `path`, which has the identity `id` when that could be read. Its name is kept when there is no
    /// identity, and for what a move wrote (`also_named`).
    fn add(&mut self, id: Option<FolderId>, path: &Path, also_named: bool) {
        if let Some(id) = id {
            self.ids.insert(id);
        }

        if id.is_none() || also_named {
            self.spelled.insert(fold(path));
        }
    }

    /// Whether what is at `path`, which has the identity `id` when that could be read, is something the job made.
    fn holds(&self, id: Option<FolderId>, path: &Path) -> bool {
        id.is_some_and(|id| self.ids.contains(&id)) || self.spelled.contains(&fold(path))
    }
}

fn fold(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// The error of a replace that a move refuses: what is there cannot be told from what the move wrote.
fn unsure() -> io::Error {
    refusal(
        io::ErrorKind::AlreadyExists,
        "Kept the original: the file here could not be told apart from one this move already wrote",
    )
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
        own.add(self.identify(path), path, self.moving());
    }

    /// Whether the file or folder that exists at `path` is one this job made.
    pub(super) fn is_own(&self, own: &Own, path: &Path) -> bool {
        own.holds(self.identify(path), path)
    }

    /// Fails when replacing what is at `path` could lose what this job wrote: it is one of those, or (for a move, which
    /// would lose a file with its original; a copy of an item is never lost with its original) it cannot be told from one.
    pub(super) fn replaceable(&self, own: &Own, path: &Path) -> io::Result<()> {
        if self.is_own(own, path) {
            return Err(same_name());
        }

        if self.moving() && self.identify(path).is_none() {
            return Err(unsure());
        }

        Ok(())
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

        own.add(Some(made), &PathBuf::from("/to/A.txt"), false);

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
    fn a_copy_does_not_take_a_file_with_an_identity_for_another_by_its_spelling() {
        let mut own = Own::default();

        own.add(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/A.txt"), false);

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

        own.add(None, &PathBuf::from("/to/A.txt"), false);

        assert!(own.holds(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/a.txt")));
        assert!(!own.holds(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/b.txt")));
    }

    #[cfg(unix)]
    #[test]
    fn a_move_also_knows_what_it_wrote_by_its_name_for_an_identity_that_is_not_stable() {
        let mut own = Own::default();

        own.add(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/A.txt"), true);

        assert!(
            own.holds(Some(FolderId::of_ids((1, 8))), &PathBuf::from("/to/a.txt")),
            "the file has another identity now, and the same name"
        );
        assert!(
            own.holds(Some(FolderId::of_ids((1, 7))), &PathBuf::from("/to/other.txt")),
            "and its identity still tells it under another name"
        );
        assert!(
            !own.holds(Some(FolderId::of_ids((1, 8))), &PathBuf::from("/to/b.txt")),
            "a name that differs never refuses on its own"
        );
    }

    #[test]
    fn where_no_identity_is_read_the_name_in_lower_case_decides() {
        let mut own = Own::default();

        own.add(None, &PathBuf::from("/to/A.txt"), false);

        assert!(own.holds(None, &PathBuf::from("/to/a.TXT")));
        assert!(!own.holds(None, &PathBuf::from("/to/b.txt")));
    }
}

//! The files the built-in editor has opened as text in this run.
//!
//! A save is only for a file that `edit_open` gave the page as text: the page cannot use `edit_save` to write a file it
//! never opened in the editor (it is not the whole defence, see the header of the parent module). The files are kept by
//! where they are on disk (the folder resolved, the name as it is), up to [`MAX_OPENED`]; past that the oldest is
//! forgotten, and a save of it is refused until it is opened again.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// The most files remembered.
const MAX_OPENED: usize = 4096;

#[derive(Default)]
struct Files {
    set: HashSet<PathBuf>,
    /// Oldest first.
    order: VecDeque<PathBuf>,
}

/// The files opened as text, which a save may write.
#[derive(Default)]
pub(super) struct Opened {
    files: Mutex<Files>,
}

/// Where the file `name` of `folder` is, to be told again later: the folder with its links resolved (so a link to it is the
/// same place) and the name, which is not looked at, so that a file that is gone has a key too. `None` when the folder
/// cannot be resolved.
pub(super) fn key(folder: &Path, name: &str) -> Option<PathBuf> {
    fs::canonicalize(folder).ok().map(|folder| folder.join(name))
}

impl Opened {
    /// Remembers `file` as opened: the newest, and the oldest is forgotten when there are too many.
    pub(super) fn remember(&self, file: PathBuf) {
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);

        if files.set.contains(&file) {
            files.order.retain(|kept| *kept != file);
        } else {
            files.set.insert(file.clone());
        }
        files.order.push_back(file);

        while files.order.len() > MAX_OPENED {
            if let Some(oldest) = files.order.pop_front() {
                files.set.remove(&oldest);
            }
        }
    }

    /// Whether `file` was opened as text in this run (and is still remembered).
    pub(super) fn contains(&self, file: &Path) -> bool {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set
            .contains(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(number: usize) -> PathBuf {
        PathBuf::from(format!("/folder/{number}.txt"))
    }

    #[test]
    fn only_what_was_remembered_is_there() {
        let opened = Opened::default();
        assert!(!opened.contains(&file(1)));

        opened.remember(file(1));

        assert!(opened.contains(&file(1)));
        assert!(!opened.contains(&file(2)));
    }

    #[test]
    fn the_oldest_is_forgotten_past_the_bound_and_one_opened_again_is_new() {
        let opened = Opened::default();
        opened.remember(file(0));
        opened.remember(file(1));
        // Opened again: it is the newest now, so file 1 is the oldest.
        opened.remember(file(0));
        for number in 2..MAX_OPENED {
            opened.remember(file(number));
        }
        assert!(
            opened.contains(&file(0)) && opened.contains(&file(1)),
            "the bound is not passed yet"
        );

        opened.remember(file(MAX_OPENED));

        assert!(!opened.contains(&file(1)), "the oldest went");
        assert!(opened.contains(&file(0)));
        assert!(opened.contains(&file(MAX_OPENED)));
        assert_eq!(opened.files.lock().expect("lock").set.len(), MAX_OPENED);
    }

    #[test]
    fn a_folder_that_is_not_there_has_no_key_and_a_file_that_is_gone_has_one() {
        let folder = tempfile::tempdir().expect("folder");

        assert_eq!(key(&folder.path().join("missing"), "a.txt"), None);

        let gone = key(folder.path(), "gone.txt").expect("a key");
        assert_eq!(gone, fs::canonicalize(folder.path()).expect("path").join("gone.txt"));
    }
}

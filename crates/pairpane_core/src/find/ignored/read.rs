//! Reading the layer of a folder: which files are looked at, what each costs the budget, and what is left out.

use std::fs;
use std::hash::BuildHasher;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};

use crate::ops::open_listed;

use super::layer::{Layer, Rules};
use super::parse::{MAX_BYTES, count_lines, listed, parse, present, read_limited};
use super::{BIG, Ignores};

impl Ignores {
    /// The layer of the folder `dir`, below `parent`; `None` when the folder adds nothing to the layer above it: no rules
    /// and no `.git` (or one below another that is only a `.git`), or rules there is no room for (counted as left out).
    /// A folder with a `.git` that there is no room for is a boundary, that costs nothing.
    pub(super) fn read(&self, parent: Option<&Arc<Layer>>, dir: &Path) -> Option<Layer> {
        let git_entry = fs::symlink_metadata(dir.join(".git")).ok();
        let repository = git_entry.is_some();
        let in_repository = repository || parent.is_some_and(|parent| parent.in_repository);

        // A `.git` with no rules below a layer that is only a `.git` ends the reach of git's files where it already ends.
        let marker = repository && !parent.is_some_and(|parent| parent.repository && !parent.has_rules());

        // Only a `.git` folder holds an exclude file; a `.git` file points elsewhere and is not followed.
        let git_folder = git_entry.is_some_and(|entry| entry.is_dir());
        let files = present(dir, ".ignore")
            || (in_repository && present(dir, ".gitignore"))
            || (git_folder && present(dir, ".git/info/exclude"));

        if !files && !marker {
            return None;
        }

        // What the layer itself holds is charged as well as its rules, so that no tree can make it hold more than the
        // budget, however little each folder has; and before the files are read, so that a search with no room does not
        // read and compile rules only to give them up.
        let Some(cost) = self.reserve_layer(dir) else {
            if files {
                self.count_left_out(dir, "");
            }

            return marker.then(|| Layer::boundary(parent, self.clock.fetch_add(1, Ordering::Relaxed)));
        };

        let ignore = self.file(dir, ".ignore");
        let git = if in_repository {
            self.file(dir, ".gitignore")
        } else {
            None
        };
        let exclude = if git_folder {
            self.file(dir, ".git/info/exclude")
        } else {
            None
        };

        let layer = Layer {
            parent: parent.cloned(),
            dir: dir.to_owned(),
            depth: parent.map_or(0, |parent| parent.depth + 1),
            ignore,
            git,
            exclude,
            repository,
            in_repository,
            cost: Some(cost),
            used: AtomicU64::new(self.clock.fetch_add(1, Ordering::Relaxed)),
        };

        (marker || layer.has_rules()).then_some(layer)
    }

    /// The rules in the file `name` (names joined with `/`) of the folder `dir`: `None` when there is none, when it holds
    /// no rule, or when it cannot be used (which is counted).
    fn file(&self, dir: &Path, name: &str) -> Option<Rules> {
        // Most folders have none: one look at the name tells, where opening it step by step costs a call for each folder
        // on the way (and what is opened is checked as before).
        if !present(dir, name) {
            return None;
        }

        let opened = match dir.strip_prefix(&self.root) {
            Ok(below) => match listed(below, name) {
                Some(listed) => open_listed(&self.root, &listed),
                None => return self.left_out(dir, name),
            },
            Err(_) => open_listed(dir, name),
        };

        let (file, metadata) = match opened {
            Ok(opened) => opened,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => return self.left_out(dir, name),
        };

        if metadata.len() > MAX_BYTES {
            return self.left_out(dir, name);
        }

        let Some(text) = read_limited(file) else {
            return self.left_out(dir, name);
        };
        let bytes = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&text);

        let Some(lines) = count_lines(bytes) else {
            return self.left_out(dir, name);
        };
        let Some(charge) = self.reserve(lines, bytes.len(), 0) else {
            return self.left_out(dir, name);
        };

        // The rules of a big file take much memory while they are built: only a few are built at a time.
        let _turn = (lines > BIG).then(|| self.turns.wait());

        #[cfg(test)]
        self.compiled.fetch_add(1, Ordering::Relaxed);

        match parse(dir, bytes) {
            Some(gitignore) if gitignore.is_empty() => None,
            Some(gitignore) => Some(Rules { gitignore, charge }),
            None => self.left_out(dir, name),
        }
    }

    /// Counts the file `name` of `dir` (or, with no name, the layer of the folder) as left out, the first time only: a
    /// folder read again (after it was forgotten, or by two threads) counts nothing more.
    pub(super) fn count_left_out(&self, dir: &Path, name: &str) {
        let key = self.hasher.hash_one((dir, name));
        let first = self.counted.lock().unwrap_or_else(PoisonError::into_inner).insert(key);

        if first {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Counts the file `name` of `dir` as left out; `None`, as there are no rules from it.
    fn left_out(&self, dir: &Path, name: &str) -> Option<Rules> {
        self.count_left_out(dir, name);

        None
    }
}

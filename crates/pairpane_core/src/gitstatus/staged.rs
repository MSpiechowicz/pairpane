//! The index against `HEAD`, for the folder asked for: the files that are staged (added, deleted, or with another content or
//! mode than `HEAD` has). This is done here and not by gix's `tree_index_status`, which turns the whole tree of `HEAD` into
//! an index first, with no cap and no way to stop it: a repository can hold a few kilobytes of trees that all name the same
//! subtree again and again, which is a billion paths. Here only the folder's subtree of `HEAD` is walked, from the folder
//! down (each step of the way is a bounded lookup), with an explicit stack, and every entry costs one of the entries of the
//! [`Meter`] (so a bomb is `TooBig` after the cap, not after the billion) and looks at the stop flag. The bytes held by the
//! walk, the depth and the length of a path are bounded too, and gix's own limit on the size of one object (16 MiB for a
//! repository of reduced trust) holds for each object read.
//!
//! A sparse index (`git sparse-checkout set --cone --sparse-index`) holds a folder that is out of the work tree as one entry
//! of the folder's name with a slash at its end, with the mode of a folder and the id of its tree: everything of `HEAD` below
//! it is there and unchanged, or the entry's tree is another one than `HEAD`'s, and then the folder is staged. Such a folder of
//! `HEAD` is not walked, its files are not "deleted" for the index not listing them, and the entry is not "added" for `HEAD`
//! not listing it as a file.

use std::ops::Range;

use gix::bstr::{BStr, ByteSlice};
use gix::index::State;
use gix::index::entry::{Flags, Mode, Stage};
use gix::objs::tree::EntryMode;
use gix::objs::{FindExt, Kind, TreeRefIter};

use super::GitError;
use super::marks::Marks;
use super::watch::{Meter, Stop};
use crate::model::GitMark;

/// The deepest a tree is walked (the folder's own depth is not counted).
const MAX_DEPTH: usize = 256;

/// The longest path walked, in bytes: git itself writes none over 4 KiB, and Windows none over 32 KiB.
const MAX_PATH: usize = 64 * 1024;

/// The most the trees held open by the walk may add up to, in bytes.
const MAX_HELD: usize = 64 * 1024 * 1024;

/// What `HEAD` is: the commit (none for an unborn branch) and its tree.
pub(super) struct Head {
    pub(super) commit: Option<gix::ObjectId>,
    pub(super) tree: gix::ObjectId,
}

/// Reads `HEAD`: the commit it leads to and its tree. An unborn `HEAD` is an empty tree, so every entry is new.
pub(super) fn head(repository: &gix::Repository, stop: &Stop) -> Result<Head, GitError> {
    let read = || -> Result<Head, gix::Error> {
        let mut head = repository.head()?;
        if head.is_unborn() {
            return Ok(Head {
                commit: None,
                tree: repository.empty_tree().id,
            });
        }

        let commit = head.peel_to_commit()?;

        Ok(Head {
            commit: Some(commit.id),
            tree: commit.tree_id()?.detach(),
        })
    };

    read().map_err(|error| stop.explain(&error))
}

/// One tree being walked: its bytes, where the next entry is, and how long the path is for the entries of this tree.
struct Frame {
    data: Vec<u8>,
    at: usize,
    path_len: usize,
}

/// What the walk of one tree gave next.
enum Next {
    Done,
    Entry(EntryMode, gix::ObjectId),
}

/// The staged marks of the folder `marks` is for: `state` (the index) against the tree of `head`.
pub(super) fn staged(
    repository: &gix::Repository,
    state: &State,
    head: &gix::ObjectId,
    marks: &mut Marks,
    stop: &Stop,
    meter: &mut Meter<'_>,
) -> Result<(), GitError> {
    let range = state.prefixed_entries_range(marks.prefix().as_bstr()).unwrap_or(0..0);
    let mut walk = Walk {
        repository,
        state,
        seen: vec![false; range.len()],
        range,
        marks,
        stop,
        meter,
    };

    // A folder that is out of a sparse checkout is all as `HEAD` has it, and its files are not in the index to be compared.
    if walk.inside_sparse_directory() {
        return Ok(());
    }

    match walk.subtree(head)? {
        Subtree::Stopped => return Ok(()),
        Subtree::Missing => {}
        Subtree::Found(tree) => walk.walk(&tree)?,
    }
    if !walk.stop.is_stopped() {
        walk.added();
    }

    Ok(())
}

/// Where the folder is in the tree of `HEAD`.
enum Subtree {
    /// The walk was stopped on the way.
    Stopped,
    /// `HEAD` has no such folder: everything of the index in it is new.
    Missing,
    Found(gix::ObjectId),
}

struct Walk<'a, 'm> {
    repository: &'a gix::Repository,
    state: &'a State,
    /// The entries of the index inside the folder, and which of them `HEAD` has too.
    range: Range<usize>,
    seen: Vec<bool>,
    marks: &'a mut Marks,
    stop: &'a Stop,
    meter: &'a mut Meter<'m>,
}

impl Walk<'_, '_> {
    /// The tree of the folder inside the tree `head`, found by one lookup for each name of its path.
    fn subtree(&mut self, head: &gix::ObjectId) -> Result<Subtree, GitError> {
        let prefix = self.marks.prefix().to_vec();
        let mut tree = *head;

        for name in prefix.split(|byte| *byte == b'/').filter(|name| !name.is_empty()) {
            let child = self.child(&tree, name)?;
            if self.stop.is_stopped() {
                return Ok(Subtree::Stopped);
            }

            match child {
                Some(child) => tree = child,
                None => return Ok(Subtree::Missing),
            }
        }

        Ok(Subtree::Found(tree))
    }

    /// The tree called `name` in the tree `parent`.
    fn child(&mut self, parent: &gix::ObjectId, name: &[u8]) -> Result<Option<gix::ObjectId>, GitError> {
        let data = self.read(parent)?;

        for entry in TreeRefIter::from_bytes(&data, self.repository.object_hash()) {
            let entry = entry.map_err(|error| self.unreadable(&error))?;
            if !self.meter.spend(1) {
                return Ok(None);
            }

            if entry.mode.is_tree() && entry.filename.as_bytes() == name {
                return Ok(Some(entry.oid.to_owned()));
            }
        }

        Ok(None)
    }

    /// The bytes of the tree `id`, which is an object of another kind (a damaged repository) when it is not one.
    fn read(&self, id: &gix::oid) -> Result<Vec<u8>, GitError> {
        let mut buffer = Vec::new();
        let kind = self
            .repository
            .find(id, &mut buffer)
            .map_err(|error| self.stop.explain(&error))?
            .kind;

        if kind == Kind::Tree {
            Ok(buffer)
        } else {
            tracing::debug!("git status: a tree of HEAD is not a tree");

            Err(GitError::Repository)
        }
    }

    fn unreadable(&self, error: &dyn std::fmt::Display) -> GitError {
        tracing::debug!(%error, "git status: a tree of HEAD cannot be read");

        GitError::Repository
    }

    /// Walks the tree `root` and everything below it, with a stack of the trees open, comparing each file with the index.
    fn walk(&mut self, root: &gix::ObjectId) -> Result<(), GitError> {
        let mut path = self.marks.prefix().to_vec();
        let mut held = 0;
        let first = self.read(root)?;
        held += first.len();
        let mut stack = vec![Frame {
            data: first,
            at: 0,
            path_len: path.len(),
        }];

        while let Some(frame) = stack.last_mut() {
            if self.stop.is_stopped() {
                return Ok(());
            }

            let next = self.next(frame, &mut path)?;
            let Next::Entry(mode, id) = next else {
                held -= frame.data.len();
                stack.pop();

                continue;
            };

            if !self.meter.spend(1) {
                return Ok(());
            }
            if path.len() > MAX_PATH {
                self.meter.exhaust();

                return Ok(());
            }

            if mode.is_tree() {
                path.push(b'/');
                if self.sparse_directory(path.as_bstr(), &id) {
                    continue;
                }
                path.pop();

                if stack.len() >= MAX_DEPTH {
                    self.meter.exhaust();

                    return Ok(());
                }

                let data = self.read(&id)?;
                held += data.len();
                if held > MAX_HELD {
                    self.meter.exhaust();

                    return Ok(());
                }
                path.push(b'/');
                stack.push(Frame {
                    data,
                    at: 0,
                    path_len: path.len(),
                });
            } else {
                self.file(path.as_bstr(), mode, &id);
            }
        }

        Ok(())
    }

    /// Whether the folder this status is for is, or is below, a sparse directory of the index: one of the folders on its way (the
    /// folder itself too) has an entry of its own, with a slash at its end.
    fn inside_sparse_directory(&self) -> bool {
        let prefix = self.marks.prefix();

        prefix
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'/')
            .any(|(at, _)| self.sparse_entry(prefix[..=at].as_bstr()).is_some())
    }

    /// The position in the index of the sparse directory `path` (a folder's path with a slash at its end), if the index has one.
    fn sparse_entry(&self, path: &BStr) -> Option<usize> {
        let position = self.state.entry_index_by_path_and_stage(path, Stage::Unconflicted)?;

        self.state.entries()[position].mode.is_sparse().then_some(position)
    }

    /// A folder of `HEAD`, whose path with a slash is `path`, that the index holds as a sparse directory: it is not looked
    /// into. It is staged when the tree the index holds is not the tree of `HEAD`.
    fn sparse_directory(&mut self, path: &BStr, head_tree: &gix::oid) -> bool {
        let Some(position) = self.sparse_entry(path) else {
            return false;
        };
        if let Some(seen) = position
            .checked_sub(self.range.start)
            .and_then(|at| self.seen.get_mut(at))
        {
            *seen = true;
        }

        if *self.state.entries()[position].id != *head_tree {
            let folder = &path[..path.len() - 1];
            self.marks.note(folder.as_bstr(), GitMark::Staged);
        }

        true
    }

    /// The next entry of the tree open in `frame`, with its name put in `path` after the path of the tree (what an entry
    /// of a tree that was left, or of a folder that was entered, had there is cut off).
    fn next(&self, frame: &mut Frame, path: &mut Vec<u8>) -> Result<Next, GitError> {
        let rest = &frame.data[frame.at..];
        let mut entries = TreeRefIter::from_bytes(rest, self.repository.object_hash());
        let item = entries.next();
        let used = entries.offset_to_next_entry(rest);

        let entry = match item {
            None => return Ok(Next::Done),
            Some(Ok(entry)) => entry,
            Some(Err(error)) => return Err(self.unreadable(&error)),
        };

        path.truncate(frame.path_len);
        path.extend_from_slice(entry.filename);
        let next = Next::Entry(entry.mode, entry.oid.to_owned());
        frame.at += used;

        Ok(next)
    }

    /// A file of `HEAD` (or a link, or a submodule): the index has it with the same id and mode, with other ones (staged),
    /// or not at all (staged as deleted, unless it is in conflict, which is told by the other comparison).
    fn file(&mut self, path: &BStr, head_mode: EntryMode, head_id: &gix::oid) {
        let Some(position) = self.state.entry_index_by_path_and_stage(path, Stage::Unconflicted) else {
            if self.state.entry_by_path(path).is_none() {
                self.marks.note(path, GitMark::Staged);
            }

            return;
        };
        if let Some(seen) = position
            .checked_sub(self.range.start)
            .and_then(|at| self.seen.get_mut(at))
        {
            *seen = true;
        }

        let entry = &self.state.entries()[position];
        if entry.flags.contains(Flags::INTENT_TO_ADD) {
            return;
        }
        if *entry.id != *head_id || entry.mode != Mode::from(head_mode) {
            self.marks.note(path, GitMark::Staged);
        }
    }

    /// The entries of the index that `HEAD` does not have: staged as added (an entry that was only meant to be added later
    /// is not, and an entry in conflict is told by the other comparison).
    fn added(&mut self) {
        for (offset, seen) in self.seen.iter().enumerate() {
            if *seen {
                continue;
            }

            let entry = &self.state.entries()[self.range.start + offset];
            if entry.stage() != Stage::Unconflicted || entry.flags.contains(Flags::INTENT_TO_ADD) {
                continue;
            }
            // A sparse directory is a folder of `HEAD` that was not looked into (or one `HEAD` does not have, which is not a file
            // that is added).
            if entry.mode.is_sparse() {
                continue;
            }
            if !self.meter.spend(1) {
                return;
            }

            self.marks.note(entry.path(self.state), GitMark::Staged);
        }
    }
}

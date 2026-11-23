//! What the comparison of the work tree with the index is let to do: read a tracked file only when it is a file that can be
//! read, within a size, and never open the submodules.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use gix::bstr::{BStr, ByteSlice};
use gix::index::State;
use gix::index::entry::Mode;
use gix::status::plumbing::index_as_worktree::traits::{CompareBlobs, FastEq, ReadData, SubmoduleStatus};

use super::files;

/// Compares a tracked file that changed on disk with its index entry by size first and by content next, as gix does, with
/// these exceptions, which all count as modified without the file being read:
///
/// - a file over `max_bytes` (its read could not be interrupted, and the time limit would be at its mercy);
/// - anything that is not a regular file where the index has one, which a read would wait on or never end (a named pipe, a
///   device), or whose `.gitattributes` on the way is such a thing (gix reads those to convert the file);
/// - an entry that cannot be looked at.
///
/// The hook is called before gix opens the file (`index_as_worktree`'s `compare_blobs` call is what reads it, through the
/// `data` given to the hook), so nothing was opened for the file yet.
#[derive(Clone)]
pub(super) struct BoundedCompare<'a> {
    pub(super) root: &'a Path,
    pub(super) state: &'a State,
    pub(super) max_bytes: u64,
    /// The folders whose `.gitattributes` was looked at already.
    pub(super) looked_at: Arc<Mutex<HashSet<Vec<u8>>>>,
}

impl<'a> BoundedCompare<'a> {
    pub(super) fn new(root: &'a Path, state: &'a State, max_bytes: u64) -> Self {
        Self {
            root,
            state,
            max_bytes,
            looked_at: Arc::default(),
        }
    }

    /// Whether the work-tree file of `entry` may be opened and read.
    fn readable(&self, entry: &gix::index::Entry) -> bool {
        let relative = entry.path(self.state);
        let Ok(relative_path) = gix::path::from_bstr(relative) else {
            return false;
        };
        let Ok(metadata) = fs::symlink_metadata(self.root.join(relative_path)) else {
            return false;
        };

        // A link is read as a link (its target is the content) where the index has a link; a regular file is read.
        let kind = metadata.file_type();
        let plain = kind.is_file() || (kind.is_symlink() && entry.mode == Mode::SYMLINK);

        plain && self.attributes_readable(relative)
    }

    /// Whether the `.gitattributes` of each folder above the file are fine to read, each looked at once.
    fn attributes_readable(&self, relative: &BStr) -> bool {
        let mut looked_at = self.looked_at.lock().unwrap_or_else(PoisonError::into_inner);
        let mut folder = Vec::new();
        let mut names = relative.split(|byte| *byte == b'/').peekable();

        loop {
            if looked_at.insert(folder.clone()) {
                let Ok(folder_path) = gix::path::from_bstr(folder.as_bstr()) else {
                    return false;
                };
                if files::risky_rules_file(&self.root.join(folder_path).join(".gitattributes")) {
                    // Left out of the set: every file below looks at it again, and is modified.
                    looked_at.remove(&folder);

                    return false;
                }
            }

            let Some(name) = names.next() else {
                return true;
            };
            if names.peek().is_none() {
                return true;
            }
            if !folder.is_empty() {
                folder.push(b'/');
            }
            folder.extend_from_slice(name);
        }
    }
}

impl CompareBlobs for BoundedCompare<'_> {
    type Output = ();

    fn compare_blobs<'a, 'b>(
        &mut self,
        entry: &gix::index::Entry,
        worktree_blob_size: u64,
        data: impl ReadData<'a>,
        buf: &mut Vec<u8>,
    ) -> gix::Result<Option<Self::Output>> {
        if worktree_blob_size > self.max_bytes || !self.readable(entry) {
            return Ok(Some(()));
        }

        FastEq.compare_blobs(entry, worktree_blob_size, data, buf)
    }
}

/// The status of a submodule is "no change", without opening it, reading `.gitmodules` or touching its folder: the
/// repository of a submodule has a config of its own that is no more trusted than the superproject's, and opening it
/// would start a status there under the trust of the folder's owner.
#[derive(Clone)]
pub(super) struct NoSubmodules;

impl SubmoduleStatus for NoSubmodules {
    type Output = ();

    fn status(&mut self, _entry: &gix::index::Entry, _rela_path: &BStr) -> gix::Result<Option<Self::Output>> {
        Ok(None)
    }
}

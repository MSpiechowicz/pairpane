//! What the comparison of the work tree with the index is let to do: read a tracked file only when it is a file that can be
//! read, within a size, and never open the submodules.

use std::fs;
use std::path::Path;

use gix::bstr::{BStr, ByteSlice};
use gix::index::State;
use gix::index::entry::Mode;
use gix::status::plumbing::index_as_worktree::traits::{CompareBlobs, FastEq, ReadData, SubmoduleStatus};

use super::budget::{Budget, Charged};
use super::watch::Stop;

/// Compares a tracked file that changed on disk with its index entry by size first and by content next, as gix does, with
/// these exceptions, which all count as modified without the file being read:
///
/// - a file over `max_bytes` (its read could not be interrupted, and the time limit would be at its mercy);
/// - anything that is not a regular file where the index has one, which a read would wait on or never end (a named pipe, a
///   device), or whose rules (`.gitignore` and `.gitattributes` of the folders on its way) are such a thing or too big to
///   be rules (gix reads those to convert the file);
/// - an entry that cannot be looked at.
///
/// The rules of the folders on the way are counted in the budget of the status before they are read (see [`Budget`]), and
/// past its total the status is too big.
///
/// The hook is called before gix opens the file (`index_as_worktree`'s `compare_blobs` call is what reads it, through the
/// `data` given to the hook), so nothing was opened for the file yet.
#[derive(Clone)]
pub(super) struct BoundedCompare<'a> {
    pub(super) root: &'a Path,
    pub(super) state: &'a State,
    pub(super) max_bytes: u64,
    pub(super) budget: &'a Budget,
    pub(super) stop: &'a Stop,
}

impl BoundedCompare<'_> {
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

        plain && self.rules_readable(relative)
    }

    /// Whether the rules of each folder above the file are fine to read, each counted once.
    fn rules_readable(&self, relative: &BStr) -> bool {
        let mut folder = Vec::new();
        let mut names = relative.split(|byte| *byte == b'/').peekable();

        loop {
            match self.budget.charge(folder.as_bstr()) {
                Charged::Fine => {}
                Charged::Large | Charged::Odd => return false,
                Charged::Over => {
                    self.stop.exhaust();

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

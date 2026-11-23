//! The one budget of the rules a status may read. gix reads an ignore or an attributes file for every folder it goes through
//! and holds every pattern of all of them in memory: the `.gitignore` and `.gitattributes` of the work tree, which it reads on
//! the way down from the root to the folder asked for, in each folder the directory walk enters and in the folder of each
//! file it compares; and, for a file that is not in the work tree (deleted, or marked skip-worktree in the index), the blob of
//! the index entry in its place. A file or a blob that is only big, or a long chain of folders that each hold a big one, is a
//! memory and a time that nothing else bounds, so every source is counted here *before* gix pushes the folder: one source may
//! be [`SOURCE_BYTES`] at most (a rules file that is more is not one a person wrote), and all of them together
//! [`TOTAL_BYTES`], the rules of the git folder (`info/exclude`, `info/attributes`) included. Each folder is counted once.
//!
//! The work-tree file is sized by the file that is opened (see [`super::files::rules_file`]), the blob by its header in the
//! object database (`find_header`, which is the size after its deltas are applied, without reading the blob). The blobs are
//! sized when the budget is made, for the entries of the index that can matter (the folder's subtree and the rules files of its
//! parents), because gix's repository cannot be used from the places that count. A source that
//! is a named pipe, a device or such is [`Charged::Odd`], and one over its cap [`Charged::Large`]. When the work-tree file and
//! the blob can both be read the larger of the two is counted. The memory limit of the process that runs the status is the
//! backstop for anything that is missed here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use gix::bstr::{BStr, ByteSlice};
use gix::index::State;
use gix::index::entry::Stage;

use super::files::{self, Rules};
use super::watch::Stop;

/// The most one rules source (a `.gitignore`, a `.gitattributes`, a blob in the place of one, `info/exclude`) may hold.
pub(super) const SOURCE_BYTES: u64 = 256 * 1024;

/// The most the rules sources of one status may add up to.
pub(super) const TOTAL_BYTES: u64 = 8 * 1024 * 1024;

/// The rules files of a folder.
const NAMES: [&str; 2] = [".gitignore", ".gitattributes"];

/// What counting the rules of a folder found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Charged {
    /// Counted; gix may go through the folder.
    Fine,
    /// A source over [`SOURCE_BYTES`]: the folder is not to be read through.
    Large,
    /// A source that is a pipe, a device or such: the folder is not to be read through.
    Odd,
    /// The total is over [`TOTAL_BYTES`]: the status is too big.
    Over,
}

/// The budget of one status. Shared by the steps of the status (clones of the compare share it), which count from different
/// places, so its state is behind a lock.
pub(super) struct Budget {
    workdir: PathBuf,
    /// The size of the blob of each index entry that is a rules file, by its repository-relative path: what gix reads when the
    /// work-tree file is not there or is skip-worktree.
    blobs: HashMap<Vec<u8>, u64>,
    state: Mutex<Counted>,
}

struct Counted {
    total: u64,
    /// What counting each folder (its repository-relative path, `/` separated, empty for the root) found.
    folders: HashMap<Vec<u8>, Charged>,
}

/// Whether `name` is the name of a rules file.
fn is_rules_name(name: &[u8]) -> bool {
    NAMES.iter().any(|rules| rules.as_bytes() == name)
}

impl Budget {
    /// The budget of a status of the repository `repository` (its index is `index`) for the folder `prefix`
    /// (repository-relative, `/` separated, without a slash at its end, empty for the work tree's root) whose work tree is
    /// `workdir`, with `used` bytes spent already (the rules of the git folder). The blobs of the rules files of the folder's
    /// subtree and of its parents are sized now; `stop` ends that if the status is stopped.
    pub(super) fn new(
        workdir: &Path,
        repository: &gix::Repository,
        index: &State,
        prefix: &str,
        used: u64,
        stop: &Stop,
    ) -> Self {
        let mut blobs = HashMap::new();
        let mut size = |entry: &gix::index::Entry| {
            let bytes = repository.find_header(entry.id).map_or(0, |header| header.size());

            blobs.insert(entry.path(index).to_vec(), bytes);
        };

        let mut with_slash = prefix.as_bytes().to_vec();
        if !with_slash.is_empty() {
            with_slash.push(b'/');
        }
        if let Some(range) = index.prefixed_entries_range(with_slash.as_bstr()) {
            for entry in &index.entries()[range] {
                if stop.is_stopped() {
                    break;
                }

                let path = entry.path(index);
                let name = path.rsplit(|byte| *byte == b'/').next().unwrap_or_default();
                if entry.stage() == Stage::Unconflicted && is_rules_name(name) {
                    size(entry);
                }
            }
        }

        // The parents of the folder, and the folder itself: the rules gix reads for any entry of it.
        let mut folder = Vec::new();
        for name in std::iter::once("").chain(prefix.split('/').filter(|name| !name.is_empty())) {
            if !name.is_empty() {
                if !folder.is_empty() {
                    folder.push(b'/');
                }
                folder.extend_from_slice(name.as_bytes());
            }
            for rules in NAMES {
                let mut path = folder.clone();
                if !path.is_empty() {
                    path.push(b'/');
                }
                path.extend_from_slice(rules.as_bytes());

                if let Some(entry) = index.entry_by_path_and_stage(path.as_bstr(), Stage::Unconflicted) {
                    size(entry);
                }
            }
        }

        Self {
            workdir: workdir.to_path_buf(),
            blobs,
            state: Mutex::new(Counted {
                total: used,
                folders: HashMap::new(),
            }),
        }
    }

    /// Counts the rules sources of the folder `folder` (repository-relative, `/` separated, empty for the work tree's root)
    /// the first time it is asked, and answers what that found each time after.
    pub(super) fn charge(&self, folder: &BStr) -> Charged {
        let mut counted = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(charged) = counted.folders.get(folder.as_bytes()) {
            return *charged;
        }

        let charged = self.count(&mut counted, folder);
        counted.folders.insert(folder.to_vec(), charged);

        charged
    }

    fn count(&self, counted: &mut Counted, folder: &BStr) -> Charged {
        let mut worst = Charged::Fine;

        for name in NAMES {
            let mut relative = folder.to_vec();
            if !relative.is_empty() {
                relative.push(b'/');
            }
            relative.extend_from_slice(name.as_bytes());

            match self.size_of(&relative) {
                Ok(bytes) => counted.total += bytes,
                Err(Charged::Odd) => worst = Charged::Odd,
                Err(Charged::Large) if worst != Charged::Odd => worst = Charged::Large,
                Err(_) => {}
            }
            if counted.total > TOTAL_BYTES {
                return Charged::Over;
            }
        }

        worst
    }

    /// The bytes gix may read for the rules source at `relative`: the larger of the work-tree file and the blob of the index
    /// entry. `Err` for a source that must not be read.
    fn size_of(&self, relative: &[u8]) -> Result<u64, Charged> {
        let Ok(path) = gix::path::from_bstr(relative.as_bstr()) else {
            return Err(Charged::Odd);
        };

        let in_tree = match files::rules_file(&self.workdir.join(path), SOURCE_BYTES) {
            Rules::Bytes(bytes) => bytes,
            Rules::Large => return Err(Charged::Large),
            Rules::Odd => return Err(Charged::Odd),
        };
        let in_index = self.blobs.get(relative).copied().unwrap_or(0);
        if in_index > SOURCE_BYTES {
            return Err(Charged::Large);
        }

        Ok(in_tree.max(in_index))
    }
}

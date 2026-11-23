//! The ignored children of a folder that is untracked as a whole. The directory walk says so with one entry for the folder
//! and says nothing of what is ignored inside it, so what its direct children are is asked of the ignore rules (the same
//! ones that the walk used). A child the rules name is ignored. A child folder that the rules do not name but that holds
//! ignored files only is ignored too, as git has it (`git status --ignored` lists a folder whose files are all ignored as one
//! ignored entry); that is looked at here, with a bound of its own on top of the meter and the rules budget: at most
//! [`LOOKED_AT`] entries of such folders are looked at in one status, and a folder that is not settled by then (or one whose
//! rules cannot be read) stays untracked, which is the answer when nothing is known.

use std::fs;
use std::path::PathBuf;

use gix::bstr::{BString, ByteSlice};
use gix::index::entry::Mode;

use super::GitError;
use super::budget::{Budget, Charged};
use super::marks::Marks;
use super::watch::{Meter, Stop};
use crate::model::GitMark;

/// How many entries of the folders that are no child the rules name are looked at in one status.
const LOOKED_AT: usize = 10_000;

/// The look at the children of the folder a status is for.
pub(super) struct Quiet<'a, 'm> {
    pub(super) repository: &'a gix::Repository,
    pub(super) state: &'a gix::index::State,
    /// The real path of the folder the status is for.
    pub(super) folder: &'a std::path::Path,
    pub(super) budget: &'a Budget,
    pub(super) marks: &'a mut Marks,
    pub(super) stop: &'a Stop,
    pub(super) meter: &'a mut Meter<'m>,
}

/// What the rules say of an entry of the folder, with its mode.
fn mode_of(file_type: fs::FileType) -> Mode {
    if file_type.is_dir() {
        Mode::DIR
    } else if file_type.is_symlink() {
        Mode::SYMLINK
    } else {
        Mode::FILE
    }
}

impl Quiet<'_, '_> {
    pub(super) fn run(mut self) -> Result<(), GitError> {
        let stop = self.stop;
        let explain = |error: gix::Error| stop.explain(&error);
        let mut excludes = self
            .repository
            .excludes(
                self.state,
                None,
                gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(explain)?;
        let mut left = LOOKED_AT;

        for entry in fs::read_dir(self.folder).map_err(|error| GitError::Folder(error.kind()))? {
            let entry = entry.map_err(|error| GitError::Folder(error.kind()))?;
            if !self.meter.spend(1) {
                return Ok(());
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let Ok(name) = gix::path::os_str_into_bstr(&entry.file_name()).map(ToOwned::to_owned) else {
                continue;
            };
            if name == ".git" {
                continue;
            }

            let mut path = self.marks.prefix().to_vec();
            path.extend_from_slice(&name);
            let path = BString::from(path);
            let mode = mode_of(file_type);

            // The rules of the folders the status is for were counted already: the folder and its parents, and the entries here
            // are the folder's own.
            let mut ignored = excludes
                .at_entry(path.as_bstr(), Some(mode))
                .map_err(explain)?
                .is_excluded();
            if !ignored && file_type.is_dir() {
                let found = self.only_ignored(&mut excludes, &path, &entry.path(), &mut left)?;
                ignored = found && !self.stop.is_stopped();
            }

            if ignored {
                self.marks.note(path.as_bstr(), GitMark::Ignored);
            }
        }

        Ok(())
    }

    /// Whether the folder at `real` (`relative` is its repository-relative path) that no rule names holds nothing but ignored
    /// files: at least one entry, each of them ignored by the rules or a folder of the same kind. A folder that holds a `.git`
    /// is a repository of its own and untracked; one that cannot be settled is not ignored.
    fn only_ignored(
        &mut self,
        excludes: &mut gix::AttributeStack<'_>,
        relative: &BString,
        real: &std::path::Path,
        left: &mut usize,
    ) -> Result<bool, GitError> {
        let stop = self.stop;
        let explain = |error: gix::Error| stop.explain(&error);
        let mut pending: Vec<(BString, PathBuf)> = vec![(relative.clone(), real.to_path_buf())];
        let mut any = false;

        while let Some((folder, real)) = pending.pop() {
            // The rules gix reads for the entries of this folder are counted before the first of them is asked.
            match self.budget.charge(folder.as_bstr()) {
                Charged::Fine => {}
                Charged::Over => {
                    self.meter.exhaust();

                    return Ok(false);
                }
                Charged::Large | Charged::Odd => return Ok(false),
            }
            let Ok(listing) = fs::read_dir(&real) else {
                return Ok(false);
            };

            for entry in listing {
                let Ok(entry) = entry else {
                    return Ok(false);
                };
                if *left == 0 || !self.meter.spend(1) {
                    return Ok(false);
                }
                *left -= 1;

                let Ok(file_type) = entry.file_type() else {
                    return Ok(false);
                };
                let Ok(name) = gix::path::os_str_into_bstr(&entry.file_name()).map(ToOwned::to_owned) else {
                    return Ok(false);
                };
                if name == ".git" {
                    return Ok(false);
                }

                let mut path = folder.to_vec();
                path.push(b'/');
                path.extend_from_slice(&name);
                let path = BString::from(path);
                let ignored = excludes
                    .at_entry(path.as_bstr(), Some(mode_of(file_type)))
                    .map_err(explain)?
                    .is_excluded();

                if ignored {
                    any = true;
                } else if file_type.is_dir() {
                    pending.push((path, entry.path()));
                } else {
                    return Ok(false);
                }
            }
        }

        Ok(any)
    }
}

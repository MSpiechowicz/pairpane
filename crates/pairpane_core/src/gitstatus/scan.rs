//! The status of a folder: find the repository, then merge the three comparisons limited to the folder into marks.

use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gix::bstr::{BStr, BString, ByteSlice};
use gix::dir::EntryRef;
use gix::dir::entry::{Kind as DiskKind, Status as DirStatus};
use gix::dir::walk::{Action, Delegate, EmissionMode};
use gix::status::plumbing::index_as_worktree::traits::{CompareBlobs, FastEq, ReadData, SubmoduleStatus};
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
use gix::status::plumbing::index_as_worktree_with_renames::{Entry, VisitEntry};
use gix::status::tree_index::TrackRenames;
use gix::worktree::stack::state::attributes::Source;

use super::marks::Marks;
use super::watch::{Meter, Stop, guarded};
use super::{GitError, Limits, MAX_COMPARE_BYTES, open};
use crate::model::{GitMark, GitStatus};

/// The status of the direct children of `folder`; see [`super::status`].
pub(super) fn status(folder: &Path, limits: Limits, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    // The user's home folder is where the search for a repository ends (it is the last folder looked in).
    let home = std::env::home_dir().and_then(|home| gix::path::realpath(home).ok());

    status_below(folder, home, limits, cancel)
}

/// Like [`status`], with the folder the search for a repository must not go above given (`None`: the filesystem root).
pub(super) fn status_below(
    folder: &Path,
    ceiling: Option<PathBuf>,
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<GitStatus, GitError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    // Links are resolved first: the work tree has to be found as a parent of where the folder really is.
    let real = gix::path::realpath(folder).map_err(|_| GitError::Repository)?;
    match std::fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(GitError::Folder(io::ErrorKind::NotADirectory)),
        Err(error) => return Err(GitError::Folder(error.kind())),
    }
    let stop = Stop::default();

    let result = guarded(&stop, limits.time, cancel, || {
        let mut meter = Meter::new(&stop, limits.entries);

        read(&real, ceiling, &stop, &mut meter)
    });

    // A flag that was raised is the answer, whatever error the interrupted read ended in.
    match stop.reason() {
        Some(reason) => Err(reason),
        None => result,
    }
}

fn not_a_repository() -> GitStatus {
    GitStatus {
        repo: false,
        marks: Vec::new(),
    }
}

fn read(real: &Path, ceiling: Option<PathBuf>, stop: &Stop, meter: &mut Meter<'_>) -> Result<GitStatus, GitError> {
    let Some(repository) = open::find(real, ceiling)? else {
        return Ok(not_a_repository());
    };
    // A bare repository has no work tree, so none of its folders is in one.
    let Some(workdir) = repository.workdir() else {
        return Ok(not_a_repository());
    };
    let Some(prefix) = prefix_of(real, workdir) else {
        return Ok(not_a_repository());
    };
    let mut marks = Marks::new(&prefix);

    compare(&repository, &prefix, &mut marks, stop, meter).map_err(|error| failed(&error, stop))?;
    let marks = marks.finish(real, meter).map_err(|error| match stop.reason() {
        Some(reason) => reason,
        None => GitError::Folder(error.kind()),
    })?;

    Ok(GitStatus { repo: true, marks })
}

/// The path of `folder` below `workdir`, `/` separated and without a slash at its end (empty for the work tree's root),
/// or `None` when `folder` is not inside it, is inside the `.git` folder, or has a name that is not valid text.
fn prefix_of(folder: &Path, workdir: &Path) -> Option<String> {
    let workdir: PathBuf = gix::path::realpath(workdir).ok()?;
    let rest = folder.strip_prefix(&workdir).ok()?;
    let rest = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(rest).ok()?);
    let rest = rest.to_str().ok()?;

    if rest == ".git" || rest.starts_with(".git/") {
        return None;
    }

    Some(rest.to_owned())
}

/// What an interrupted or failed comparison is told as: a raised flag is its own answer, anything else is an unreadable
/// repository (the details are only logged).
fn failed(error: &gix::Error, stop: &Stop) -> GitError {
    if let Some(reason) = stop.reason() {
        return reason;
    }
    tracing::debug!(%error, "git status: the repository cannot be read");

    GitError::Repository
}

/// The three comparisons of one status, all limited to the same pathspec and merging into the same marks.
struct Run<'a, 'm> {
    repository: &'a gix::Repository,
    state: &'a gix::index::State,
    patterns: Vec<BString>,
    marks: &'a mut Marks,
    stop: &'a Stop,
    meter: &'a mut Meter<'m>,
}

fn compare(
    repository: &gix::Repository,
    prefix: &str,
    marks: &mut Marks,
    stop: &Stop,
    meter: &mut Meter<'_>,
) -> gix::Result<()> {
    let index = repository.index_or_empty()?;
    let state: &gix::index::State = &index;

    // The entries of the index inside the folder are what is checked; a folder with more than the cap is too big.
    let range = state.prefixed_entries_range(marks.prefix().as_bstr());
    if !meter.spend(range.map_or(0, |range| range.len())) {
        return Ok(());
    }

    // `:(literal)` keeps the characters of a folder's name (`*`, `[`, `?`) from being read as a pattern.
    let patterns: Vec<BString> = if prefix.is_empty() {
        Vec::new()
    } else {
        vec![format!(":(literal){prefix}").into()]
    };
    let mut run = Run {
        repository,
        state,
        patterns,
        marks,
        stop,
        meter,
    };

    run.staged()?;
    run.changed()?;
    run.walked()
}

impl Run<'_, '_> {
    /// The index against `HEAD`: the files that are staged. An unborn `HEAD` is an empty tree, so every entry is new.
    fn staged(&mut self) -> gix::Result<()> {
        if self.stop.is_stopped() {
            return Ok(());
        }

        let head = self.repository.head_tree_id_or_empty()?;
        // The attributes of a pathspec are never asked for (the pattern has none), so none are read from the work tree.
        let mut pathspec = self
            .repository
            .pathspec(false, &self.patterns, true, self.state, Source::IdMapping)?;
        let (marks, meter) = (&mut *self.marks, &mut *self.meter);

        self.repository.tree_index_status(
            &head,
            self.state,
            Some(&mut pathspec),
            TrackRenames::Disabled,
            |change, _, _| {
                if !meter.spend(1) {
                    return Ok(ControlFlow::Break(()));
                }

                let folder_like = match &change {
                    gix::diff::index::ChangeRef::Addition { entry_mode, .. }
                    | gix::diff::index::ChangeRef::Deletion { entry_mode, .. }
                    | gix::diff::index::ChangeRef::Modification { entry_mode, .. }
                    | gix::diff::index::ChangeRef::Rewrite { entry_mode, .. } => entry_mode.is_submodule(),
                };
                marks.note(change.location(), GitMark::Staged, folder_like);

                Ok(ControlFlow::Continue(()))
            },
        )?;

        Ok(())
    }

    /// The work tree against the index, for the tracked files: the files that are modified or in conflict.
    fn changed(&mut self) -> gix::Result<()> {
        if self.stop.is_stopped() {
            return Ok(());
        }

        let options = gix::status::index_worktree::Options {
            // No directory walk here (it is `walked`), no rename tracking, one thread.
            dirwalk_options: None,
            rewrites: None,
            sorting: None,
            thread_limit: Some(1),
        };
        let mut visitor = Changes {
            marks: &mut *self.marks,
            meter: &mut *self.meter,
        };

        self.repository.index_worktree_status(
            self.state,
            &self.patterns,
            &mut visitor,
            BoundedCompare,
            NoSubmodules,
            &mut gix::progress::Discard,
            &self.stop.interrupt,
            options,
        )?;

        Ok(())
    }

    /// The directory walk: the files and folders that are untracked or ignored.
    fn walked(&mut self) -> gix::Result<()> {
        if self.stop.is_stopped() {
            return Ok(());
        }

        // A folder whose files are all of one kind is one entry, not one for each file. Nested repositories are not entered.
        let options = self
            .repository
            .dirwalk_options()?
            .emit_untracked(EmissionMode::CollapseDirectory)
            .emit_ignored(Some(EmissionMode::CollapseDirectory))
            .emit_tracked(false)
            .emit_pruned(false)
            .emit_empty_directories(false)
            .recurse_repositories(false);
        let mut walker = Walker {
            marks: &mut *self.marks,
            meter: &mut *self.meter,
        };

        self.repository
            .dirwalk(self.state, &self.patterns, &self.stop.interrupt, options, &mut walker)?;

        Ok(())
    }
}

/// Compares a tracked file that changed on disk with its index entry by size first and by content next, as gix does,
/// except for a file over [`MAX_COMPARE_BYTES`], which is read no further and counts as modified: its read could not be
/// interrupted, and the time limit would be at its mercy.
#[derive(Clone)]
struct BoundedCompare;

impl CompareBlobs for BoundedCompare {
    type Output = ();

    fn compare_blobs<'a, 'b>(
        &mut self,
        entry: &gix::index::Entry,
        worktree_blob_size: u64,
        data: impl ReadData<'a>,
        buf: &mut Vec<u8>,
    ) -> gix::Result<Option<Self::Output>> {
        if worktree_blob_size > MAX_COMPARE_BYTES {
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

/// Takes the changes of tracked files as they are found.
struct Changes<'a, 'm> {
    marks: &'a mut Marks,
    meter: &'a mut Meter<'m>,
}

impl<'index> VisitEntry<'index> for Changes<'_, '_> {
    type ContentChange = ();
    type SubmoduleStatus = ();

    fn visit_entry(&mut self, entry: Entry<'index, (), ()>) {
        let Entry::Modification {
            entry,
            rela_path,
            status,
            ..
        } = entry
        else {
            return;
        };

        let mark = match status {
            EntryStatus::Conflict { .. } => GitMark::Conflicted,
            EntryStatus::Change(Change::Removed | Change::Type { .. } | Change::Modification { .. })
            | EntryStatus::Change(Change::SubmoduleModification(())) => GitMark::Modified,
            EntryStatus::IntentToAdd => GitMark::Modified,
            // Only the stat of the entry is stale; the content is the same.
            EntryStatus::NeedsUpdate(_) => return,
        };

        if self.meter.spend(1) {
            self.marks.note(rela_path, mark, entry.mode.is_submodule());
        }
    }
}

/// Takes the untracked and ignored entries of the directory walk as they are found, and counts every folder it enters.
struct Walker<'a, 'm> {
    marks: &'a mut Marks,
    meter: &'a mut Meter<'m>,
}

impl Delegate for Walker<'_, '_> {
    fn emit(&mut self, entry: EntryRef<'_>, _collapsed: Option<DirStatus>) -> Action {
        if !self.meter.spend(1) {
            return ControlFlow::Break(());
        }

        let mark = match entry.status {
            DirStatus::Untracked => GitMark::Untracked,
            DirStatus::Ignored(_) => GitMark::Ignored,
            DirStatus::Pruned | DirStatus::Tracked => return ControlFlow::Continue(()),
        };
        let folder_like = matches!(entry.disk_kind, Some(DiskKind::Directory | DiskKind::Repository));
        self.marks.note(entry.rela_path.as_ref(), mark, folder_like);

        ControlFlow::Continue(())
    }

    fn can_recurse(
        &mut self,
        entry: EntryRef<'_>,
        for_deletion: Option<gix::dir::walk::ForDeletionMode>,
        worktree_root_is_repository: bool,
    ) -> bool {
        // A folder entered counts as an entry; at the cap the walk is told to stop and goes no deeper.
        if !self.meter.spend(1) {
            return false;
        }

        entry.status.can_recurse(
            entry.disk_kind,
            entry.pathspec_match,
            for_deletion,
            worktree_root_is_repository,
        )
    }
}

//! What the comparisons of a status give, taken as gix finds it: the changes of tracked files and the entries of the
//! directory walk, each counted and noted in the marks.

use std::ops::ControlFlow;
use std::path::Path;

use gix::bstr::BStr;
use gix::dir::EntryRef;
use gix::dir::entry::Status as DirStatus;
use gix::dir::walk::{Action, Delegate};
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
use gix::status::plumbing::index_as_worktree_with_renames::{Entry, VisitEntry};

use super::files;
use super::marks::Marks;
use super::watch::Meter;
use crate::model::GitMark;

/// Takes the changes of tracked files as they are found.
pub(super) struct Changes<'a, 'm> {
    pub(super) marks: &'a mut Marks,
    pub(super) meter: &'a mut Meter<'m>,
}

impl<'index> VisitEntry<'index> for Changes<'_, '_> {
    type ContentChange = ();
    type SubmoduleStatus = ();

    fn visit_entry(&mut self, entry: Entry<'index, (), ()>) {
        let Entry::Modification { rela_path, status, .. } = entry else {
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
            self.marks.note(rela_path, mark);
        }
    }
}

/// Takes the untracked and ignored entries of the directory walk as they are found, and counts every folder it enters.
pub(super) struct Walker<'a, 'm> {
    pub(super) marks: &'a mut Marks,
    pub(super) meter: &'a mut Meter<'m>,
    /// The work tree, to look at the `.gitignore` of a folder before the walk enters it.
    pub(super) workdir: &'a Path,
}

impl Walker<'_, '_> {
    /// Whether the rules files of the folder `rela_path` must not be handed to gix (see [`files::risky_rules_file`]): the
    /// walk reads its `.gitignore` on entering, and a pipe there would hold it up for ever. Such a folder is not entered.
    fn rules_are_risky(&self, rela_path: &BStr) -> bool {
        let Ok(rela_path) = gix::path::from_bstr(rela_path) else {
            return true;
        };

        files::risky_rules_file(&self.workdir.join(rela_path).join(".gitignore"))
    }
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
        self.marks.note(entry.rela_path.as_ref(), mark);

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
        ) && !self.rules_are_risky(entry.rela_path.as_ref())
    }
}

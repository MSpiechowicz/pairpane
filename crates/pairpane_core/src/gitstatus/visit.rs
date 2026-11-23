//! What the comparisons of a status give, taken as gix finds it: the changes of tracked files and the entries of the
//! directory walk, each counted and noted in the marks.

use std::ops::ControlFlow;

use gix::bstr::BStr;
use gix::dir::EntryRef;
use gix::dir::entry::{Kind as DirKind, Status as DirStatus};
use gix::dir::walk::{Action, Delegate};
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
use gix::status::plumbing::index_as_worktree_with_renames::{Entry, VisitEntry};

use super::budget::{Budget, Charged};
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
    /// The rules budget of the status: what the rules of each folder entered add up to.
    pub(super) budget: &'a Budget,
}

impl Walker<'_, '_> {
    /// Whether the walk may enter the folder `rela_path`, whose rules it reads on entering (see [`Budget`]): a pipe there
    /// would hold it up for ever, and a file too big is no rules file, so such a folder is not entered, and is `modified` as
    /// the safe thing to say of what is not looked at. The sources add up, and past the total the status is too big (the
    /// patterns of all of them are held in memory).
    fn may_enter(&mut self, rela_path: &BStr) -> bool {
        match self.budget.charge(rela_path) {
            Charged::Fine => !self.meter.is_stopped(),
            Charged::Large | Charged::Odd => {
                self.marks.note(rela_path, GitMark::Modified);

                false
            }
            Charged::Over => {
                self.meter.exhaust();

                false
            }
        }
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

        // The submodules are never entered (point 6 of the module docs): the walk is told to look in folders that hold
        // repositories, so what the index holds as a gitlink is skipped here.
        if entry.index_kind == Some(DirKind::Repository) {
            return false;
        }

        entry.status.can_recurse(
            entry.disk_kind,
            entry.pathspec_match,
            for_deletion,
            worktree_root_is_repository,
        ) && self.may_enter(entry.rela_path.as_ref())
    }
}

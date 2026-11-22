//! What one item of a synchronization may do at its target, decided for the item and not for the job.
//!
//! The page confirmed that files would be replaced, but only the files the scan saw: a file that appeared at a target the
//! scan saw empty, or one edited or swapped since, is not among them. So an item carries the terms it is written on:
//!
//! - [`Terms::Absent`]: the scan saw nothing at the target. The item is put there with a no-replace rename, which the
//!   system refuses when anything is at the name by then (a skipped item, the copy removed). A folder copied whole is
//!   written on these terms too, inside a folder this job made: nothing in it is ever replaced.
//! - [`Terms::Same`]: the scan saw this file. It is replaced only when, right before the rename that replaces it, what is
//!   there is still that file: the same kind, size and time to the millisecond, and the same identity (device and inode, or
//!   volume and file index), which a lookalike put in its place does not have. An identity that cannot be read on either
//!   side fails closed: the item is skipped.
//!
//! The compare and the rename are two steps: a file swapped in between the last look and the rename itself still wins that
//! race (no system call replaces only a file it has looked at), which is as narrow as it can be made.

use std::fs;
use std::io;
use std::path::Path;

use crate::model::{Answer, SyncFacts};

use super::identity::FolderId;
use super::sync::{facts, same_facts};
use super::transfer::Run;

/// What is at a target: its facts and its identity.
pub(super) type Seen = (SyncFacts, Option<FolderId>);

/// What the item being written may do at its target (see the module's text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Terms {
    Absent,
    Same { facts: SyncFacts, id: Option<FolderId> },
}

impl Terms {
    /// The terms for a target that the plan says is `wanted` and that is `seen`; `None` when it is not as the plan says.
    pub(super) fn judge(wanted: Option<&SyncFacts>, seen: Option<Seen>) -> Option<Self> {
        match seen {
            None => wanted.is_none().then_some(Self::Absent),
            Some((now, id)) => same_facts(Some(&now), wanted).then_some(Self::Same { facts: now, id }),
        }
    }

    /// Whether replacing what is `seen` now is within the terms.
    pub(super) fn allows(&self, seen: Option<&Seen>) -> bool {
        match (self, seen) {
            (Self::Same { facts, id: Some(id) }, Some((now, Some(now_id)))) => {
                same_facts(Some(now), Some(facts)) && now_id == id
            }
            _ => false,
        }
    }

    /// The answer to a conflict that goes with these terms: a taken name is left alone, a file that is still the one the
    /// scan saw is replaced (the final look is [`Run::may_replace`]).
    fn answer(&self) -> Answer {
        match self {
            Self::Absent => Answer::Skip,
            Self::Same { .. } => Answer::Overwrite,
        }
    }
}

/// What is at `path` now (a link being its own thing); `None` when nothing is.
pub(super) fn seen_at(path: &Path) -> io::Result<Option<Seen>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some((facts(&meta), FolderId::of_meta(path, &meta)))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

impl Run<'_> {
    /// Sets the terms the next item is written on (`None`: none, as for any other job).
    pub(super) fn set_terms(&mut self, terms: Option<Terms>) {
        self.policy = terms.as_ref().map(Terms::answer);
        self.terms = terms;
    }

    /// Whether the file at `target` may be replaced by the item being written: always for a job with no terms, else only
    /// when it is still what the terms say, which is looked at now.
    pub(super) fn may_replace(&self, target: &Path) -> bool {
        match &self.terms {
            None => true,
            Some(terms) => seen_at(target)
                .ok()
                .flatten()
                .is_some_and(|seen| terms.allows(Some(&seen))),
        }
    }
}

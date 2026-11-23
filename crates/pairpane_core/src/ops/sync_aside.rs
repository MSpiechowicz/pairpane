//! What a synchronization keeps for its undo (see `sync_log`): before an item is written, the file it is to replace is
//! hard-linked into a holding folder of the target folder, so the replacing rename leaves the old file there; after the
//! item, what the job left at its path is written down, and the folders it made on the way.
//!
//! The holding folder is a hidden folder of the target folder (`.pairpane-sync-…`) that the job makes and pins (see
//! `sync_holding`). The link is made by handle on Unix (each folder on the way entered with no link followed, `linkat` from
//! the folder held open), so a folder swapped for a link cannot put another file there; on Windows by path. Where no link
//! can be made (the item is on another drive than its target folder, or the file system has no hard links) the item is
//! replaced all the same, as the user confirmed, and the log says its old file was not kept.
//!
//! An item is written down only when the job did it: what is at its path now is a copy of its source (its kind, and for a
//! file its size and time) that was not there before (another identity), and the job says it wrote it: for a copy, its
//! skipped and failed counts did not move; for items extracted from an archive, the extraction tells which rows it wrote.
//! A folder made on the way is written down with its identity.

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use crate::listing::system_millis;
use crate::model::EntryKind;

use super::SyncPlan;
use super::held::Held;
use super::identity::FolderId;
use super::plan::Kind;
use super::sync::{same_facts, split_item};
use super::sync_holding::{Holding, Kept, forget, identity_of, make, open};
use super::sync_log::{Done, SyncLog};
use super::sync_remove::facts_of;
use super::sync_terms::seen_at;
use super::transfer::{MAX_DEPTH, Run};

/// What the name of every holding folder starts with: a comparison never lists such a folder.
pub const HOLDING_PREFIX: &str = ".pairpane-sync-";

/// What was looked at before an item was written.
pub(super) struct Before {
    /// The folders on the way (paths below the target folder) that were not there.
    missing: Vec<String>,
    /// The old file, kept in the holding folder.
    aside: Option<Kept>,
    /// The skipped and failed counts of the job.
    counts: (u32, usize),
    /// What was at the item's path.
    old: Option<FolderId>,
}

/// The time now, in milliseconds since the Unix epoch.
pub(super) fn now_millis() -> i64 {
    system_millis(SystemTime::now()).unwrap_or(0)
}

impl Run<'_> {
    /// Looks at the item `rel` of the target folder `root` before it is written (see the module's text).
    pub(super) fn before_item(&mut self, log: &SyncLog, root: &Path, rel: &str, plan: &SyncPlan) -> Before {
        let (parents, _) = split_item(rel);
        let mut missing = Vec::new();
        let mut at = String::new();

        for part in parents.split('/').filter(|part| !part.is_empty()) {
            if !at.is_empty() {
                at.push('/');
            }
            at.push_str(part);

            if !missing.is_empty() || fs::symlink_metadata(root.join(&at)).is_err() {
                missing.push(at.clone());
            }
        }

        let aside = plan.target.as_ref().and_then(|_| self.keep_aside(log, root, rel, plan));

        Before {
            missing,
            aside,
            counts: (self.skipped, self.failures()),
            old: seen_at(&root.join(rel)).ok().flatten().and_then(|(_, id)| id),
        }
    }

    /// Writes down what the job left at the item `rel` of `root`, or lets go of the old file kept for it when the job did
    /// not replace it. `wrote` says whether the job wrote the item, when it knows (an extraction); else its counts tell.
    pub(super) fn after_item(
        &mut self,
        log: &SyncLog,
        (root, rel): (&Path, &str),
        plan: &SyncPlan,
        before: Before,
        wrote: Option<bool>,
    ) {
        for made in &before.missing {
            if let Ok(Some((facts, id))) = seen_at(&root.join(made))
                && facts.kind == EntryKind::Dir
            {
                log.push(Done::Made {
                    root: root.to_owned(),
                    rel: made.clone(),
                    id,
                });
            }
        }

        let clean = wrote.unwrap_or_else(|| before.counts == (self.skipped, self.failures()));
        let path = root.join(rel);
        let now = seen_at(&path).ok().flatten();
        // A file that is still the one that was there was not replaced, whatever its size and time say.
        let copied = now.as_ref().filter(|(facts, id)| {
            clean && same_facts(Some(facts), Some(&plan.source)) && (before.old.is_none() || *id != before.old)
        });

        match (copied, plan.target) {
            (Some(&(facts, id)), Some(_)) => log.push(Done::Replaced {
                root: root.to_owned(),
                rel: rel.to_owned(),
                id,
                facts,
                aside: before.aside,
            }),
            (Some(&(facts, id)), None) => log.push(Done::Created {
                root: root.to_owned(),
                rel: rel.to_owned(),
                id,
                facts,
                totals: if facts.kind == EntryKind::Dir {
                    totals(&path)
                } else {
                    (0, 0)
                },
            }),
            (None, _) => {
                if let (Some(kept), Some(holding)) = (before.aside, log.holding_of(root)) {
                    forget(&holding, &kept);
                }
            }
        }
    }

    /// Links the file the plan replaces at `rel` of `root` into the holding folder of `root`, when it is still the file the
    /// scan saw; gives its name there.
    fn keep_aside(&mut self, log: &SyncLog, root: &Path, rel: &str, plan: &SyncPlan) -> Option<Kept> {
        let (parents, name) = split_item(rel);
        let top = Held::root(root).ok()?;
        let mut folder: Option<Held> = None;

        for part in parents.split('/').filter(|part| !part.is_empty()) {
            let above = folder.as_ref().unwrap_or(&top);
            let node = above.look(part).ok().filter(|node| node.kind == Kind::Dir)?;
            folder = Some(above.subfolder(&node).ok()?);
        }

        let folder = folder.as_ref().unwrap_or(&top);
        let node = folder.look(name).ok().filter(|node| node.kind == Kind::File)?;
        if !same_facts(Some(&facts_of(&node)), plan.target.as_ref()) {
            return None;
        }

        let holding = self.holding(log, root, &top).ok()?;
        self.sequence = self.sequence.wrapping_add(1);
        let kept = Kept {
            name: self.sequence.to_string(),
            id: identity_of(&node),
        };

        link(folder, name, &holding, &kept.name).ok()?;
        log.keep(root, kept.clone());

        Some(kept)
    }

    /// The holding folder of `root` (held open as `top`), made now when the job has none there yet.
    fn holding(&self, log: &SyncLog, root: &Path, top: &Held) -> io::Result<Held> {
        if let Some(holding) = log.holding_of(root) {
            return open(top, &holding);
        }

        let name = format!("{HOLDING_PREFIX}{}-{}", self.job.id.0, now_millis());
        let (held, id) = make(top, &name)?;
        log.hold(Holding {
            root: root.to_owned(),
            name,
            id,
            kept: Vec::new(),
        });

        Ok(held)
    }
}

/// Links the file `name` of `folder` into `holding` as `aside`, no link followed.
#[cfg(unix)]
fn link(folder: &Held, name: &str, holding: &Held, aside: &str) -> io::Result<()> {
    use rustix::fs::{AtFlags, linkat};

    linkat(&folder.dir, name, &holding.dir, aside, AtFlags::empty()).map_err(io::Error::from)
}

#[cfg(windows)]
fn link(folder: &Held, name: &str, holding: &Held, aside: &str) -> io::Result<()> {
    fs::hard_link(folder.path.join(name), holding.path.join(aside))
}

/// The files below the folder `path` and their bytes, no link followed; `(u64::MAX, 0)` when it cannot all be read, which no
/// folder matches (the undo then keeps it).
fn totals(path: &Path) -> (u64, u64) {
    fn walk(path: &Path, depth: usize, found: &mut (u64, u64)) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;

            if kind.is_file() {
                found.0 += 1;
                found.1 = found.1.saturating_add(entry.metadata()?.len());
            } else if kind.is_dir() {
                if depth + 1 >= MAX_DEPTH {
                    return Err(io::Error::other("too deep"));
                }

                walk(&entry.path(), depth + 1, found)?;
            }
        }

        Ok(())
    }

    let mut found = (0, 0);
    walk(path, 0, &mut found).map_or((u64::MAX, 0), |()| found)
}

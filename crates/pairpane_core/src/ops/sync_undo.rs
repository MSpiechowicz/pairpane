//! The undo of a synchronization (see `sync_log`): what the job made is removed and what it replaced is put back, the last
//! first, each only while it is still what the job left there.
//!
//! - A file the job made: its identity, size and time must be what the job left; it is removed.
//! - A folder the job made: its identity, and the files and bytes the job wrote into it; nothing in it, the folder
//!   included, may be newer than the end of the job (a time, or on Unix the change time); it is removed with all in it,
//!   through handles as a mirror's removal is (see `sync_unlink`), and never when it is or holds a mounted drive.
//! - A file the job replaced: the file at its path must still be the job's (identity, size and time), and the old one is
//!   renamed back over it from the holding folder. One whose old file was not kept is skipped.
//! - A folder the job made on the way to an item is removed when it is empty by then.
//!
//! Anything else is skipped and counted. Every step goes by handle on Unix (each folder on the way entered with no link
//! followed, the renames and removals done from the folders held open), by path on Windows. The holding folders are
//! removed when the undo went through to its end, with the old files that were not put back; an undo that was cancelled
//! or failed keeps them, and can be asked for again (what it did already is then skipped).

use std::io;

use crate::model::JobPhase;

use super::held::{Held, Node};
use super::plan::Kind;
use super::sync::{same_facts, split_item};
use super::sync_holding::{identity_of, kept_file, open};
use super::sync_log::{Done, SyncLog};
use super::sync_remove::{Root, Since, Survey, device_of, enter, facts_of, is_mount, survey};
use super::transfer::{Run, Stop};

impl Run<'_> {
    /// Runs the undo of the synchronization `log` describes (see the module's text).
    pub(super) fn sync_undo(&mut self, log: &SyncLog) -> Result<(), Stop> {
        let ended = log.ended();
        self.phase(JobPhase::Removing);

        let done = log.done();
        log.begin_undo();
        let result = (|| {
            for item in done.iter().rev() {
                self.checkpoint()?;

                let undone = self.undo_one(log, item, ended);
                if !matches!(item, Done::Made { .. }) {
                    self.settle(item.rel(), undone)?;
                }
            }

            Ok(())
        })();

        // Through to its end, the old files that were not put back go; cancelled or failed, they stay for another undo.
        log.end_undo(result.is_ok());

        result
    }

    /// Undoes one thing; counts it as skipped when it is not what the job left.
    fn undo_one(&mut self, log: &SyncLog, item: &Done, ended: i64) -> Result<(), Stop> {
        let (root, rel) = match item {
            Done::Created { root, rel, .. } | Done::Replaced { root, rel, .. } | Done::Made { root, rel, .. } => {
                (root, rel)
            }
        };
        let (parents, name) = split_item(rel);
        let top = Root::open(root)?;
        let Some((entered, _)) = enter(&top, parents, None)? else {
            return self.gone(item);
        };
        let folder = entered.as_ref().unwrap_or(&top.held);
        let node = match folder.look(name) {
            Ok(node) => node,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return self.gone(item),
            Err(error) => return Err(error.into()),
        };

        match item {
            Done::Made { id, .. } => {
                // Only the very folder the job made, and only once it is empty.
                if node.kind == Kind::Dir && id.is_some() && identity_of(&node) == *id {
                    let _ = remove_empty(folder, &node);
                }

                Ok(())
            }
            Done::Created { id, facts, totals, .. } => {
                // A copy keeps its source's time, which may be any: only the change time tells what changed since.
                let since = Since {
                    time: ended,
                    changes_only: true,
                };
                if !is_left(&node, *id, facts) || since.sees(&node) {
                    return self.changed();
                }

                if node.kind == Kind::Dir {
                    let held = folder.subfolder(&node)?;
                    let hooks = &self.inner.hooks;
                    if is_mount(&held, &node, (device_of(folder)?, hooks))? {
                        return self.changed();
                    }

                    let mut found = Survey::default();
                    let depth = rel.split('/').count();
                    if survey(&held, (node.device, hooks), depth, (since, None), &mut found)?.is_none()
                        || (found.files, found.bytes) != *totals
                        || found.mount
                    {
                        return self.changed();
                    }
                }

                self.remove_held(folder, &node, since)?;
                self.renamed();

                Ok(())
            }
            Done::Replaced { id, facts, aside, .. } => {
                let (Some(aside), Some(holding)) = (aside, log.holding_of(root)) else {
                    return self.changed();
                };
                // An edit in place that put the time back still moves the change time.
                let since = Since {
                    time: ended,
                    changes_only: true,
                };
                if !is_left(&node, *id, facts) || since.sees(&node) {
                    return self.changed();
                }

                let kept = open(&top.held, &holding)?;
                let old = kept_file(&kept, aside)?;
                put_back(&kept, &old, folder, &node)?;
                self.renamed();

                Ok(())
            }
        }
    }

    /// An item that is gone: nothing to undo (a folder made on the way is not counted).
    fn gone(&mut self, item: &Done) -> Result<(), Stop> {
        if matches!(item, Done::Made { .. }) {
            Ok(())
        } else {
            self.changed()
        }
    }
}

/// Whether `node` is what the job left: the same identity (when both are known), kind, and for a file size and time.
fn is_left(node: &Node, id: Option<super::identity::FolderId>, facts: &crate::model::SyncFacts) -> bool {
    same_facts(Some(&facts_of(node)), Some(facts)) && id.is_some_and(|id| identity_of(node) == Some(id))
}

/// Renames the old file `old` of the holding folder `kept` back over `node` of `folder`.
#[cfg(unix)]
fn put_back(kept: &Held, old: &Node, folder: &Held, node: &Node) -> io::Result<()> {
    use rustix::fs::renameat;

    renameat(&kept.dir, &old.name, &folder.dir, &node.name).map_err(io::Error::from)
}

#[cfg(windows)]
fn put_back(_kept: &Held, old: &Node, _folder: &Held, node: &Node) -> io::Result<()> {
    std::fs::rename(&old.path, &node.path)
}

/// Removes the folder `node` of `folder` when it is empty.
#[cfg(unix)]
fn remove_empty(folder: &Held, node: &Node) -> io::Result<()> {
    use rustix::fs::{AtFlags, unlinkat};

    unlinkat(&folder.dir, &node.name, AtFlags::REMOVEDIR).map_err(io::Error::from)
}

#[cfg(windows)]
fn remove_empty(_folder: &Held, node: &Node) -> io::Result<()> {
    std::fs::remove_dir(&node.path)
}

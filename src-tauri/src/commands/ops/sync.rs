//! `ops_sync`: the items a comparison chose copied between two folders as one job of the transfer engine.
//!
//! The command checks syntax only (the paths and the archive, like `ops_extract`). A job that copies into the archive side or
//! removes from it rewrites the archive, so it is prepared as adding files is (`ops_add_to_archive`): a format that is
//! written, the whole index, and the password for encrypted entries; the index kept for the archive goes when it ends. The engine checks the items (paths below the folders, none inside another, each
//! plan a copy can carry out) and the job looks at every item again when it runs. For an archive the command also brings
//! what `ops_extract` brings: its index (kept, or built now for a zip or 7z), the state of the file it was made from and the
//! password the user gave, and fails with `passwordRequired` when the chosen entries are encrypted and there is none.
//!
//! Where the folders go in the request (see `Task::Sync`): two folders are `from` and `to` as they are given; with an archive
//! on one side, `from` is the folder that holds the archive and `to` the other folder, the one written to. The engine queues
//! jobs by the volume of `to` alone, so two syncs that write to one disk wait for each other whichever way they copy. A mirror
//! that removes from `from` (the left folder) does so without holding that volume: removing is renames into the Trash or
//! unlinks, not a stream of writes, so another job on that disk is not slowed by it in a way the queue is there to prevent.

use pairpane_core::archive::{self, Fingerprint};
use pairpane_core::model::{CompareRoot, IpcErrorCode, JobId, PaneId, SyncDirection, SyncItem, SyncRemoval};
use pairpane_core::ops::{ArchiveJob, Request, SyncPlan, SyncRemove, SyncRoot, Task};

use super::add::{check_writable, prepare_rewrite};
use super::extract::{check_folders, prepare};
use super::{Ops, failed};
use crate::commands::archives::{Archives, Target};
use crate::commands::compare::{Root, refused};
use crate::commands::items::blocking;
use crate::error::{CommandError, CommandResult};

impl Ops {
    /// Checks the request, prepares what the job brings of the archive (on a blocking thread) and queues it.
    pub(crate) async fn sync(
        &self,
        archives: &Archives,
        (left, right): (CompareRoot, CompareRoot),
        items: Vec<SyncItem>,
        (removals, permanent): (Vec<SyncRemoval>, bool),
        read: Option<Fingerprint>,
    ) -> CommandResult<JobId> {
        let (left, right) = (Root::parse(left)?, Root::parse(right)?);
        if left.is_archive() && right.is_archive() {
            return Err(refused("Two archives cannot be compared."));
        }

        let mut names: Vec<String> = items.iter().map(|item| item.path.clone()).collect();
        names.extend(removals.iter().map(|removal| removal.path.clone()));
        let plans: Vec<SyncPlan> = items
            .iter()
            .map(|item| SyncPlan {
                direction: item.direction,
                source: item.source,
                target: item.target,
            })
            .collect();

        // What the job does to an archive side: copies out of it, and copies into it or removals from it, which rewrite it.
        let archive_pane = match (&left, &right) {
            (Root::Archive { .. }, _) => Some(PaneId::Left),
            (_, Root::Archive { .. }) => Some(PaneId::Right),
            _ => None,
        };
        let into = |pane: PaneId| match pane {
            PaneId::Left => SyncDirection::ToLeft,
            PaneId::Right => SyncDirection::ToRight,
        };
        let from_archive: Vec<String> = archive_pane
            .map(|pane| {
                items
                    .iter()
                    .filter(|item| item.direction != into(pane))
                    .map(|item| item.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let rewrites = archive_pane.is_some_and(|pane| {
            items.iter().any(|item| item.direction == into(pane)) || removals.iter().any(|removal| removal.side == pane)
        });

        let removals: Vec<SyncRemove> = removals
            .iter()
            .map(|removal| SyncRemove {
                side: removal.side,
                seen: removal.seen,
                files: removal.files,
                bytes: removal.bytes,
                scanned: removal.scanned,
            })
            .collect();

        let (sides, from, to, held) = match (left, right) {
            (Root::Folder(from), Root::Folder(to)) => ((SyncRoot::Folder, SyncRoot::Folder), from, to, None),
            (Root::Archive { target, dir }, Root::Folder(to)) => {
                let archive = self
                    .archive_root(archives, (&target, dir), (&from_archive, rewrites), read.as_ref())
                    .await?;

                ((archive, SyncRoot::Folder), target.folder.clone(), to, Some(target))
            }
            (Root::Folder(to), Root::Archive { target, dir }) => {
                let archive = self
                    .archive_root(archives, (&target, dir), (&from_archive, rewrites), read.as_ref())
                    .await?;

                ((SyncRoot::Folder, archive), target.folder.clone(), to, Some(target))
            }
            (Root::Archive { .. }, Root::Archive { .. }) => unreachable!("refused above"),
        };
        let task = Task::Sync {
            left: sides.0,
            right: sides.1,
            plans,
            removals,
            permanent,
        };
        let request = Request::task(from, names, to, task);

        let Some(target) = held else {
            return self.engine.submit(request).map_err(failed);
        };

        // A password the job finds wrong while it decrypts is forgotten when the job ends (see `extractions`).
        let password = archive_password(&request);
        let submit = || self.engine.submit(request).map_err(failed);
        if rewrites {
            self.extractions.submit_rewrite(archives, target.path, password, submit)
        } else {
            self.extractions.submit(archives, target.path, password, submit)
        }
    }

    /// The archive side of the job: the folder `dir` of `target`, with what the job brings of the archive, which is read on a
    /// blocking thread: for a job that `rewrites` it, what adding files needs (its whole index, a format that is written, the
    /// password for encrypted entries, see [`prepare_rewrite`]); else what extracting the entries `names` needs (see
    /// [`prepare`]), and nothing when there are none (the job only removes on the other side).
    async fn archive_root(
        &self,
        archives: &Archives,
        (target, dir): (&Target, String),
        (names, rewrites): (&[String], bool),
        read: Option<&Fingerprint>,
    ) -> CommandResult<SyncRoot> {
        if rewrites {
            check_writable(target)?;
            check_folders(&dir)?;

            let permit = archives.begin()?;
            let shared = archives.clone();
            let (path, inner) = (target.clone(), dir.clone());
            let source = blocking(permit, move || prepare_rewrite(&shared, &path, &inner))
                .await?
                .map_err(failed)?;
            // The job checks the archive against this state at its start and before it replaces it: it must be the file
            // the comparison read, so what the user chose is what is in it.
            if read.is_none() || source.fingerprint.as_ref() != read {
                return Err(CommandError::new(
                    IpcErrorCode::InvalidInput,
                    "The archive changed since the comparison: compare again.",
                ));
            }

            return Ok(SyncRoot::Archive {
                name: target.name.clone(),
                inner: dir,
                source,
            });
        }

        if names.is_empty() {
            return Ok(SyncRoot::Archive {
                name: target.name.clone(),
                inner: dir,
                source: ArchiveJob::default(),
            });
        }

        let permit = archives.begin()?;
        let shared = archives.clone();
        let (path, chosen, inner) = (target.clone(), names.to_vec(), dir.clone());

        let source = blocking(permit, move || prepare(&shared, &path, &inner, &chosen))
            .await?
            .map_err(failed)?;

        Ok(SyncRoot::Archive {
            name: target.name.clone(),
            inner: dir,
            source,
        })
    }
}

/// The password of the archive of the sync job `request`, which the job was given.
fn archive_password(request: &Request) -> Option<std::sync::Arc<archive::Password>> {
    let Task::Sync { left, right, .. } = &request.task else {
        return None;
    };

    [left, right].into_iter().find_map(|root| match root {
        SyncRoot::Archive { source, .. } => source.password.clone(),
        SyncRoot::Folder => None,
    })
}

#[cfg(test)]
mod tests;

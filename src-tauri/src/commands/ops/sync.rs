//! `ops_sync`: the items a comparison chose copied between two folders as one job of the transfer engine.
//!
//! The command checks syntax only (the paths and the archive, like `ops_extract`) and the one rule the page may break by
//! asking: an archive is only copied from. The engine checks the items (paths below the folders, none inside another, each
//! plan a copy can carry out) and the job looks at every item again when it runs. For an archive the command also brings
//! what `ops_extract` brings: its index (kept, or built now for a zip or 7z), the state of the file it was made from and the
//! password the user gave, and fails with `passwordRequired` when the chosen entries are encrypted and there is none.
//!
//! Where the folders go in the request (see `Task::Sync`): two folders are `from` and `to` as they are given; with an archive
//! on one side, `from` is the folder that holds the archive and `to` the other folder, the one written to. The engine queues
//! jobs by the volume of `to` alone, so two syncs that write to one disk wait for each other whichever way they copy.

use pairpane_core::archive;
use pairpane_core::model::{CompareRoot, JobId, SyncDirection, SyncItem};
use pairpane_core::ops::{Request, SyncPlan, SyncRoot, Task};

use super::extract::prepare;
use super::{Ops, failed};
use crate::commands::archives::{Archives, Target};
use crate::commands::compare::{Root, refused};
use crate::commands::items::blocking;
use crate::error::CommandResult;

impl Ops {
    /// Checks the request, prepares what the job brings of the archive (on a blocking thread) and queues it.
    pub(crate) async fn sync(
        &self,
        archives: &Archives,
        (left, right): (CompareRoot, CompareRoot),
        items: Vec<SyncItem>,
    ) -> CommandResult<JobId> {
        let (left, right) = (Root::parse(left)?, Root::parse(right)?);
        if left.is_archive() && right.is_archive() {
            return Err(refused("Two archives cannot be compared."));
        }

        let names: Vec<String> = items.iter().map(|item| item.path.clone()).collect();
        let plans: Vec<SyncPlan> = items
            .iter()
            .map(|item| SyncPlan {
                direction: item.direction,
                source: item.source,
                target: item.target,
            })
            .collect();

        // The archive is the side the items are copied from, never to.
        let into_left = items.iter().any(|item| item.direction == SyncDirection::ToLeft);
        let into_right = items.iter().any(|item| item.direction == SyncDirection::ToRight);
        if (left.is_archive() && into_left) || (right.is_archive() && into_right) {
            return Err(refused("Archives can only be copied from."));
        }

        let (sides, from, to, held) = match (left, right) {
            (Root::Folder(from), Root::Folder(to)) => ((SyncRoot::Folder, SyncRoot::Folder), from, to, None),
            (Root::Archive { target, dir }, Root::Folder(to)) => {
                let archive = self.archive_root(archives, &target, dir, &names).await?;

                ((archive, SyncRoot::Folder), target.folder.clone(), to, Some(target))
            }
            (Root::Folder(to), Root::Archive { target, dir }) => {
                let archive = self.archive_root(archives, &target, dir, &names).await?;

                ((SyncRoot::Folder, archive), target.folder.clone(), to, Some(target))
            }
            (Root::Archive { .. }, Root::Archive { .. }) => unreachable!("refused above"),
        };
        let task = Task::Sync {
            left: sides.0,
            right: sides.1,
            plans,
        };
        let request = Request::task(from, names, to, task);

        let Some(target) = held else {
            return self.engine.submit(request).map_err(failed);
        };

        // A password the job finds wrong while it decrypts is forgotten when the job ends (see `extractions`).
        let password = archive_password(&request);
        self.extractions.submit(archives, target.path, password, || {
            self.engine.submit(request).map_err(failed)
        })
    }

    /// The archive side of the job: the folder `dir` of `target`, with what the job brings of the archive (see [`prepare`]),
    /// which is read on a blocking thread.
    async fn archive_root(
        &self,
        archives: &Archives,
        target: &Target,
        dir: String,
        names: &[String],
    ) -> CommandResult<SyncRoot> {
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

//! The undo of a synchronization: the one sync that can be put back, the holding folders that keep the files it replaced,
//! and the command that undoes it.
//!
//! The slot holds the most recent sync job (set when it is queued, so a later one replaces it) and its log (see
//! [`SyncLog`]). It lives in memory only. When it is replaced or the app quits, the log is released: its holding folders
//! go, with the files they kept, as soon as the sync and any undo of it are over. The page gets the token of an undo (the
//! job's id) from the job it started and asks for the undo once the job has ended; the undo is a job of its own, which
//! leaves what changed since the sync and counts it. An undo has no undo of its own.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use pairpane_core::model::JobId;
use pairpane_core::ops::{Engine, Request, SyncLog, Task};

use super::failed;
use super::rename_undo::{nothing, over};
use crate::error::CommandResult;

/// The sync job that can be undone.
struct Pending {
    job: JobId,
    /// A folder the job wrote to: the undo job is queued on its volume.
    folder: PathBuf,
    log: SyncLog,
}

/// The slot of the sync that can be undone.
#[derive(Default)]
pub(super) struct SyncUndo {
    slot: Mutex<Option<Pending>>,
}

impl SyncUndo {
    /// Queues the sync `request` in `engine` and takes the slot with it; the sync that held it is no longer undone, and
    /// its holding folders go once it is over (one that still runs keeps them; it cannot be undone either).
    pub(super) fn submit(&self, engine: &Engine, request: Request) -> CommandResult<JobId> {
        let Task::Sync { log, .. } = &request.task else {
            return engine.submit(request).map_err(failed);
        };
        let (folder, log) = (request.to.clone(), log.clone());

        let mut slot = self.lock();
        let job = engine.submit(request).map_err(failed)?;
        if let Some(old) = slot.replace(Pending { job, folder, log }) {
            old.log.release();
        }

        Ok(job)
    }

    /// Queues the job that puts the sync `job` back, and returns its id. The sync stays in the slot: an undo that is
    /// cancelled, fails or never runs keeps the old files (see `SyncLog`), and can be asked for again; one that went
    /// through to its end is not offered again.
    ///
    /// Refused as the rename's undo is (see `rename_undo`), also while an undo of it runs, and as not found when the sync
    /// did nothing that can be undone or was undone already.
    pub(super) fn undo(&self, engine: &Engine, job: JobId) -> CommandResult<JobId> {
        let mut slot = self.lock();
        let held = slot.as_ref().filter(|pending| pending.job == job);
        let pending = over(
            held,
            |kept| kept.log.finished() && !kept.log.undoing(),
            "The synchronization or its undo is still running.",
        )?;

        if pending.log.undone() || pending.log.undoable() == 0 {
            if let Some(old) = slot.take() {
                old.log.release();
            }

            return Err(nothing());
        }

        let back = Request::task(
            pending.folder.clone(),
            pending.log.names(),
            pending.folder.clone(),
            Task::SyncUndo {
                log: pending.log.clone(),
            },
        );

        engine.submit(back).map_err(failed)
    }

    /// Lets go of the sync in the slot, as the app quits: the undo is no longer offered, and the old files go (see
    /// `SyncLog::release`).
    pub(super) fn shut_down(&self) {
        if let Some(old) = self.lock().take() {
            old.log.release();
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

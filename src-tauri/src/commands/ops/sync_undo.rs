//! The undo of a synchronization: the one sync that can be put back, the holding folders that keep the files it replaced,
//! and the command that undoes it.
//!
//! The slot holds the most recent sync job (set when it is queued, so a later one replaces it) and its log (see
//! [`SyncLog`]). It lives in memory only. When it is replaced or the app quits, the log is released: its holding folders
//! go, with the files they kept, as soon as the sync and any undo of it are over. The page gets the token of an undo (the
//! job's id) from the job it started and asks for the undo once the job has ended; the undo is a job of its own, which
//! leaves what changed since the sync and counts it. An undo has no undo of its own.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use pairpane_core::model::{JobId, OpsEvent};
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

/// The slot of the sync that can be undone, and the undo jobs queued that have not ended yet (each with its log).
#[derive(Default)]
pub(super) struct SyncUndo {
    slot: Mutex<Option<Pending>>,
    undos: Mutex<HashMap<JobId, SyncLog>>,
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
        let old = slot.replace(Pending { job, folder, log });
        drop(slot);

        // Away from the slot and the command: removing the holding folders reaches a target folder, which may not answer.
        if let Some(old) = old {
            std::thread::spawn(move || old.log.release());
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
            let old = slot.take();
            drop(slot);
            if let Some(old) = old {
                old.log.release();
            }

            return Err(nothing());
        }

        let log = pending.log.clone();
        let back = Request::task(
            pending.folder.clone(),
            log.names(),
            pending.folder.clone(),
            Task::SyncUndo { log: log.clone() },
        );

        // Until the undo job ended (even one cancelled before it ran), nothing it needs goes, and no second one is queued.
        log.undo_queued();
        match engine.submit(back) {
            Ok(undo) => {
                self.undos().insert(undo, log);

                Ok(undo)
            }
            Err(refused) => {
                log.end_undo(false);

                Err(failed(refused))
            }
        }
    }

    /// Told of every event of the engine: an undo job that ended is over for its log, however it ended.
    pub(super) fn observe(&self, event: &OpsEvent) {
        if let OpsEvent::Ended { id, .. } = event
            && let Some(log) = self.undos().remove(id)
        {
            log.end_undo(false);
        }
    }

    /// Lets go of the sync in the slot, as the app quits: the undo is no longer offered, and the old files go (see
    /// `SyncLog::release`).
    pub(super) fn shut_down(&self) {
        if let Some(old) = self.lock().take() {
            old.log.release();
        }
    }

    fn undos(&self) -> MutexGuard<'_, HashMap<JobId, SyncLog>> {
        self.undos.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

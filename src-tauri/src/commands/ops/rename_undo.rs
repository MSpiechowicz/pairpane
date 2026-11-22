//! The undo of a multi-rename: the one batch that can be put back, and the command that does it.
//!
//! The slot holds the most recent rename job (set when it is submitted, so a later batch replaces it), the folder it
//! worked in and its log (see [`RenameLog`]). It lives in memory only, so nothing outlives the app, and it is the same on
//! every system. The page gets the token of an undo (the job's id) from the job it started and asks for the undo once
//! the job has ended; the undo is a job of its own that renames each pair back, skipping what changed since (see
//! `pairpane_core::ops::Task::Rename`, which refuses when the folder it opens is not the one the rename opened). The slot is cleared when that job is queued, and an undo has no undo of its own.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use pairpane_core::model::{IpcErrorCode, JobId};
use pairpane_core::ops::{Engine, RenameLog, Request, Task};

use super::failed;
use crate::error::{CommandError, CommandResult};

/// The rename job that can be undone.
struct Pending {
    job: JobId,
    folder: PathBuf,
    log: RenameLog,
}

/// The slot of the rename that can be undone.
#[derive(Default)]
pub(super) struct RenameUndo {
    slot: Mutex<Option<Pending>>,
}

impl RenameUndo {
    /// Queues `request` in `engine`; when it is a rename (not an undo) it takes the slot, which holds the most recent one
    /// only. The slot is held while the job is queued, so an undo asked for as soon as the job ended finds it.
    pub(super) fn submit(&self, engine: &Engine, request: Request) -> CommandResult<JobId> {
        let kept = match &request.task {
            Task::Rename { undo: false, log, .. } => Some((request.from.clone(), log.clone())),
            _ => None,
        };

        let mut slot = self.lock();
        let job = engine.submit(request).map_err(failed)?;
        if let Some((folder, log)) = kept {
            *slot = Some(Pending { job, folder, log });
        }

        Ok(job)
    }

    /// Queues the job that puts the rename `job` back, and returns its id; the slot is empty after that.
    ///
    /// Refused as not found when `job` is not the rename in the slot (it was replaced, or undone already, or never was
    /// one) or renamed nothing, and as invalid input, leaving the slot as it is, while that job still runs. A refusal of the
    /// engine (too many jobs) leaves the slot too, so the page can try again.
    pub(super) fn undo(&self, engine: &Engine, job: JobId) -> CommandResult<JobId> {
        let mut slot = self.lock();
        let Some(pending) = slot.as_ref().filter(|pending| pending.job == job) else {
            return Err(nothing());
        };

        if !pending.log.finished() {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "The rename is still running.",
            ));
        }

        let done = pending.log.done();
        if done.is_empty() {
            *slot = None;

            return Err(nothing());
        }

        // The last rename is put back first; each pair goes back from its new name to its old one.
        let (names, targets): (Vec<String>, Vec<String>) = done.into_iter().rev().map(|(from, to)| (to, from)).unzip();
        let back = Request::task(
            pending.folder.clone(),
            names,
            pending.folder.clone(),
            Task::Rename {
                targets,
                undo: true,
                log: pending.log.undoing(),
            },
        );
        let undone = engine.submit(back).map_err(failed)?;
        *slot = None;

        Ok(undone)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn nothing() -> CommandError {
    CommandError::new(IpcErrorCode::NotFound, "Nothing to undo.")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_rename_that_still_runs_is_not_undone_and_keeps_its_place() {
        let undo = RenameUndo::default();
        let engine = Engine::new();
        let folder = PathBuf::from(if cfg!(windows) { "C:\\data" } else { "/data" });
        *undo.lock() = Some(Pending {
            job: JobId(7),
            folder,
            log: RenameLog::default(),
        });

        let refused = undo.undo(&engine, JobId(7)).expect_err("still running");

        assert_eq!(
            serde_json::to_value(&refused).expect("serializable"),
            json!({ "code": "invalidInput", "message": "The rename is still running." })
        );
        assert!(undo.lock().is_some(), "the slot is kept");
    }

    #[test]
    fn the_slot_starts_empty() {
        let refused = RenameUndo::default()
            .undo(&Engine::new(), JobId(1))
            .expect_err("nothing");

        assert_eq!(
            serde_json::to_value(&refused).expect("serializable"),
            json!({ "code": "notFound", "message": "Nothing to undo." })
        );
    }
}

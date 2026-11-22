//! The extraction jobs that run with a password, so a password the archive turns down while the job decrypts is forgotten.
//!
//! The check before a job starts catches a password the directory of a zip or 7z turns down, but a password that only
//! fails on the data (an encrypted entry's checksum, a RAR is never given one) ends the job with `passwordRequired` and
//! the text "the password is wrong". Without this the stored password would be sent with the next try, and the page could
//! never ask for another one. The events the engine produces pass through [`Extractions::observe`] before they are
//! delivered, so the password is dropped before the page learns the job failed.
//!
//! [`Extractions::submit`] holds the lock from before the job is queued until it is registered, so a job that ends at
//! once waits to find its entry. The events produced on the thread that is submitting (the job being added, or a job
//! that could not be started at all) never look at the entries: that thread holds the lock, and neither is a password
//! failure.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::archive::{self, Password, PasswordFault};
use pairpane_core::model::{IpcError, IpcErrorCode, JobId, OpsEvent, Outcome};

use crate::commands::archives::Archives;
use crate::error::CommandResult;

thread_local! {
    /// Whether this thread is inside [`Extractions::submit`], holding the lock.
    static SUBMITTING: Cell<bool> = const { Cell::new(false) };
}

/// The archive and password of a running job.
struct Running {
    archives: Archives,
    archive: PathBuf,
    password: Arc<Password>,
}

/// The extraction jobs that were given a password, by id, until they end.
#[derive(Default)]
pub(super) struct Extractions {
    jobs: Mutex<HashMap<JobId, Running>>,
}

impl Extractions {
    fn lock(&self) -> MutexGuard<'_, HashMap<JobId, Running>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `submit`, which queues the job, and, when it was given `password` for the archive at `archive`, keeps that
    /// until the job ends. The lock is held throughout.
    pub(super) fn submit(
        &self,
        archives: &Archives,
        archive: PathBuf,
        password: Option<Arc<Password>>,
        submit: impl FnOnce() -> CommandResult<JobId>,
    ) -> CommandResult<JobId> {
        let mut jobs = self.lock();
        SUBMITTING.set(true);
        let queued = submit();
        SUBMITTING.set(false);

        let id = queued?;
        if let Some(password) = password {
            jobs.insert(
                id,
                Running {
                    archives: archives.clone(),
                    archive,
                    password,
                },
            );
        }

        Ok(id)
    }

    /// Looks at an event the engine produced: when a job ends, its entry goes, and when it ended because its password was
    /// wrong, the password is forgotten (unless the user gave another one since).
    pub(super) fn observe(&self, event: &OpsEvent) {
        let OpsEvent::Ended { id, outcome, .. } = event else {
            return;
        };
        if SUBMITTING.get() {
            return;
        }

        let Some(running) = self.lock().remove(id) else {
            return;
        };

        if let Outcome::Failed { error } = outcome
            && is_wrong_password(error)
        {
            running.archives.forget_used(&running.archive, &running.password);
        }
    }
}

/// Whether `error` is what a job ends with when the password it was given does not open the archive.
fn is_wrong_password(error: &IpcError) -> bool {
    error.code == IpcErrorCode::PasswordRequired && error.message == archive::fault_text(PasswordFault::Wrong)
}

#[cfg(test)]
mod tests;

//! The jobs on an archive that run with a password, so a password the archive turns down while the job decrypts is
//! forgotten, and the jobs that rewrite an archive, so the index kept for it goes when they end.
//!
//! The check before a job starts catches a password the directory of a zip or 7z turns down, but a password that only
//! fails on the data (an encrypted entry's checksum, a RAR is never given one) ends the job with `passwordRequired` and
//! the text "the password is wrong". Without this the stored password would be sent with the next try, and the page could
//! never ask for another one. The events the engine produces pass through [`Extractions::observe`] before they are
//! delivered, so the password is dropped before the page learns the job failed. A job that adds to an archive may replace
//! the file, however it ends, so the index kept for the archive is dropped with it and the next open reads it again.
//!
//! [`Extractions::submit`] holds the lock from before the job is queued until it is registered, so a job that ends at
//! once waits to find its entry. The events produced on the thread that is submitting (the job being added, a job that
//! could not be started at all, or a job that was waiting and ended because of the new one) cannot take the lock that thread
//! holds: they are set aside and settled, entry and password, once the job is registered and before `submit` returns.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::archive::{self, Password, PasswordFault};
use pairpane_core::model::{IpcError, IpcErrorCode, JobId, OpsEvent, Outcome};

use crate::commands::archives::Archives;
use crate::error::CommandResult;

/// A job that ended, and whether it ended because its password was wrong.
type Ended = (JobId, bool);

thread_local! {
    /// The jobs that ended on this thread while it is inside [`Extractions::submit`], holding the lock; `None` otherwise.
    static DEFERRED: RefCell<Option<Vec<Ended>>> = const { RefCell::new(None) };
}

/// A thread's stay inside [`Extractions::submit`]: from [`Submitting::begin`] the ends on this thread are set aside. It is
/// over when [`Submitting::end`] hands them back, or the guard is dropped (the submission panicked: the thread goes back
/// to settling its ends at once, and the ones set aside are lost with the panic).
struct Submitting;

impl Submitting {
    fn begin() -> Self {
        DEFERRED.with_borrow_mut(|deferred| *deferred = Some(Vec::new()));

        Self
    }

    fn end(self) -> Vec<Ended> {
        DEFERRED.with_borrow_mut(Option::take).unwrap_or_default()
    }
}

impl Drop for Submitting {
    fn drop(&mut self) {
        DEFERRED.with_borrow_mut(|deferred| *deferred = None);
    }
}

/// Sets `ended` aside when this thread is inside [`Extractions::submit`]; says whether it did.
fn defer(ended: Ended) -> bool {
    DEFERRED.with_borrow_mut(|deferred| match deferred {
        Some(list) => {
            list.push(ended);

            true
        }
        None => false,
    })
}

/// The archive of a running job, the password it was given and whether it rewrites the archive.
struct Running {
    archives: Archives,
    archive: PathBuf,
    password: Option<Arc<Password>>,
    rewrites: bool,
}

/// The jobs that were given a password or that rewrite their archive, by id, until they end.
#[derive(Default)]
pub(super) struct Extractions {
    jobs: Mutex<HashMap<JobId, Running>>,
}

impl Extractions {
    fn lock(&self) -> MutexGuard<'_, HashMap<JobId, Running>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `submit`, which queues the job, and, when it was given `password` for the archive at `archive`, keeps that
    /// until the job ends. The lock is held throughout; the ends that took place on this thread meanwhile are settled before it
    /// returns, even when `submit` failed. A `submit` that panics leaves the thread as it was.
    pub(super) fn submit(
        &self,
        archives: &Archives,
        archive: PathBuf,
        password: Option<Arc<Password>>,
        submit: impl FnOnce() -> CommandResult<JobId>,
    ) -> CommandResult<JobId> {
        self.track(archives, archive, password, false, submit)
    }

    /// [`Extractions::submit`] for a job that may replace the archive: the index kept for the archive goes when the job
    /// ends, however it ends, with or without a password.
    pub(super) fn submit_rewrite(
        &self,
        archives: &Archives,
        archive: PathBuf,
        password: Option<Arc<Password>>,
        submit: impl FnOnce() -> CommandResult<JobId>,
    ) -> CommandResult<JobId> {
        self.track(archives, archive, password, true, submit)
    }

    fn track(
        &self,
        archives: &Archives,
        archive: PathBuf,
        password: Option<Arc<Password>>,
        rewrites: bool,
        submit: impl FnOnce() -> CommandResult<JobId>,
    ) -> CommandResult<JobId> {
        let mut jobs = self.lock();
        let submitting = Submitting::begin();
        let queued = submit();
        let deferred = submitting.end();

        if let Ok(id) = &queued
            && (password.is_some() || rewrites)
        {
            jobs.insert(
                *id,
                Running {
                    archives: archives.clone(),
                    archive,
                    password,
                    rewrites,
                },
            );
        }

        // The job just registered may be one of those that ended meanwhile.
        let settled = settle(&mut jobs, deferred);
        drop(jobs);
        forget_all(settled);

        queued
    }

    /// Looks at an event the engine produced: when a job ends, its entry goes, and when it ended because its password was
    /// wrong, the password is forgotten (unless the user gave another one since). On the thread that is submitting a job
    /// this waits until the submission is done (see the module documentation).
    pub(super) fn observe(&self, event: &OpsEvent) {
        let OpsEvent::Ended { id, outcome, .. } = event else {
            return;
        };
        let ended = (
            *id,
            matches!(outcome, Outcome::Failed { error } if is_wrong_password(error)),
        );

        if defer(ended) {
            return;
        }

        let settled = settle(&mut self.lock(), vec![ended]);
        forget_all(settled);
    }
}

/// Takes the entries of the jobs `ended` out of `jobs`, each with whether it ended with a wrong password.
fn settle(jobs: &mut HashMap<JobId, Running>, ended: Vec<Ended>) -> Vec<(Running, bool)> {
    ended
        .into_iter()
        .filter_map(|(id, wrong)| jobs.remove(&id).map(|running| (running, wrong)))
        .collect()
}

/// Forgets what the jobs that ended leave stale, with no lock of the entries held: the index of an archive a job may have
/// replaced, and the password of a job that ended with it wrong (unless the user gave another one since).
fn forget_all(jobs: Vec<(Running, bool)>) {
    for (running, wrong) in jobs {
        if running.rewrites {
            running.archives.forget_index(&running.archive);
        }

        if wrong && let Some(used) = &running.password {
            running.archives.forget_used(&running.archive, used);
        }
    }
}

/// Whether `error` is what a job ends with when the password it was given does not open the archive.
fn is_wrong_password(error: &IpcError) -> bool {
    error.code == IpcErrorCode::PasswordRequired && error.message == archive::fault_text(PasswordFault::Wrong)
}

#[cfg(test)]
mod tests;

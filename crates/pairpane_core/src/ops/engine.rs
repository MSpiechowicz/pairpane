//! The queue and the workers.
//!
//! Jobs wait in one list in submit order. A worker takes the oldest job whose volume is free, runs it to its end
//! and looks again, so jobs for one volume run one after another in submit order and jobs for different volumes
//! run side by side, up to [`MAX_WORKERS`] at once. A worker that finds nothing to do exits.
//!
//! The volume of a job needs the disk, which `submit` must not touch, so the workers find it: a worker that has
//! nothing to run looks up the volume of the oldest job that has none and nobody is looking up. Lookups run side
//! by side, so a folder that does not answer holds its own worker (like any hung mount) and nothing else: the
//! jobs behind it start as soon as their volumes are known. One consequence: a job whose lookup is still going on
//! does not hold back a later job for the same volume, so the submit order of two jobs is only certain for jobs
//! whose volumes are both known when the first one is ready (an instant, unless a folder hangs).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::model::{Answer, ControlAction, IpcError, IpcErrorCode, JobId, OpsEvent};

use super::control::locked;
use super::hooks::Hooks;
use super::job::Job;
use super::transfer::{self, Finish};
use super::validate::validate;
use super::volume::VolumeKey;
use super::{AnswerError, MAX_JOBS, MAX_WORKERS, Refused, Request, Sink, UnknownJob};

/// Stack of a worker: a folder tree is walked recursively, up to [`super::transfer::MAX_DEPTH`] levels.
const WORKER_STACK: usize = 8 * 1024 * 1024;

/// A job that has not started, and the volume it will use once the workers know it.
struct Waiting {
    job: Arc<Job>,
    key: Option<VolumeKey>,
    /// A worker is looking up the volume right now.
    resolving: bool,
}

#[derive(Default)]
pub(super) struct State {
    last_id: u64,
    /// Every job that has not ended, queued or running.
    jobs: BTreeMap<JobId, Arc<Job>>,
    /// The jobs that have not started, oldest first.
    waiting: VecDeque<Waiting>,
    /// Volumes with a running job.
    busy: HashSet<VolumeKey>,
    workers: usize,
}

impl State {
    /// Whether `job` is still in the queue, not started.
    pub(super) fn is_queued(&self, job: &Arc<Job>) -> bool {
        self.waiting.iter().any(|waiting| Arc::ptr_eq(&waiting.job, job))
    }
}

/// What the engine shares with its workers.
pub(super) struct Inner {
    sink: Mutex<Option<Sink>>,
    state: Mutex<State>,
    /// Signalled when the last worker is gone.
    idle: Condvar,
    pub(super) hooks: Hooks,
}

impl Inner {
    pub(super) fn state(&self) -> MutexGuard<'_, State> {
        locked(&self.state)
    }

    /// Sends `event` to the sink with no lock held, so the sink may call back into the engine. A sink that
    /// panics does not take the worker down with it.
    pub(super) fn emit(&self, event: OpsEvent) {
        let sink = locked(&self.sink).clone();

        if let Some(sink) = sink
            && panic::catch_unwind(AssertUnwindSafe(|| sink(event))).is_err()
        {
            tracing::error!("the transfers listener panicked");
        }
    }

    pub(super) fn job(&self, id: JobId) -> Result<Arc<Job>, UnknownJob> {
        self.state().jobs.get(&id).cloned().ok_or(UnknownJob)
    }

    /// Puts a job at the end of the queue and makes sure a worker is there to take it.
    fn enqueue(self: &Arc<Self>, job: &Arc<Job>) {
        let mut state = self.state();
        state.waiting.push_back(Waiting {
            job: Arc::clone(job),
            key: None,
            resolving: false,
        });

        if state.workers >= MAX_WORKERS || self.start_worker(&mut state).is_ok() {
            return;
        }

        // With no worker at all nothing would ever take the queue: end its jobs instead of leaving them waiting.
        if state.workers > 0 {
            return;
        }
        state.jobs.clear();
        let stranded: Vec<Waiting> = state.waiting.drain(..).collect();
        drop(state);

        for waiting in stranded {
            let error = IpcError::new(IpcErrorCode::Io, "could not start the transfer");
            self.send(&waiting.job, ended(&waiting.job, Finish::failed(error)), true);
        }
    }

    /// Starts one more worker. The caller has checked that [`MAX_WORKERS`] is not reached yet.
    fn start_worker(self: &Arc<Self>, state: &mut State) -> io::Result<()> {
        let inner = Arc::clone(self);
        thread::Builder::new()
            .name("pairpane-ops".to_owned())
            .stack_size(WORKER_STACK)
            .spawn(move || inner.work())
            .inspect_err(|error| tracing::error!(%error, "could not start a transfer worker"))?;
        state.workers += 1;

        Ok(())
    }

    /// A worker: runs jobs until none is left that it may take.
    fn work(self: &Arc<Self>) {
        while let Some((job, key)) = self.next() {
            let finish = panic::catch_unwind(AssertUnwindSafe(|| transfer::execute(self, &job))).unwrap_or_else(|_| {
                Finish::failed(IpcError::new(
                    IpcErrorCode::Internal,
                    "the transfer stopped unexpectedly",
                ))
            });

            {
                let mut state = self.state();
                state.jobs.remove(&job.id);
                state.busy.remove(&key);
            }

            self.send(&job, ended(&job, finish), true);
        }
    }

    /// The oldest job whose volume is free, claimed for the calling worker; `None` (and the worker is gone from
    /// the count) when there is nothing it may take now.
    fn next(self: &Arc<Self>) -> Option<(Arc<Job>, VolumeKey)> {
        loop {
            let mut guard = self.state();
            let state = &mut *guard;

            let unresolved = state
                .waiting
                .iter_mut()
                .find(|waiting| waiting.key.is_none() && !waiting.resolving);

            if let Some(waiting) = unresolved {
                waiting.resolving = true;
                let job = Arc::clone(&waiting.job);
                drop(guard);
                self.resolve(&job);
                continue;
            }

            let free = state
                .waiting
                .iter()
                .position(|waiting| waiting.key.as_ref().is_some_and(|key| !state.busy.contains(key)));

            let Some(index) = free else {
                state.workers -= 1;
                if state.workers == 0 {
                    self.idle.notify_all();
                }

                return None;
            };
            let waiting = state.waiting.remove(index)?;
            let key = waiting.key?;
            state.busy.insert(key.clone());

            // Other jobs may be ready to start, and the workers that came for them may be gone (they saw nothing
            // yet while this one was still looking up volumes): bring one for them.
            let more = state
                .waiting
                .iter()
                .any(|waiting| waiting.key.as_ref().is_some_and(|key| !state.busy.contains(key)));
            if more && state.workers < MAX_WORKERS {
                let _ = self.start_worker(state);
            }

            return Some((waiting.job, key));
        }
    }

    /// Looks up the volume of `job`, which the caller has marked as being resolved. A lookup that panics gives
    /// [`VolumeKey::Unknown`] instead of leaving the job marked for ever (and the worker dead).
    fn resolve(&self, job: &Arc<Job>) {
        let key = panic::catch_unwind(AssertUnwindSafe(|| self.hooks.key_of(&job.request.to))).unwrap_or_else(|_| {
            tracing::error!("looking up the volume of a transfer panicked");

            VolumeKey::Unknown
        });

        let mut state = self.state();

        // A job cancelled while its volume was looked up is gone from the queue already.
        if let Some(waiting) = state.waiting.iter_mut().find(|waiting| Arc::ptr_eq(&waiting.job, job)) {
            waiting.key = Some(key);
            waiting.resolving = false;
        }
    }

    /// Cancels `jobs`: a running one stops at its next checkpoint and ends on its worker, and one that has not
    /// started ends here and now, as it holds no worker.
    fn cancel(&self, jobs: &[Arc<Job>]) {
        self.announce_stopping(jobs);

        // All at once, under the queue's lock: a worker that frees up meanwhile cannot start a job that is not
        // cancelled yet.
        let unstarted: Vec<&Arc<Job>> = {
            let mut state = self.state();

            jobs.iter()
                .filter(|job| {
                    job.control.cancel();
                    Self::unqueue(&mut state, job)
                })
                .collect()
        };

        for job in unstarted {
            self.send(job, ended(job, Finish::cancelled()), true);
        }
    }

    /// Takes `job` out of the queue when it has not started; `false` when a worker has it.
    fn unqueue(state: &mut State, job: &Arc<Job>) -> bool {
        let Some(index) = state.waiting.iter().position(|waiting| Arc::ptr_eq(&waiting.job, job)) else {
            return false;
        };
        state.waiting.remove(index);
        state.jobs.remove(&job.id);

        true
    }
}

fn ended(job: &Job, finish: Finish) -> OpsEvent {
    OpsEvent::Ended {
        id: job.id,
        outcome: finish.outcome,
        skipped: finish.skipped,
        errors: finish.errors,
        more_errors: finish.more_errors,
    }
}

/// The queue and the workers. Cheap to clone; every clone is the same engine.
#[derive(Clone)]
pub struct Engine {
    pub(super) inner: Arc<Inner>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with no jobs and no sink. Starts no thread; workers start when a job is submitted.
    pub fn new() -> Self {
        Self::with_hooks(Hooks::default())
    }

    pub(super) fn with_hooks(hooks: Hooks) -> Self {
        Self {
            inner: Arc::new(Inner {
                sink: Mutex::new(None),
                state: Mutex::new(State::default()),
                idle: Condvar::new(),
                hooks,
            }),
        }
    }

    /// How many workers are running right now.
    #[cfg(test)]
    pub(super) fn workers(&self) -> usize {
        self.inner.state().workers
    }

    /// Where events go from now on, replacing the earlier sink. Set it before `submit`: events produced while
    /// there is no sink are dropped. Does not send anything; call [`Engine::snapshot`] for the current state.
    pub fn set_sink(&self, sink: Sink) {
        *locked(&self.inner.sink) = Some(sink);
    }

    /// Queues a job and returns its id at once, without touching the disk. [`OpsEvent::Added`] reaches the
    /// sink before this returns, and before any other event of the job (a snapshot taken in between lists the
    /// job already, so a listener treats `added` for a job it knows as the same job).
    ///
    /// Refuses with [`Refused::InvalidNames`] for no names, more than [`super::MAX_NAMES`], a duplicate name or a
    /// name that is not a plain file name, with [`Refused::NamesTooLarge`] for a name or a list of names that is
    /// too long, with [`Refused::InvalidTask`] when the task does not fit the request (see [`Request::task`]), and with
    /// [`Refused::Busy`] when [`MAX_JOBS`] jobs exist.
    pub fn submit(&self, request: Request) -> Result<JobId, Refused> {
        validate(&request)?;

        let job = {
            let mut state = self.inner.state();

            if state.jobs.len() >= MAX_JOBS {
                return Err(Refused::Busy);
            }
            state.last_id += 1;

            let job = Arc::new(Job::new(JobId(state.last_id), request));
            state.jobs.insert(job.id, Arc::clone(&job));
            job
        };

        let view = job.view().clone();
        self.inner.emit(OpsEvent::Added { job: view });
        self.inner.enqueue(&job);

        Ok(job.id)
    }

    /// Pauses, resumes or cancels a job. Pausing a paused job, or resuming a running one, succeeds and changes
    /// nothing. Cancelling also ends a wait for an answer. Never blocks on the job's I/O.
    ///
    /// # Errors
    ///
    /// [`UnknownJob`] when the job never existed or has ended.
    pub fn control(&self, job: JobId, action: ControlAction) -> Result<(), UnknownJob> {
        let job = self.inner.job(job)?;

        match action {
            ControlAction::Cancel => self.inner.cancel(&[job]),
            ControlAction::Pause => {
                job.control.pause();
                self.inner.announce_paused(&job);
            }
            ControlAction::Resume => {
                // Announced before the job is let go, so no event of its later work (down to its end) can come
                // before it.
                job.control.unpause();
                self.inner.announce_paused(&job);
                job.control.wake();
            }
        }

        Ok(())
    }

    /// Answers the open question `question` of `job`. With `all`, the same choice is applied to every later
    /// conflict of the job without asking. `Answer::Compare` is not final: it keeps the question open and
    /// reports [`OpsEvent::Compared`], and `all` is ignored for it.
    ///
    /// # Errors
    ///
    /// [`AnswerError::UnknownJob`] for a job that is not running, [`AnswerError::StaleQuestion`] when `question`
    /// is not the job's open question.
    pub fn answer(&self, job: JobId, question: u32, answer: Answer, all: bool) -> Result<(), AnswerError> {
        let job = self.inner.job(job).map_err(|_| AnswerError::UnknownJob)?;

        job.control.answer(question, answer, all)
    }

    /// [`OpsEvent::Snapshot`] of the jobs that have not ended and the questions waiting for an answer, in
    /// submit order. Send it to a listener that has just subscribed.
    pub fn snapshot(&self) -> OpsEvent {
        let jobs: Vec<Arc<Job>> = self.inner.state().jobs.values().cloned().collect();

        OpsEvent::Snapshot {
            questions: jobs.iter().filter_map(|job| job.control.open_question()).collect(),
            jobs: jobs.iter().map(|job| job.view().clone()).collect(),
        }
    }

    /// Cancels every job. Does not wait for the workers; each job still cleans up after itself and ends with
    /// [`OpsEvent::Ended`].
    pub fn cancel_all(&self) {
        let jobs: Vec<Arc<Job>> = self.inner.state().jobs.values().cloned().collect();

        self.inner.cancel(&jobs);
    }

    /// Cancels every job, as the app quits, and waits up to `timeout` for the workers to finish cleaning up
    /// (removing partial files and the folders they made). Their [`OpsEvent::Ended`] is queued by then, though a
    /// slow sink on another thread may still be delivering it. Returns at once
    /// when no worker is running. `false` when a worker is still busy after `timeout`: one stuck on a folder
    /// that does not answer, which the app cannot wait for.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        self.cancel_all();

        let deadline = Instant::now() + timeout;
        let mut state = self.inner.state();

        while state.workers > 0 {
            let left = deadline.saturating_duration_since(Instant::now());

            if left.is_zero() {
                return false;
            }
            state = self
                .inner
                .idle
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }

        true
    }

    /// Whether `job` is paused, as its control says.
    #[cfg(test)]
    pub(super) fn is_paused(&self, job: JobId) -> bool {
        self.inner.job(job).is_ok_and(|job| job.control.is_paused())
    }
}

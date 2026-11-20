//! Telling the listener about a job from the threads that pause, resume and cancel it, without ever speaking after
//! its `ended`.
//!
//! Every event of that kind goes through the job's outbox: one thread at a time empties it, in the order the events
//! went in, and the `ended` closes it under the same lock that lets an event in. So an event is either let in before
//! the `ended` (and sent before it) or refused, however the threads race, and a sink that calls back into the engine
//! cannot deadlock (its event waits in the outbox for the thread that is sending already).

use std::sync::Arc;

use crate::model::{JobPhase, OpsEvent};

use super::control::locked;
use super::engine::Inner;
use super::job::Job;

impl Inner {
    /// Sends `event` of `job` in order with its other announced events, unless the job has ended. `last` closes the
    /// outbox: the event is the job's `ended`, and no other follows it. Returns whether `event` was let in.
    pub(super) fn send(&self, job: &Job, event: OpsEvent, last: bool) -> bool {
        {
            let mut outbox = locked(&job.outbox);

            if outbox.closed {
                return false;
            }
            outbox.closed = last;
            outbox.events.push_back(event);

            if outbox.sending {
                return true;
            }
            outbox.sending = true;
        }

        loop {
            let next = {
                let mut outbox = locked(&job.outbox);

                match outbox.events.pop_front() {
                    Some(next) => next,
                    None => {
                        outbox.sending = false;

                        return true;
                    }
                }
            };

            self.emit(next);
        }
    }

    /// Tells the listener that the running ones among `jobs` are about to stop. This goes out before the cancel
    /// itself, so no `ended` of theirs can come before it.
    pub(super) fn announce_stopping(&self, jobs: &[Arc<Job>]) {
        let running: Vec<&Arc<Job>> = {
            let state = self.state();

            jobs.iter().filter(|job| !state.is_queued(job)).collect()
        };

        for job in running.into_iter().filter(|job| !job.control.is_cancelled()) {
            let event = OpsEvent::Phase {
                id: job.id,
                phase: JobPhase::Cancelling,
            };

            job.view().phase = JobPhase::Cancelling;
            self.send(job, event, false);
        }
    }

    /// Tells the listener whether `job` is paused, until what it has been told is what is true.
    ///
    /// Pauses and resumes can arrive from several threads at once. The value sent is read when it is sent, and
    /// only one thread sends at a time (a thread that finds another one sending leaves it the work: that one
    /// looks again before it stops), so the last event the listener gets is always the job's current state. A
    /// sink that calls back with a pause or a resume is such a thread too, and cannot deadlock.
    pub(super) fn announce_paused(&self, job: &Job) {
        {
            let mut sending = locked(&job.announcing);

            if *sending {
                return;
            }
            *sending = true;
        }

        loop {
            let paused = job.control.is_paused();
            let changed = std::mem::replace(&mut job.view().paused, paused) != paused;

            if changed {
                self.send(job, OpsEvent::Paused { id: job.id, paused }, false);
            }

            // Under the same lock another thread takes to leave its change to us: it either sees `sending` and
            // we see its change here, or we have stopped and it sends itself.
            let mut sending = locked(&job.announcing);

            if job.control.is_paused() == job.view().paused {
                *sending = false;

                return;
            }
        }
    }
}

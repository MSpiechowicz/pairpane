//! What the user can do to a running job, and how the worker notices it: pause, cancel and the answer to a
//! conflict question.
//!
//! One mutex and one condvar. Every change (pause, resume, cancel, answer) calls `notify_all`, so a worker that
//! sleeps for a resume or an answer always wakes for a cancel, and the wait needs no timeout.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::model::{Answer, ConflictInfo};

use super::AnswerError;

/// The job was cancelled; every long step returns this and unwinds to the worker, which cleans up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cancelled;

/// The user's reply to a question, as it reaches the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reply {
    pub(super) answer: Answer,
    /// Keep this answer for every later conflict of the job.
    pub(super) all: bool,
}

#[derive(Default)]
struct State {
    paused: bool,
    cancelled: bool,
    /// The question the worker waits on; stays set until its final answer.
    open: Option<ConflictInfo>,
    reply: Option<Reply>,
    /// Questions asked so far; the number of the next one is this plus one.
    asked: u32,
}

/// Pause, cancel and conflict answers of one job.
#[derive(Default)]
pub(super) struct Control {
    state: Mutex<State>,
    changed: Condvar,
}

/// Locks `mutex`, going on with its data if another thread panicked while holding it: every state here stays
/// valid at each step, and a panic in one job must not take the engine down.
pub(super) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Control {
    fn state(&self) -> MutexGuard<'_, State> {
        locked(&self.state)
    }

    /// Holds the job at its next checkpoint. `false` when it was paused already.
    pub(super) fn pause(&self) -> bool {
        !std::mem::replace(&mut self.state().paused, true)
    }

    /// Clears the pause without waking the job: a sleeping worker stays asleep until [`Control::wake`], so the
    /// engine can announce the resume first. `false` when it was not paused.
    pub(super) fn unpause(&self) -> bool {
        std::mem::replace(&mut self.state().paused, false)
    }

    /// Wakes a worker that sleeps in a pause or a question, to look at what changed.
    pub(super) fn wake(&self) {
        self.changed.notify_all();
    }

    /// Stops the job at its next checkpoint, waking it from a pause or a question. `false` when it was
    /// cancelled already.
    pub(super) fn cancel(&self) -> bool {
        let first = !std::mem::replace(&mut self.state().cancelled, true);
        self.changed.notify_all();

        first
    }

    pub(super) fn is_paused(&self) -> bool {
        self.state().paused
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.state().cancelled
    }

    /// Called between chunks and between items: blocks while the job is paused, and fails once it is cancelled.
    /// Returns how long it was held, so the speed can leave that time out.
    pub(super) fn checkpoint(&self) -> Result<Duration, Cancelled> {
        let mut state = self.state();

        if state.cancelled {
            return Err(Cancelled);
        }
        if !state.paused {
            return Ok(Duration::ZERO);
        }

        let started = Instant::now();
        while state.paused && !state.cancelled {
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }

        if state.cancelled {
            return Err(Cancelled);
        }

        Ok(started.elapsed())
    }

    /// Opens a question: numbers it and keeps it for [`Control::wait_reply`] and snapshots. Returns the question
    /// with its number set.
    pub(super) fn open(&self, mut question: ConflictInfo) -> ConflictInfo {
        let mut state = self.state();
        state.asked += 1;
        question.question = state.asked;
        state.reply = None;
        state.open = Some(question.clone());

        question
    }

    /// Waits, without a timeout, for the reply to the open question or for a cancel. Returns the reply and how
    /// long it took.
    pub(super) fn wait_reply(&self) -> Result<(Reply, Duration), Cancelled> {
        let mut state = self.state();
        let started = Instant::now();

        loop {
            if state.cancelled {
                return Err(Cancelled);
            }

            if let Some(reply) = state.reply.take() {
                return Ok((reply, started.elapsed()));
            }

            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The open question, for a snapshot.
    pub(super) fn open_question(&self) -> Option<ConflictInfo> {
        self.state().open.clone()
    }

    /// Gives the reply to question `question`. A final answer closes the question at once, so a second answer
    /// to it is stale; `compare` keeps it open.
    pub(super) fn answer(&self, question: u32, answer: Answer, all: bool) -> Result<(), AnswerError> {
        let mut state = self.state();

        if state.open.as_ref().map(|open| open.question) != Some(question) {
            return Err(AnswerError::StaleQuestion);
        }

        if answer == Answer::Compare {
            state.reply = Some(Reply { answer, all: false });
        } else {
            state.reply = Some(Reply { answer, all });
            state.open = None;
        }
        self.changed.notify_all();

        Ok(())
    }

    /// Closes the open question without an answer (the job was cancelled while it waited).
    pub(super) fn close(&self) {
        self.state().open = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;

    use pretty_assertions::assert_eq;

    use crate::model::{FileFacts, JobId};

    use super::*;

    /// How long a thread that must stay blocked is given to prove it does.
    const STAYS_BLOCKED: Duration = Duration::from_millis(150);

    const SOON: Duration = Duration::from_secs(10);

    fn question() -> ConflictInfo {
        let facts = FileFacts {
            size: 1,
            modified: None,
            dir: false,
        };

        ConflictInfo {
            job: JobId(1),
            question: 0,
            name: "a.txt".to_owned(),
            source: facts.clone(),
            target: facts,
            suggested: "a (2).txt".to_owned(),
            comparable: true,
        }
    }

    /// Runs `work` on a thread; `started` fires right before it, and its result arrives on the second channel.
    fn spawn<T: Send + 'static>(
        control: &Arc<Control>,
        work: impl FnOnce(&Control) -> T + Send + 'static,
    ) -> (mpsc::Receiver<()>, mpsc::Receiver<T>) {
        let (started_tx, started) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let control = Arc::clone(control);

        thread::spawn(move || {
            started_tx.send(()).expect("receiver is alive");
            done_tx.send(work(&control)).expect("receiver is alive");
        });

        (started, done)
    }

    /// A paused control, and the result of a checkpoint that a thread has just reached.
    fn paused_checkpoint() -> (Arc<Control>, mpsc::Receiver<Result<Duration, Cancelled>>) {
        let control = Arc::new(Control::default());
        control.pause();
        let (started, done) = spawn(&control, |control| control.checkpoint());
        started.recv_timeout(SOON).expect("started");

        (control, done)
    }

    #[test]
    fn a_checkpoint_passes_until_the_job_is_paused() {
        let control = Control::default();

        assert!(control.checkpoint().is_ok());
        assert!(control.pause());
        assert!(!control.pause());
        assert!(control.unpause());
        assert!(!control.unpause());
        assert!(control.checkpoint().is_ok());
    }

    #[test]
    fn a_paused_checkpoint_blocks_until_resume() {
        let (control, done) = paused_checkpoint();

        assert!(done.recv_timeout(STAYS_BLOCKED).is_err(), "returned while paused");
        control.unpause();
        control.wake();

        let held = done.recv_timeout(SOON).expect("woke").expect("not cancelled");
        assert!(held >= STAYS_BLOCKED / 2);
    }

    #[test]
    fn cancel_wakes_a_paused_checkpoint() {
        let (control, done) = paused_checkpoint();

        assert!(control.cancel());
        assert!(!control.cancel());

        assert_eq!(done.recv_timeout(SOON).expect("woke"), Err(Cancelled));
        assert!(control.is_cancelled());
    }

    #[test]
    fn a_final_answer_closes_the_question_and_reaches_the_waiter() {
        let control = Arc::new(Control::default());
        let opened = control.open(question());
        let (started, done) = spawn(&control, |control| control.wait_reply().map(|(reply, _)| reply));
        started.recv_timeout(SOON).expect("started");

        assert_eq!(control.answer(opened.question, Answer::Skip, true), Ok(()));

        let reply = done.recv_timeout(SOON).expect("woke").expect("not cancelled");
        assert_eq!(
            reply,
            Reply {
                answer: Answer::Skip,
                all: true
            }
        );
        assert_eq!(control.open_question(), None);
        assert_eq!(
            control.answer(opened.question, Answer::Skip, false),
            Err(AnswerError::StaleQuestion)
        );
    }

    #[test]
    fn compare_keeps_the_question_open_and_ignores_all() {
        let control = Control::default();
        let opened = control.open(question());

        assert_eq!(control.answer(opened.question, Answer::Compare, true), Ok(()));

        let (reply, _) = control.wait_reply().expect("a reply is waiting");
        assert_eq!(
            reply,
            Reply {
                answer: Answer::Compare,
                all: false
            }
        );
        assert_eq!(control.open_question(), Some(opened));
    }

    #[test]
    fn questions_are_numbered_from_one_and_other_numbers_are_stale() {
        let control = Control::default();

        assert_eq!(control.answer(1, Answer::Skip, false), Err(AnswerError::StaleQuestion));
        let first = control.open(question());
        assert_eq!(first.question, 1);
        assert_eq!(control.answer(2, Answer::Skip, false), Err(AnswerError::StaleQuestion));
        assert_eq!(control.answer(1, Answer::Skip, false), Ok(()));
        assert_eq!(control.open(question()).question, 2);
    }

    #[test]
    fn cancel_wakes_a_worker_waiting_for_an_answer() {
        let control = Arc::new(Control::default());
        control.open(question());
        let (started, done) = spawn(&control, |control| control.wait_reply().map(|(reply, _)| reply));
        started.recv_timeout(SOON).expect("started");

        control.cancel();

        assert_eq!(done.recv_timeout(SOON).expect("woke"), Err(Cancelled));
    }
}

//! The deadline of a git status. A status is bounded by its own time limit, which gix polls between entries; but a read that
//! the operating system never ends (an open of a named pipe that was swapped in after the repository was looked at, a mount
//! that does not answer) never polls anything. So a status is not run on the thread that answers the request: it runs on a
//! thread of its own, and the request waits for it for the time limit and a short grace, and answers when that is over
//! whether the thread has come back or not. A thread that did not come back is counted as stuck until it ends (it cannot be
//! stopped), and while [`MAX_STUCK`] of them are, no more are started: a page that keeps asking about a repository that
//! holds a thread cannot fill the process with them.
//!
//! The count is taken before the thread is started and not after it was given up on: a status holds a place in it (a
//! [`Reservation`]) for as long as it runs, and keeps it as a stuck one when it is abandoned. So the number of statuses that
//! run or are stuck never goes over [`MAX_STUCK`], even when several are started in the same moment (counted after the fact,
//! two that were let in with one place left would both end up stuck).
//!
//! Why [`MAX_STUCK`] is small: a thread that does not come back can be one that holds a read that is not bounded by anything
//! (a pipe, a mount that does not answer) and that costs nothing, or one that spends the processor on work that cannot be
//! interrupted (a long chain of deltas in one object: gix copies, and has no point at which it looks at a flag). A thread of the
//! second kind burns a core until it is done, so no more than [`MAX_STUCK`] of them may burn at once.

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// How many threads of a status may be stuck before a new status is refused as busy.
pub(super) const MAX_STUCK: usize = 2;

const RUNNING: u8 = 0;
const FINISHED: u8 = 1;
const ABANDONED: u8 = 2;

/// Why work did not answer.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unanswered {
    /// It was not done at the deadline; its thread goes on and counts as stuck until it ends.
    TimedOut,
    /// Its thread could not be started, or ended without an answer (it panicked).
    Failed,
    /// [`MAX_STUCK`] statuses run or are stuck: nothing was started.
    Busy,
}

/// A place in the count of the statuses that run or are stuck, held while the status runs. It is given back when it is dropped,
/// unless it was [kept](Reservation::keep) because the status was given up on, and then the thread gives it back as it ends.
struct Reservation {
    count: Arc<AtomicUsize>,
    kept: bool,
}

impl Reservation {
    /// Takes a place, if one is left: the check and the count are one step.
    fn take(count: &Arc<AtomicUsize>) -> Option<Self> {
        count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |taken| {
                (taken < MAX_STUCK).then_some(taken + 1)
            })
            .ok()
            .map(|_| Self {
                count: Arc::clone(count),
                kept: false,
            })
    }

    /// The place stays taken after this is dropped.
    fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.kept {
            self.count.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// What a thread of a status does as it ends, however it ends (a panic too): it says it is done, and when it was given up on
/// it takes its place in the count back.
struct Ending {
    state: Arc<AtomicU8>,
    count: Arc<AtomicUsize>,
}

impl Drop for Ending {
    fn drop(&mut self) {
        if self.state.swap(FINISHED, Ordering::AcqRel) == ABANDONED {
            self.count.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// The threads of statuses that did not answer at their deadline and are still running; clones share the count.
#[derive(Clone, Default)]
pub(super) struct Stuck(Arc<AtomicUsize>);

impl Stuck {
    /// Whether [`MAX_STUCK`] statuses run or are stuck, which is a quick answer of "busy" without a place of the threads taken.
    pub(super) fn is_full(&self) -> bool {
        self.count() >= MAX_STUCK
    }

    /// How many statuses run or are stuck now.
    pub(super) fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    /// Runs `work` on a thread of its own and waits at most `wait` for what it returns. At the end of `wait` the thread is
    /// left running and counted as stuck until it ends. [`Unanswered::Busy`] when [`MAX_STUCK`] others run or are stuck.
    pub(super) fn run<T: Send + 'static>(
        &self,
        wait: Duration,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Unanswered> {
        let reservation = Reservation::take(&self.0).ok_or(Unanswered::Busy)?;
        let state = Arc::new(AtomicU8::new(RUNNING));
        let (sender, receiver) = mpsc::channel();
        let ending = Ending {
            state: Arc::clone(&state),
            count: Arc::clone(&self.0),
        };

        std::thread::Builder::new()
            .name("git-status".to_owned())
            .spawn(move || {
                // The answer is sent before the thread says it is done (as `ending` is dropped), so a request that finds the
                // thread done finds its answer.
                let _ = sender.send(work());
                drop(ending);
            })
            .map_err(|_| Unanswered::Failed)?;

        match receiver.recv_timeout(wait) {
            Ok(answer) => Ok(answer),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Unanswered::Failed),
            Err(mpsc::RecvTimeoutError::Timeout) => Self::abandon(&state, &receiver, reservation),
        }
    }

    /// The thread did not answer in time: it is counted as stuck (its place is kept, until the thread gives it back as it
    /// ends), unless it has finished in the meantime.
    fn abandon<T>(state: &AtomicU8, receiver: &mpsc::Receiver<T>, reservation: Reservation) -> Result<T, Unanswered> {
        // The place was taken before the thread started, so the thread, which takes it back only when it finds itself
        // abandoned, never takes one that was not counted.
        if state
            .compare_exchange(RUNNING, ABANDONED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            reservation.keep();

            return Err(Unanswered::TimedOut);
        }

        // It finished between the end of the wait and now, and its answer is there; the place is given back as it is dropped.
        receiver.try_recv().map_err(|_| Unanswered::Failed)
    }
}

/// Waits until `condition` holds, for at most ten seconds.
#[cfg(test)]
pub(super) fn until(condition: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    while !condition() {
        assert!(started.elapsed() < Duration::from_secs(10), "it never held");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn work_that_answers_in_time_is_answered_and_leaves_nothing_stuck() {
        let stuck = Stuck::default();

        assert_eq!(stuck.run(Duration::from_secs(10), || 7), Ok(7));
        assert_eq!(stuck.count(), 0);
    }

    #[test]
    fn work_that_does_not_answer_is_given_up_at_the_deadline_and_counts_as_stuck_until_it_ends() {
        let stuck = Stuck::default();
        let (release, hold) = mpsc::channel::<()>();
        let started = Instant::now();

        let answer = stuck.run(Duration::from_millis(50), move || {
            let _ = hold.recv();
        });

        assert_eq!(answer, Err(Unanswered::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert_eq!(stuck.count(), 1);

        drop(release);
        until(|| stuck.count() == 0);
    }

    /// Runs work that does not return, given up on at its deadline: one more stuck thread, which ends when the sender it
    /// leaves in `held` is dropped.
    fn stick(stuck: &Stuck, held: &mut Vec<mpsc::Sender<()>>) {
        let (release, hold) = mpsc::channel::<()>();
        held.push(release);

        assert_eq!(
            stuck.run(Duration::from_millis(10), move || {
                let _ = hold.recv();
            }),
            Err(Unanswered::TimedOut)
        );
    }

    #[test]
    fn the_count_is_full_at_the_most_and_is_freed_by_the_threads_ending() {
        let stuck = Stuck::default();
        let mut held = Vec::new();

        for _ in 0..MAX_STUCK {
            assert!(!stuck.is_full());
            stick(&stuck, &mut held);
        }
        assert!(stuck.is_full());

        held.pop();
        until(|| !stuck.is_full());
    }

    #[test]
    fn statuses_started_in_the_same_moment_never_make_more_stuck_threads_than_the_most() {
        let stuck = Stuck::default();
        let mut held = Vec::new();

        // All but one of the places are taken by threads that do not come back.
        for _ in 0..MAX_STUCK - 1 {
            stick(&stuck, &mut held);
        }

        // Several are let in at once; each would be stuck at its deadline.
        let start = std::sync::Barrier::new(8);
        let answers: Vec<_> = std::thread::scope(|scope| {
            let attempts: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        let (release, hold) = mpsc::channel::<()>();
                        let answer = stuck.run(Duration::from_millis(30), move || {
                            let _ = hold.recv();
                        });

                        (answer, release)
                    })
                })
                .collect();

            attempts
                .into_iter()
                .map(|attempt| attempt.join().expect("an attempt"))
                .collect()
        });

        let timed_out = answers
            .iter()
            .filter(|(answer, _)| *answer == Err(Unanswered::TimedOut))
            .count();
        let busy = answers
            .iter()
            .filter(|(answer, _)| *answer == Err(Unanswered::Busy))
            .count();
        assert_eq!((timed_out, busy), (1, 7), "one place was left");
        assert_eq!(stuck.count(), MAX_STUCK);

        drop(held);
        drop(answers);
        until(|| stuck.count() == 0);
    }

    #[test]
    fn work_that_panics_is_failed_and_is_not_counted_as_stuck_even_when_it_panics_late() {
        let stuck = Stuck::default();

        let answer: Result<(), _> = stuck.run(Duration::from_secs(10), || panic!("a read failed"));
        assert_eq!(answer, Err(Unanswered::Failed));
        assert_eq!(stuck.count(), 0);

        // It panics after it was given up on: the place it held is given back too.
        let late: Result<(), _> = stuck.run(Duration::from_millis(20), || {
            std::thread::sleep(Duration::from_millis(150));
            panic!("a read failed, late");
        });
        assert_eq!(late, Err(Unanswered::TimedOut));
        until(|| stuck.count() == 0);
    }

    #[test]
    fn work_that_ends_while_the_deadline_passes_never_leaves_the_count_wrong() {
        let stuck = Stuck::default();

        for _ in 0..200 {
            // Answers about when the wait ends.
            let _ = stuck.run(Duration::from_micros(200), || {
                std::thread::sleep(Duration::from_micros(200))
            });
        }

        until(|| stuck.count() == 0);
    }
}

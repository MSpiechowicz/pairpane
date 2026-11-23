//! What stops a status early: the time limit, the app's exit and the cap on entries, all as one flag that gix polls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::GitError;

/// How often the watchdog looks at the app's exit flag.
const POLL: Duration = Duration::from_millis(50);

/// Why the status was told to stop. `interrupt` is the flag gix polls (`should_interrupt`); the others say why it was set.
#[derive(Default)]
pub(super) struct Stop {
    pub(super) interrupt: AtomicBool,
    timed_out: AtomicBool,
    cancelled: AtomicBool,
    over_cap: AtomicBool,
}

impl Stop {
    /// The reason the status was stopped for, if it was.
    pub(super) fn reason(&self) -> Option<GitError> {
        if self.cancelled.load(Ordering::Acquire) {
            Some(GitError::Cancelled)
        } else if self.over_cap.load(Ordering::Acquire) {
            Some(GitError::TooBig)
        } else if self.timed_out.load(Ordering::Acquire) {
            Some(GitError::TimedOut)
        } else {
            None
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.interrupt.load(Ordering::Acquire)
    }

    /// What an interrupted or failed read is told as: a raised flag is its own answer, an object over gix's allocation limit
    /// is a repository too big to read here, and anything else is an unreadable repository (the details are only logged).
    pub(super) fn explain(&self, error: &gix::Error) -> GitError {
        if let Some(reason) = self.reason() {
            return reason;
        }
        if error.is_resource_exhausted() {
            tracing::debug!(%error, "git status: an object is too big to read");

            return GitError::TooBig;
        }
        tracing::debug!(%error, "git status: the repository cannot be read");

        GitError::Repository
    }
}

/// The entries one status may still walk.
pub(super) struct Meter<'a> {
    stop: &'a Stop,
    left: usize,
}

impl<'a> Meter<'a> {
    pub(super) fn new(stop: &'a Stop, cap: usize) -> Self {
        Self { stop, left: cap }
    }

    /// Counts `count` entries. When there are no more to give, the status is told to stop and this answers `false`.
    pub(super) fn spend(&mut self, count: usize) -> bool {
        match self.left.checked_sub(count) {
            Some(left) => {
                self.left = left;

                !self.stop.is_stopped()
            }
            None => {
                self.exhaust();

                false
            }
        }
    }

    /// Something other than a count is over its bound (a path too long, a tree nested too deep): the same as running out
    /// of entries.
    pub(super) fn exhaust(&mut self) {
        self.left = 0;
        self.stop.over_cap.store(true, Ordering::Release);
        self.stop.interrupt.store(true, Ordering::Release);
    }
}

/// Runs `work` while a watchdog thread sets `stop` when `time` is up or `cancel` is raised. The watchdog ends with `work`.
pub(super) fn guarded<T>(
    stop: &Stop,
    time: Duration,
    cancel: &AtomicBool,
    work: impl FnOnce() -> Result<T, GitError>,
) -> Result<T, GitError> {
    let deadline = Instant::now() + time;

    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("git-status-watchdog".to_owned())
            .spawn_scoped(scope, move || watch(stop, deadline, cancel, &finished))
            .map_err(|_| GitError::Repository)?;

        let result = work();
        // Dropping the sender wakes the watchdog, which then ends and lets the scope close.
        drop(done);

        result
    })
}

fn watch(stop: &Stop, deadline: Instant, cancel: &AtomicBool, finished: &mpsc::Receiver<()>) {
    loop {
        let wait = deadline.saturating_duration_since(Instant::now()).min(POLL);

        match finished.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => {}
            // The work is over (the sender is dropped); nothing is ever sent.
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }

        if cancel.load(Ordering::Relaxed) {
            stop.cancelled.store(true, Ordering::Release);
            stop.interrupt.store(true, Ordering::Release);

            return;
        }

        if Instant::now() >= deadline {
            stop.timed_out.store(true, Ordering::Release);
            stop.interrupt.store(true, Ordering::Release);

            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// Work that does nothing but wait for the flag, as gix does between entries.
    fn until_stopped(stop: &Stop) -> Result<(), GitError> {
        let started = Instant::now();
        while !stop.is_stopped() {
            assert!(started.elapsed() < Duration::from_secs(20), "never stopped");
            thread::sleep(Duration::from_millis(1));
        }

        Ok(())
    }

    #[test]
    fn the_time_limit_raises_the_flag_and_the_reason_is_timed_out() {
        let stop = Stop::default();

        guarded(&stop, Duration::from_millis(20), &AtomicBool::new(false), || {
            until_stopped(&stop)
        })
        .expect("ended");

        assert!(matches!(stop.reason(), Some(GitError::TimedOut)));
    }

    #[test]
    fn the_exit_flag_raises_the_flag_and_the_reason_is_cancelled() {
        let stop = Stop::default();
        let cancel = AtomicBool::new(false);

        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(10));
                cancel.store(true, Ordering::Relaxed);
            });
            guarded(&stop, Duration::from_secs(30), &cancel, || until_stopped(&stop)).expect("ended");
        });

        assert!(matches!(stop.reason(), Some(GitError::Cancelled)));
    }

    #[test]
    fn work_that_ends_first_leaves_no_flag_and_does_not_wait_for_the_time_limit() {
        let stop = Stop::default();
        let started = Instant::now();

        let answer = guarded(&stop, Duration::from_secs(30), &AtomicBool::new(false), || Ok(7)).expect("ended");

        assert_eq!(answer, 7);
        assert!(stop.reason().is_none() && !stop.is_stopped());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the watchdog was waited for"
        );
    }

    #[test]
    fn the_cap_on_entries_raises_the_flag_when_there_is_no_room_left() {
        let stop = Stop::default();
        let mut meter = Meter::new(&stop, 5);

        assert!(meter.spend(3));
        assert!(meter.spend(2));
        assert!(!stop.is_stopped());
        assert!(!meter.spend(1));

        assert!(stop.is_stopped());
        assert!(matches!(stop.reason(), Some(GitError::TooBig)));
    }
}

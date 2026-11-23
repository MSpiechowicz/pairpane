//! What stops the parent's own look at a repository early (`locate`, `inspect`, `objects`; the child has the watchdog of
//! `watch`): the app's exit and the time limit of the status. The look is a number of `lstat`, `open` and `fstat` calls over
//! folders that a repository names, which can be many and can be slow (a network or a slow disk): it is held to the same time as
//! the status, and ends at once when the app exits, whatever the repository holds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::GitError;

/// Why a look stopped before its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Halt {
    /// The app is exiting.
    Cancelled,
    /// The time limit is up.
    TimedOut,
}

impl From<Halt> for GitError {
    fn from(halt: Halt) -> Self {
        match halt {
            Halt::Cancelled => Self::Cancelled,
            Halt::TimedOut => Self::TimedOut,
        }
    }
}

/// The exit flag of the app and the moment the look must be over, asked with [`Gate::check`] before each step that costs a call to
/// the system.
#[derive(Clone, Copy)]
pub(super) struct Gate<'a> {
    cancel: &'a AtomicBool,
    /// `None`: no limit.
    until: Option<Instant>,
}

impl<'a> Gate<'a> {
    /// A gate that stops when `cancel` is raised, or `time` after now.
    pub(super) fn new(cancel: &'a AtomicBool, time: Duration) -> Self {
        Self {
            cancel,
            until: Instant::now().checked_add(time),
        }
    }

    /// A gate that never stops anything.
    #[cfg(test)]
    pub(super) fn never() -> Gate<'static> {
        /// The flag of a gate that is never raised.
        static NEVER: AtomicBool = AtomicBool::new(false);

        Gate {
            cancel: &NEVER,
            until: None,
        }
    }

    /// Whether the look may go on.
    pub(super) fn check(&self) -> Result<(), Halt> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Halt::Cancelled)
        } else if self.until.is_some_and(|until| Instant::now() >= until) {
            Err(Halt::TimedOut)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gate_stops_at_the_exit_flag_and_at_the_time_and_not_before() {
        let cancel = AtomicBool::new(false);

        assert_eq!(Gate::new(&cancel, Duration::from_secs(3600)).check(), Ok(()));
        assert_eq!(Gate::never().check(), Ok(()));
        assert_eq!(Gate::new(&cancel, Duration::ZERO).check(), Err(Halt::TimedOut));

        cancel.store(true, Ordering::Relaxed);
        // The exit is the reason when both are.
        assert_eq!(Gate::new(&cancel, Duration::ZERO).check(), Err(Halt::Cancelled));
        assert_eq!(
            Gate::new(&cancel, Duration::from_secs(3600)).check(),
            Err(Halt::Cancelled)
        );
        assert!(matches!(GitError::from(Halt::TimedOut), GitError::TimedOut));
        assert!(matches!(GitError::from(Halt::Cancelled), GitError::Cancelled));
    }

    #[test]
    fn a_time_that_cannot_be_added_is_no_limit() {
        let cancel = AtomicBool::new(false);

        assert_eq!(Gate::new(&cancel, Duration::MAX).check(), Ok(()));
    }
}

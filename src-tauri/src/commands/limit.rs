//! A cap on the blocking threads one pane can have outstanding.
//!
//! Listing and size jobs run on Tokio's blocking pool, and cancelling one only sets a flag: a job stuck in a system
//! call on a hung mount (a stale NFS or FUSE share) never sees it and keeps its thread for good. Without a cap a page
//! could start jobs until the pool (512 threads) is full and the whole app hangs, so a request is refused while the
//! pane already has the most such threads it is allowed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many threads of one kind a pane has right now; clones share the count.
#[derive(Clone, Default)]
pub(super) struct ThreadCap(Arc<AtomicUsize>);

impl ThreadCap {
    /// Takes one of the `max` places, or `None` when all are taken. The place is given back when the permit is dropped,
    /// which must happen when the thread ends (move the permit into its closure).
    pub(super) fn acquire(&self, max: usize) -> Option<Permit> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |taken| {
                (taken < max).then_some(taken + 1)
            })
            .ok()?;

        Some(Permit(Arc::clone(&self.0)))
    }
}

/// One place of a [`ThreadCap`], held by a thread for as long as it runs.
#[derive(Debug)]
pub(super) struct Permit(Arc<AtomicUsize>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_more_places_than_the_cap_are_given_until_one_is_dropped() {
        let cap = ThreadCap::default();

        let first = cap.acquire(2).expect("first");
        let _second = cap.acquire(2).expect("second");
        assert!(cap.acquire(2).is_none(), "full");

        drop(first);
        let _third = cap.acquire(2).expect("a freed place");
        assert!(cap.acquire(2).is_none(), "full again");
    }

    #[test]
    fn clones_share_the_places_and_a_dropped_permit_on_another_thread_frees_one() {
        let cap = ThreadCap::default();
        let permit = cap.acquire(1).expect("place");
        assert!(cap.clone().acquire(1).is_none());

        std::thread::spawn(move || drop(permit)).join().expect("thread");

        assert!(cap.acquire(1).is_some());
    }

    #[test]
    fn a_permit_is_given_back_when_its_thread_panics() {
        let cap = ThreadCap::default();
        let permit = cap.acquire(1).expect("place");

        let result = std::thread::spawn(move || {
            let _held = permit;
            panic!("job failed");
        })
        .join();

        assert!(result.is_err());
        assert!(cap.acquire(1).is_some());
    }
}

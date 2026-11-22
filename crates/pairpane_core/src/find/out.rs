//! What a search shares between its threads and what it tells its caller: the stop flags and counters of a [`Run`], and
//! the [`Out`] that batches the results into events.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::model::{FindEvent, FindHit};

/// Results are sent when this many are waiting.
const BATCH_HITS: usize = 256;

/// Results and progress are sent at least this often.
const BATCH_TIME: Duration = Duration::from_millis(100);

/// How long the thread that tells the caller waits for the workers before it looks at the clock again.
const TICK: Duration = Duration::from_millis(20);

/// The state of one search that its threads share: the caller's cancel flag, a flag of the search's own that stops it at
/// a limit, and the counters the progress is made of.
pub(crate) struct Run<'a> {
    cancel: &'a AtomicBool,
    capped: AtomicBool,
    walk_over: AtomicBool,
    pub(crate) folders: AtomicU64,
    pub(crate) files: AtomicU64,
    /// Bytes of the files looked at; while duplicates are hashed, the bytes hashed.
    pub(crate) bytes: AtomicU64,
    pub(crate) skipped: AtomicU64,
}

impl<'a> Run<'a> {
    pub(crate) fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            cancel,
            capped: AtomicBool::new(false),
            walk_over: AtomicBool::new(false),
            folders: AtomicU64::new(0),
            files: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }

    /// Whether the caller stopped the search.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The caller's flag, for the readers of archives.
    pub(crate) fn flag(&self) -> &AtomicBool {
        self.cancel
    }

    /// Whether the work must stop: the caller stopped it or a limit was reached.
    pub(crate) fn raised(&self) -> bool {
        self.cancelled() || self.capped.load(Ordering::Relaxed)
    }

    /// Stops the work at a limit.
    pub(crate) fn cap(&self) {
        self.capped.store(true, Ordering::Relaxed);
    }

    /// Stops the walk alone, at a limit on what it collects: what was collected is still worked on.
    pub(crate) fn end_walk(&self) {
        self.walk_over.store(true, Ordering::Relaxed);
    }

    /// Whether the walk was stopped at a limit (see [`Run::end_walk`]).
    pub(crate) fn walk_over(&self) -> bool {
        self.walk_over.load(Ordering::Relaxed)
    }

    /// Counts one thing that could not be looked at.
    pub(crate) fn skip(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }
}

/// Collects results and hands them to the caller's `emit` as `found`, `progress` and `group` events, and ends the
/// search with `done`.
pub(crate) struct Out<'e> {
    emit: &'e mut dyn FnMut(FindEvent),
    pending: Vec<FindHit>,
    sent_at: Instant,
    last: (u64, u64, u64),
    found: u64,
    cap: u64,
    truncated: bool,
}

impl<'e> Out<'e> {
    pub(crate) fn new(emit: &'e mut dyn FnMut(FindEvent), cap: u64) -> Self {
        Self {
            emit,
            pending: Vec::new(),
            sent_at: Instant::now(),
            last: (0, 0, 0),
            found: 0,
            cap,
            truncated: false,
        }
    }

    /// Marks the search as stopped at a limit.
    pub(crate) fn truncate(&mut self, run: &Run<'_>) {
        self.truncated = true;
        run.cap();
    }

    /// Marks the search as stopped at a limit on what the walk collects; the walk ends, the rest of the work goes on.
    pub(crate) fn truncate_walk(&mut self, run: &Run<'_>) {
        self.truncated = true;
        run.end_walk();
    }

    /// Adds one result; past the cap on results ([`super::MAX_HITS`]) the search stops, and the result is dropped.
    pub(crate) fn push(&mut self, hit: FindHit, run: &Run<'_>) {
        if self.found >= self.cap {
            self.truncate(run);
            return;
        }

        self.found += 1;
        self.pending.push(hit);
    }

    /// Sends a group of duplicates, as long as the results stay within the cap on results.
    pub(crate) fn group(&mut self, size: u64, hits: Vec<FindHit>, run: &Run<'_>) {
        if self.found + hits.len() as u64 > self.cap {
            self.truncate(run);
            return;
        }

        self.found += hits.len() as u64;
        (self.emit)(FindEvent::Group { size, hits });
    }

    /// Sends what is waiting once there are enough results or the time is up, and the progress when it changed.
    pub(crate) fn tick(&mut self, run: &Run<'_>) {
        if self.pending.len() >= BATCH_HITS || self.sent_at.elapsed() >= BATCH_TIME {
            self.flush();
            self.progress(run);
            self.sent_at = Instant::now();
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            (self.emit)(FindEvent::Found {
                hits: std::mem::take(&mut self.pending),
            });
        }
    }

    fn progress(&mut self, run: &Run<'_>) {
        let now = (
            run.folders.load(Ordering::Relaxed),
            run.files.load(Ordering::Relaxed),
            run.bytes.load(Ordering::Relaxed),
        );

        if now != self.last {
            self.last = now;
            (self.emit)(FindEvent::Progress {
                folders: now.0,
                files: now.1,
                bytes: now.2,
            });
        }
    }

    /// Sends the last results and progress, then `done`.
    pub(crate) fn finish(mut self, run: &Run<'_>) {
        self.flush();
        self.progress(run);
        (self.emit)(FindEvent::Done {
            found: self.found,
            skipped: run.skipped.load(Ordering::Relaxed),
            truncated: self.truncated,
            cancelled: run.cancelled(),
        });
    }
}

/// Takes what the workers send until all of them are done, telling the caller as it goes. `on` gets each message.
pub(crate) fn pump<M>(messages: &Receiver<M>, run: &Run<'_>, out: &mut Out<'_>, mut on: impl FnMut(M, &mut Out<'_>)) {
    loop {
        match messages.recv_timeout(TICK) {
            Ok(message) => on(message, out),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        out.tick(run);
    }
}

//! What a comparison keeps while it runs, and what it tells its caller: the stop flag, the counters the progress is made of,
//! the cap on rows, and the batching of rows into events.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::model::{CompareEvent, CompareOptions, CompareRow};

use super::Limits;

/// Rows are sent when this many are waiting.
const BATCH_ROWS: usize = 256;

/// Rows and progress are sent at least this often.
pub(super) const BATCH_TIME: Duration = Duration::from_millis(100);

/// The state of one comparison. Single threaded: the walk, the content checks and the events all run on its thread.
pub(super) struct Scan<'a> {
    cancel: &'a AtomicBool,
    emit: &'a mut dyn FnMut(CompareEvent),
    pub(super) options: CompareOptions,
    limits: Limits,
    pub(super) fold: bool,
    pending: Vec<CompareRow>,
    sent_at: Instant,
    last: (u64, u64, u64),
    rows: u64,
    pub(super) folders: u64,
    pub(super) files: u64,
    /// The bytes of the files looked at; with content compared, the bytes read.
    pub(super) bytes: u64,
    pub(super) skipped: u64,
    truncated: bool,
}

impl<'a> Scan<'a> {
    pub(super) fn new(
        cancel: &'a AtomicBool,
        emit: &'a mut dyn FnMut(CompareEvent),
        options: CompareOptions,
        limits: Limits,
    ) -> Self {
        Self {
            cancel,
            emit,
            options,
            limits,
            fold: limits.fold,
            pending: Vec::new(),
            sent_at: Instant::now(),
            last: (0, 0, 0),
            rows: 0,
            folders: 0,
            files: 0,
            bytes: 0,
            skipped: 0,
            truncated: false,
        }
    }

    /// Whether the work must stop: the caller stopped it or a limit was reached.
    pub(super) fn halted(&self) -> bool {
        self.truncated || self.cancelled()
    }

    pub(super) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Adds one row; past the cap the row is dropped and the comparison stops.
    pub(super) fn push(&mut self, row: CompareRow) {
        if self.rows >= self.limits.rows {
            self.truncated = true;

            return;
        }

        self.rows += 1;
        self.pending.push(row);
    }

    /// Sends what is waiting once there are enough rows or the time is up, and the progress when it changed.
    pub(super) fn tick(&mut self) {
        if self.pending.len() >= BATCH_ROWS || self.sent_at.elapsed() >= self.limits.tick {
            self.flush();
            self.progress();
            self.sent_at = Instant::now();
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            (self.emit)(CompareEvent::Rows {
                rows: std::mem::take(&mut self.pending),
            });
        }
    }

    fn progress(&mut self) {
        let now = (self.folders, self.files, self.bytes);

        if now != self.last {
            self.last = now;
            (self.emit)(CompareEvent::Progress {
                folders: now.0,
                files: now.1,
                bytes: now.2,
            });
        }
    }

    /// Sends the last rows and progress, then `done`; `more` says the archive's index was cut short.
    pub(super) fn finish(mut self, more: bool) {
        self.flush();
        self.progress();
        let done = CompareEvent::Done {
            rows: self.rows,
            skipped: self.skipped,
            truncated: self.truncated || more,
            cancelled: self.cancelled(),
        };

        (self.emit)(done);
    }
}

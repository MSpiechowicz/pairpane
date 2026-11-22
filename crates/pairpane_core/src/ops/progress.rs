//! What a running job reports: its phase, its counters, and the speed. The counters live in the job's view (for
//! snapshots) and go out as events at most every 100 ms.

use std::time::{Duration, Instant};

use crate::model::{JobPhase, OpsEvent};

use super::control::Cancelled;
use super::rate::Rate;
use super::transfer::Run;

/// The counters of one job, and when it last reported.
pub(super) struct Progress {
    total_bytes: u64,
    total_files: u32,
    done_bytes: u64,
    done_files: u32,
    current: Option<String>,
    rate: Rate,
    reported_at: Instant,
    /// A phase that changed since the last event. The two that alternate for every file (copying and verifying)
    /// are sent with the next progress event instead of one event each.
    pending_phase: Option<JobPhase>,
    /// The file being copied: its size, and how many of its bytes were counted so far.
    file_size: u64,
    file_counted: u64,
}

impl Progress {
    pub(super) fn new() -> Self {
        let now = Instant::now();

        Self {
            total_bytes: 0,
            total_files: 0,
            done_bytes: 0,
            done_files: 0,
            current: None,
            rate: Rate::new(now),
            reported_at: now,
            pending_phase: None,
            file_size: 0,
            file_counted: 0,
        }
    }
}

impl Run<'_> {
    /// Moves the job to `phase`. Once the job is cancelled the phase stays `cancelling`.
    pub(super) fn phase(&mut self, phase: JobPhase) {
        if self.job.control.is_cancelled() {
            return;
        }
        self.job.view().phase = phase;

        if matches!(phase, JobPhase::Copying | JobPhase::Verifying) {
            self.progress.pending_phase = Some(phase);
            return;
        }

        self.progress.pending_phase = None;
        self.inner.emit(OpsEvent::Phase { id: self.job.id, phase });
    }

    /// Stops here when the job is cancelled and waits here while it is paused, without counting that time in
    /// the speed.
    pub(super) fn checkpoint(&mut self) -> Result<(), Cancelled> {
        let held = self.job.control.checkpoint()?;
        self.exclude(held);

        Ok(())
    }

    /// Leaves `time` out of the speed: the job was waiting for the user, not working.
    pub(super) fn exclude(&mut self, time: Duration) {
        self.progress.rate.exclude(time);
    }

    /// The scan found a file (or a link) of `bytes`.
    pub(super) fn found(&mut self, bytes: u64) {
        self.progress.total_files = self.progress.total_files.saturating_add(1);
        self.progress.total_bytes = self.progress.total_bytes.saturating_add(bytes);
        self.tick(false);
    }

    /// A job that renames `count` items knows its total from the start.
    pub(super) fn expect_items(&mut self, count: usize) {
        self.progress.total_files = u32::try_from(count).unwrap_or(u32::MAX);
        self.tick(false);
    }

    /// A whole item was moved by a rename: it counts as one finished file.
    pub(super) fn renamed(&mut self) {
        self.progress.total_files = self.progress.total_files.saturating_add(1);
        self.progress.done_files = self.progress.done_files.saturating_add(1);
        self.tick(false);
    }

    /// The copy of the file `rel` of `size` bytes starts.
    pub(super) fn begin_file(&mut self, rel: &str, size: u64) {
        self.progress.current = Some(rel.to_owned());
        self.progress.file_size = size;
        self.progress.file_counted = 0;
        self.tick(false);
    }

    /// `bytes` more of the current file are written.
    pub(super) fn add_bytes(&mut self, bytes: u64) {
        self.progress.done_bytes = self.progress.done_bytes.saturating_add(bytes);
        self.progress.file_counted = self.progress.file_counted.saturating_add(bytes);
        self.tick(false);
    }

    /// The current file is over, however it ended: what was not written (a skipped or failed file) counts as
    /// done, so the bar reaches its end.
    pub(super) fn end_file(&mut self) {
        let progress = &mut self.progress;
        let rest = progress.file_size.saturating_sub(progress.file_counted);

        progress.done_bytes = progress.done_bytes.saturating_add(rest);
        progress.done_files = progress.done_files.saturating_add(1);
        progress.file_counted = progress.file_size;
        self.tick(false);
    }

    /// Updates the job's view and, once the interval has passed (or `force`), sends the progress event.
    pub(super) fn tick(&mut self, force: bool) {
        let now = Instant::now();
        let due = now.saturating_duration_since(self.progress.reported_at) >= self.inner.hooks.progress_interval();

        if !force && !due {
            return;
        }

        let progress = &mut self.progress;
        progress.reported_at = now;
        progress.rate.sample(now, progress.done_bytes);

        let bytes_per_sec = progress.rate.bytes_per_sec();
        let eta_secs = progress
            .rate
            .eta_secs(progress.total_bytes.saturating_sub(progress.done_bytes));

        {
            let mut view = self.job.view();
            view.done_bytes = progress.done_bytes;
            view.total_bytes = progress.total_bytes;
            view.done_files = progress.done_files;
            view.total_files = progress.total_files;
            view.current.clone_from(&progress.current);
            view.bytes_per_sec = bytes_per_sec;
            view.eta_secs = eta_secs;
        }

        if let Some(phase) = progress.pending_phase.take() {
            self.inner.emit(OpsEvent::Phase { id: self.job.id, phase });
        }
        self.inner.emit(OpsEvent::Progress {
            id: self.job.id,
            done_bytes: progress.done_bytes,
            total_bytes: progress.total_bytes,
            done_files: progress.done_files,
            total_files: progress.total_files,
            current: progress.current.clone(),
            bytes_per_sec,
            eta_secs,
        });
    }
}

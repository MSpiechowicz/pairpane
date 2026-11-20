//! Speed and time left of one job: an exponentially weighted moving average over samples, with the time the job
//! spent paused or waiting for an answer left out.

use std::time::{Duration, Instant};

/// A sample is taken only after this long, so a burst of tiny chunks does not make the speed jump around.
const MIN_SAMPLE: Duration = Duration::from_millis(100);

/// How much of the newest sample counts (the rest is the history).
const WEIGHT: f64 = 0.3;

/// Active time needed before a time left is offered: the first second is too noisy to promise anything.
const WARM_UP: Duration = Duration::from_secs(1);

/// Smoothed bytes per second of one job.
#[derive(Debug)]
pub(super) struct Rate {
    speed: Option<f64>,
    sampled_at: Instant,
    sampled_bytes: u64,
    /// Time not spent working since the last sample, taken out of the next interval.
    idle: Duration,
    /// Time spent working since the job started.
    active: Duration,
}

impl Rate {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            speed: None,
            sampled_at: now,
            sampled_bytes: 0,
            idle: Duration::ZERO,
            active: Duration::ZERO,
        }
    }

    /// Leaves `time` out of the speed: the job was paused or waiting for an answer.
    pub(super) fn exclude(&mut self, time: Duration) {
        self.idle += time;
    }

    /// Takes a sample once enough time has passed since the last one. `done_bytes` only grows.
    pub(super) fn sample(&mut self, now: Instant, done_bytes: u64) {
        let elapsed = now.saturating_duration_since(self.sampled_at);
        let working = elapsed.saturating_sub(self.idle);

        if elapsed < MIN_SAMPLE.saturating_add(self.idle) {
            return;
        }

        let instant = done_bytes.saturating_sub(self.sampled_bytes) as f64 / working.as_secs_f64();
        self.speed = Some(self.speed.map_or(instant, |old| old + WEIGHT * (instant - old)));
        self.active += working;
        self.sampled_at = now;
        self.sampled_bytes = done_bytes;
        self.idle = Duration::ZERO;
    }

    /// Whole bytes per second; 0 before the first sample.
    pub(super) fn bytes_per_sec(&self) -> u64 {
        self.speed.map_or(0, |speed| speed as u64)
    }

    /// Seconds left for `remaining` bytes; `None` while it cannot be told (warming up, or no speed).
    pub(super) fn eta_secs(&self, remaining: u64) -> Option<u64> {
        let speed = self.speed.filter(|speed| *speed >= 1.0)?;

        if self.active < WARM_UP {
            return None;
        }

        Some((remaining as f64 / speed).ceil() as u64)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn secs(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    #[test]
    fn the_first_sample_is_the_plain_speed() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        rate.sample(start + secs(0.5), 5_000);

        assert_eq!(rate.bytes_per_sec(), 10_000);
    }

    #[test]
    fn a_sample_before_the_minimum_interval_is_ignored() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        rate.sample(start + secs(0.05), 5_000);

        assert_eq!(rate.bytes_per_sec(), 0);
    }

    #[test]
    fn later_samples_move_the_speed_part_of_the_way() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        rate.sample(start + secs(1.0), 1_000);
        rate.sample(start + secs(2.0), 3_000);

        // 1000 + 0.3 * (2000 - 1000)
        assert_eq!(rate.bytes_per_sec(), 1_300);
    }

    #[test]
    fn paused_time_does_not_slow_the_speed_down() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        // One second of work and nine seconds paused: still 1000 bytes per second.
        rate.exclude(secs(9.0));
        rate.sample(start + secs(10.0), 1_000);

        assert_eq!(rate.bytes_per_sec(), 1_000);
    }

    #[test]
    fn a_short_stretch_after_a_long_pause_waits_for_more_work() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        rate.exclude(secs(9.0));
        rate.sample(start + secs(9.05), 100);

        assert_eq!(rate.bytes_per_sec(), 0);
    }

    #[test]
    fn the_time_left_waits_for_a_second_of_work() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        rate.sample(start + secs(0.5), 500);
        assert_eq!(rate.eta_secs(10_000), None);

        rate.sample(start + secs(1.5), 1_500);
        assert_eq!(rate.eta_secs(10_000), Some(10));
    }

    #[test]
    fn no_speed_means_no_time_left() {
        let start = Instant::now();
        let mut rate = Rate::new(start);

        rate.sample(start + secs(2.0), 0);

        assert_eq!(rate.eta_secs(10_000), None);
    }
}

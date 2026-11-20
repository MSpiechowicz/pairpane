//! Switches that only tests flip, to reach what a test machine cannot produce by itself: a second volume, a disk
//! that fails, a copy that differs from its source. The engine reads them through the methods below, which
//! answer for the normal case unless a test set the field (nothing outside `#[cfg(test)]` code does).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::listing::size::PROGRESS_INTERVAL;

use super::volume::{self, VolumeKey};

/// Decides the volume of a target folder.
pub(super) type Keyer = Arc<dyn Fn(&Path) -> VolumeKey + Send + Sync>;

#[derive(Clone, Default)]
pub(super) struct Hooks {
    /// Decides the volume of a target folder.
    pub(super) keyer: Option<Keyer>,
    /// Every rename fails as if the two paths were on different disks.
    pub(super) cross_device: bool,
    /// Every copy hashes differently from its source.
    pub(super) corrupt_verify: bool,
    /// Every copy of a file fails with a disk error once its data is written.
    pub(super) fail_after_data: bool,
    /// Every copy of a file fails with "already exists" once its data is written, as an error after the temporary
    /// file was made, which must not look like a taken name.
    pub(super) taken_after_data: bool,
    /// No clone and no kernel copy: the buffered loop moves every byte.
    pub(super) buffered_only: bool,
    /// Progress goes out after every chunk, not every 100 ms.
    pub(super) every_chunk: bool,
    /// The worker panics when the job starts.
    pub(super) panic_on_start: bool,
    /// No two paths compare equal as one file: the case of a Windows alias that the check of a conflict does not see
    /// through (the identities are still read, by the checks that come after it).
    pub(super) blind_identity: bool,
    /// No identity can be read, as when a Windows ACL refuses the open while the names are unchanged.
    pub(super) unreadable_identity: bool,
}

impl Hooks {
    pub(super) fn key_of(&self, target: &Path) -> VolumeKey {
        self.keyer
            .as_ref()
            .map_or_else(|| volume::key_of(target), |keyer| keyer(target))
    }

    pub(super) fn progress_interval(&self) -> Duration {
        if self.every_chunk {
            Duration::ZERO
        } else {
            PROGRESS_INTERVAL
        }
    }
}

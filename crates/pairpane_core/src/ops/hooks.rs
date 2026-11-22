//! Switches that only tests flip, to reach what a test machine cannot produce by itself: a second volume, a disk
//! that fails, a copy that differs from its source. The engine reads them through the methods below, which
//! answer for the normal case unless a test set the field (nothing outside `#[cfg(test)]` code does).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::archive::{ArchiveIndex, EntrySource};
use crate::listing::size::PROGRESS_INTERVAL;

use super::volume::{self, VolumeKey};

/// Decides the volume of a target folder.
pub(super) type Keyer = Arc<dyn Fn(&Path) -> VolumeKey + Send + Sync>;

/// What a test runs before a folder is opened by a walk, given the folder's path relative to the job's folder.
pub(super) type OpenHook = Arc<dyn Fn(&str) + Send + Sync>;

/// What a test runs with the path of a temporary file just after it was made: it swaps the path for a link there.
pub(super) type PathHook = Arc<dyn Fn(&Path) + Send + Sync>;

/// Makes the source of the data of an extraction from its index, in place of the archive's own.
pub(super) type SourceHook = Arc<dyn Fn(&ArchiveIndex) -> Box<dyn EntrySource> + Send + Sync>;

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
    /// Called with the path (relative to the job's folder) of each folder a handle walk is about to open, after it was
    /// looked at: a test swaps the folder for a link there.
    pub(super) before_open: Option<OpenHook>,
    /// No identity can be read, as when a Windows ACL refuses the open while the names are unchanged.
    pub(super) unreadable_identity: bool,
    /// The data of an extraction comes from this source, whatever the archive holds.
    pub(super) entry_source: Option<SourceHook>,
    /// The free space an extraction leaves on the disk, in place of `MIN_FREE_RESERVE`.
    pub(super) free_reserve: Option<u64>,
    /// Called with the path of the temporary file of an add or a pack job right after it was made, before it is given
    /// its final permissions.
    pub(super) after_temp: Option<PathHook>,
    /// Called with the path of the temporary name an item of a multi-rename was just moved aside to, before the next
    /// one is: a test puts something at a new name, or cancels the job, there.
    pub(super) after_aside: Option<PathHook>,
    /// Called with the name of each item of a multi-rename when it is over, however it came out: a test cancels the job
    /// there, after a number of items.
    pub(super) after_item: Option<OpenHook>,
    /// The new archive of an add job cannot be given the old one's group, as when the user is not in it (Unix).
    #[cfg(unix)]
    pub(super) refuse_group: bool,
    /// The most entries and the most bytes of names a pack job takes, in place of the limits of the archive readers.
    pub(super) pack_limits: Option<(usize, usize)>,
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

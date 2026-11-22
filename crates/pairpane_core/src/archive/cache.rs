//! The indexes of the archives that were opened lately, so moving between the folders of one archive (or opening it
//! again) does not read it again.

use std::collections::VecDeque;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use crate::listing::platform;

use super::index::ArchiveIndex;

/// How many indexes are kept.
pub const CACHE_SIZE: usize = 4;

/// What tells one state of an archive file from another: where it is, how big it is, when it was changed and, where the
/// system says, which file it is (so a file replaced by another of the same size and time is a different one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    identity: Option<(u64, u64)>,
}

impl Fingerprint {
    /// The fingerprint of the file at `path` whose metadata (read from the open handle) is `metadata`.
    pub fn of(path: &Path, metadata: &Metadata) -> Self {
        Self {
            path: path.to_path_buf(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            identity: platform::identity(metadata),
        }
    }
}

/// The last [`CACHE_SIZE`] indexes used, least recently used out first. Shared by every thread that opens archives.
#[derive(Debug, Default)]
pub struct IndexCache {
    entries: Mutex<VecDeque<(Fingerprint, Arc<ArchiveIndex>)>>,
}

impl IndexCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The index of the archive in this state, which counts as used now.
    pub fn get(&self, fingerprint: &Fingerprint) -> Option<Arc<ArchiveIndex>> {
        let mut entries = self.lock();
        let at = entries.iter().position(|(key, _)| key == fingerprint)?;
        let found = entries.remove(at)?;
        let index = Arc::clone(&found.1);
        entries.push_back(found);

        Some(index)
    }

    /// Keeps `index` for the archive in this state, and drops the one used longest ago when there are too many. An
    /// older index of the same file (another state) goes at once.
    pub fn put(&self, fingerprint: Fingerprint, index: Arc<ArchiveIndex>) {
        let mut entries = self.lock();

        entries.retain(|(key, _)| key.path != fingerprint.path);
        entries.push_back((fingerprint, index));

        while entries.len() > CACHE_SIZE {
            entries.pop_front();
        }
    }

    /// Drops what is kept for the archive at `path`.
    pub fn forget(&self, path: &Path) {
        self.lock().retain(|(key, _)| key.path != path);
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<(Fingerprint, Arc<ArchiveIndex>)>> {
        self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

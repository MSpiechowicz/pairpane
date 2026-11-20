//! Comparing content by BLAKE3 hash: to check a fresh copy against its source (verify), and to answer a
//! conflict's "compare".
//!
//! A hash computed soon after the copy is usually read back from the system's cache, not from the disk, so a
//! match proves the copy logic more than the disk.

use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek};
use std::path::Path;

use crate::model::JobPhase;

use super::open;
use super::report::refusal;
use super::transfer::{Run, Stop};

/// Bytes hashed between two checkpoints.
const CHUNK: usize = 1024 * 1024;

impl Run<'_> {
    /// Hashes `source` and its `copy` from the start and compares. The phase is `verifying` meanwhile.
    ///
    /// # Errors
    ///
    /// Fails the item when the two differ: the copy is not committed.
    pub(super) fn verify(&mut self, source: &mut File, copy: &mut File) -> Result<(), Stop> {
        self.phase(JobPhase::Verifying);

        let hashes = self.hash(source).and_then(|original| Ok((original, self.hash(copy)?)));
        self.phase(JobPhase::Copying);
        let (original, copied) = hashes?;

        // A test can make every copy differ, to see a mismatch handled.
        if original != copied || self.inner.hooks.corrupt_verify {
            return Err(refusal(io::ErrorKind::InvalidData, "the copy does not match the original").into());
        }

        Ok(())
    }

    /// Whether the file `source_path` (seen as `source`) and the file at `target` hold the same bytes. A target
    /// that is no regular file, or a pair of different sizes, is not the same without reading anything. A file
    /// that cannot be read is not the same either.
    pub(super) fn same_content(&mut self, source_path: &Path, source: &Metadata, target: &Path) -> Result<bool, Stop> {
        let Ok(seen) = fs::symlink_metadata(target) else {
            return Ok(false);
        };
        if !source.is_file() || !seen.is_file() || source.len() != seen.len() {
            return Ok(false);
        }

        let (Ok(mut first), Ok(mut second)) = (open::source(source_path, source), open::source(target, &seen)) else {
            return Ok(false);
        };

        match self.hash(&mut first).and_then(|a| Ok(a == self.hash(&mut second)?)) {
            Err(Stop::Failed(_)) => Ok(false),
            result => result,
        }
    }

    /// The BLAKE3 hash of `file`, read from its start, stopping between chunks for a pause or a cancel.
    fn hash(&mut self, file: &mut File) -> Result<blake3::Hash, Stop> {
        file.rewind()?;

        let mut hasher = blake3::Hasher::new();
        self.buffer.resize(CHUNK, 0);

        loop {
            self.checkpoint()?;

            let read = match file.read(&mut self.buffer) {
                Ok(0) => return Ok(hasher.finalize()),
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            hasher.update(&self.buffer[..read]);
            self.tick(false);
        }
    }
}

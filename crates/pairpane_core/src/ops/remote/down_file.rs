//! One file of a download: asked about, read in packets into a temporary file, checked, and put in place; for a move, the
//! original is removed last, only when it is seen unchanged.
//!
//! The file is read exactly as large as the server says (one byte more is read, to see it), in packets of at most
//! [`CHUNK`] so a pause or a cancel is noticed. Its mode and time follow the rule of an extraction (`extract_write`). A move
//! removes the original when the copy is committed and flushed (the folder too), and the file is the one that was read: the
//! open handle and the name show the same size and time as at the start, and the server gave a time at all. Anything else
//! keeps the original and reports the item.
//!
//! What this cannot see: a file rewritten to the same size within the same second (SFTP times are whole seconds) shows
//! the same size and time at the end as at the start, and the move then removes the newer original. A caller that cannot
//! accept that copies instead of moving (see `RemoteFs`).

use std::fs::File;
use std::io;
use std::path::Path;

use crate::model::{FileFacts, JobPhase};
use crate::vfs::{RemoteKind, RemoteMeta, RemotePath, RemoteReader};

use super::super::conflict::Place;
use super::super::copy::{flush, flush_folder, in_context};
use super::super::meta;
use super::super::open;
use super::super::partial::Partial;
use super::super::report::refusal;
use super::super::task::Tally;
use super::super::transfer::{Run, Stop};
use super::Source;
use super::down::Down;
use super::own::{Own, same_name};
use super::walk::changed;

/// Bytes asked of the server between two checkpoints.
const CHUNK: u64 = 256 * 1024;

/// The permission bits of a file whose server states none.
const DEFAULT_FILE_MODE: u32 = 0o644;

impl Run<'_> {
    /// Downloads the file `remote` as `name` into the folder `to`: asks what to do when the target exists, writes a
    /// temporary file and commits it; a move then removes the file from the server.
    pub(super) fn download_file(
        &mut self,
        down: &mut Down<'_>,
        remote: &RemotePath,
        to: &Path,
        name: &str,
        rel: &str,
    ) -> Result<(), Stop> {
        let Some(seen) = down.fs.lstat(remote)? else {
            return Err(io::Error::from(io::ErrorKind::NotFound).into());
        };

        match seen.kind {
            RemoteKind::File => {}
            RemoteKind::Dir => return Err(changed().into()),
            RemoteKind::Link | RemoteKind::Other => {
                self.skipped += 1;

                return Ok(());
            }
        }

        let facts = FileFacts {
            size: seen.size,
            modified: seen.modified_ms,
            dir: false,
        };
        let target = self.local(to, name);
        let mut place = self.place_own(&down.own, rel, &facts, &target)?;

        if place == Place::Skip {
            self.skipped += 1;

            return Ok(());
        }

        let mut reader = down.fs.open_read(remote).map_err(|error| self.interrupted(error))?;
        let opened = reader.meta()?;

        if opened.kind != RemoteKind::File {
            return Err(changed().into());
        }
        if self.moving() && opened.modified_ms.is_none() {
            return Err(refusal(
                io::ErrorKind::Other,
                "Kept the original: the server does not tell when it was changed",
            )
            .into());
        }

        let (temp, mut copy) = self.make_temp(to, open::create_new)?;
        let mut guard = Partial::new(temp.clone());

        let received = self.receive(&mut *reader, &mut copy, opened.size)?;
        self.checkpoint()?;
        if let Some(hash) = received {
            self.check_copy(&mut copy, hash)?;
        }

        // On Windows a read-only file cannot be renamed over, so the attribute is given once the file is in place.
        let mode = opened.mode.unwrap_or(DEFAULT_FILE_MODE);
        let (now, read_only) = split_read_only(mode);

        meta::apply_entry(&copy, Some(now), opened.modified_ms)?;

        // A move removes the original once the copy is committed, so the copy must be on the disk first.
        if self.moving() {
            self.keep_on_disk(&copy)?;
        }
        drop(copy);

        let own = &down.own;

        if !self.commit_with(&temp, &mut place, |run, again| run.place_own(own, rel, &facts, again))? {
            self.skipped += 1;

            return Ok(());
        }
        guard.committed = true;
        self.mark_own(&mut down.own, place.path());

        if read_only && let Err(error) = make_read_only(place.path()) {
            self.fail(rel, &error);
        }

        if self.moving() {
            flush_folder(to)?;
            self.remove_original(down, remote, reader, &opened)?;
        }

        Ok(())
    }

    /// [`Run::place_entry`], except that a target this job wrote itself is never replaced: it is told that two items of
    /// the server have one name here.
    fn place_own(&mut self, own: &Own, rel: &str, facts: &FileFacts, target: &Path) -> Result<Place, Stop> {
        let place = self.place_entry(rel, facts, target)?;

        if matches!(place, Place::Replace(_)) && self.is_own(own, place.path()) {
            return Err(same_name().into());
        }

        Ok(place)
    }

    /// Reads the file `reader` holds into `copy`: exactly `size` bytes, or the file is not what the server said. Returns
    /// the hash of what was read when the job verifies.
    fn receive(
        &mut self,
        reader: &mut dyn RemoteReader,
        copy: &mut File,
        size: u64,
    ) -> Result<Option<blake3::Hash>, Stop> {
        if !self.job.request.verify {
            self.receive_into(reader, copy, size, &mut crc32fast::Hasher::new())?;

            return Ok(None);
        }

        let mut hasher = blake3::Hasher::new();
        self.receive_into(reader, copy, size, &mut hasher)?;

        Ok(Some(hasher.finalize()))
    }

    fn receive_into(
        &mut self,
        reader: &mut dyn RemoteReader,
        copy: &mut File,
        size: u64,
        seen: &mut impl Tally,
    ) -> Result<(), Stop> {
        let mut source = Source {
            reader,
            cancel: self.job.control.abort_flag(),
        };
        // One byte more than the size is read, to find out whether there is more.
        let limit = size.saturating_add(1);
        let mut got = 0_u64;

        while got < limit {
            let slice = CHUNK.min(limit - got);
            let count = self.pump(&mut source, copy, slice, seen)?;
            got += count;

            if count < slice {
                break;
            }
        }

        match got.cmp(&size) {
            std::cmp::Ordering::Less => Err(refusal(
                io::ErrorKind::UnexpectedEof,
                "the file ended before its size on the server",
            )
            .into()),
            std::cmp::Ordering::Greater => Err(refusal(
                io::ErrorKind::InvalidData,
                "the file is larger than its size on the server",
            )
            .into()),
            std::cmp::Ordering::Equal => Ok(()),
        }
    }

    /// Fails unless what `copy` holds on the disk hashes as what was received.
    fn check_copy(&mut self, copy: &mut File, received: blake3::Hash) -> Result<(), Stop> {
        self.phase(JobPhase::Verifying);
        let written = self.hash(copy);
        self.phase(JobPhase::Copying);

        // A test can make every copy differ, to see a mismatch handled.
        if written? != received || self.inner.hooks.corrupt_verify {
            return Err(refusal(io::ErrorKind::InvalidData, "the copy does not match the original").into());
        }

        Ok(())
    }

    /// Writes `copy` to the disk, where a write that fails only then (a full or pulled drive) fails the item.
    fn keep_on_disk(&self, copy: &File) -> Result<(), Stop> {
        if self.inner.hooks.fail_flush {
            return Err(io::Error::other("the disk failed to keep the copy (a test asked for it)").into());
        }

        flush(copy.sync_all())
            .map_err(|error| in_context("Kept the original: the copy could not be written to the disk", &error))?;

        Ok(())
    }

    /// Removes the file `remote` from the server after the move of it: only when it is still the one that was read (`opened`
    /// is what the server said of it when the reader opened it). The original is kept, and the item reported, otherwise.
    fn remove_original(
        &mut self,
        down: &Down<'_>,
        remote: &RemotePath,
        mut reader: Box<dyn RemoteReader>,
        opened: &RemoteMeta,
    ) -> Result<(), Stop> {
        // A pause ends before the looks: they must be of the moment of the removal.
        self.checkpoint()?;

        let unchecked =
            |error: io::Error| in_context("Kept the original: it could not be checked after the copy", &error);
        let at_end = reader.meta().map_err(unchecked)?;
        drop(reader);
        let named = down.fs.lstat(remote).map_err(unchecked)?;

        let same = |now: &RemoteMeta| {
            now.kind == RemoteKind::File && now.size == opened.size && now.modified_ms == opened.modified_ms
        };
        if !same(&at_end) || !named.as_ref().is_some_and(same) {
            return Err(refusal(
                io::ErrorKind::Other,
                "Kept the original: it changed while it was being moved",
            )
            .into());
        }

        // Not repeated by anyone: after a lost connection nobody knows whether the file is gone.
        // Where the file is, once more and as late as it can be: a folder swapped for a link would have the removal act in
        // another folder.
        if let Some(parent) = remote.parent() {
            down.home.confirm(down.fs, &parent)?;
        }

        down.fs.remove_file(remote).map_err(|error| {
            let what = if error.kind() == io::ErrorKind::NotFound {
                "The copy was made, but the original was already gone from the server"
            } else {
                "The copy was made, but the original may still be on the server"
            };

            in_context(what, &error)
        })?;

        Ok(())
    }
}

/// The mode a file is given before it is put in place, and whether it is made read-only after (Windows has no other bit
/// of the mode to give).
#[cfg(windows)]
fn split_read_only(mode: u32) -> (u32, bool) {
    (mode | 0o200, mode & 0o200 == 0)
}

#[cfg(not(windows))]
fn split_read_only(mode: u32) -> (u32, bool) {
    (mode, false)
}

/// Gives the file at `path` the read-only attribute.
#[cfg(windows)]
fn make_read_only(path: &Path) -> io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_readonly(true);

    std::fs::set_permissions(path, permissions)
}

#[cfg(not(windows))]
fn make_read_only(_: &Path) -> io::Result<()> {
    Ok(())
}

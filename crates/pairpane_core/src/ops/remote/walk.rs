//! What a download walks on the server, and what it takes care of while it does.
//!
//! A folder is looked at before it is listed (a folder swapped for a link since it was listed is never read), and what
//! the walk holds is bounded: the entries of the folders it is inside of, and in all those it lists. A move checks where
//! a folder is before it removes anything from it (see [`Home`]).

use std::io;

use crate::vfs::{RemoteEntry, RemoteFs, RemoteKind, RemotePath};

use super::super::copy::in_context;
use super::super::report::refusal;
use super::super::transfer::{MAX_DEPTH, Run, Stop};
use super::down::Down;
use super::usable_here;

/// Entries of the folders a walk is inside of (listed, and not walked through yet).
const HELD_ENTRIES: usize = 2_000_000;

/// Bytes of the names of those entries.
const HELD_NAME_BYTES: usize = 256 * 1024 * 1024;

/// Entries listed in one walk over the whole job (the scan, and then the copy).
const WALKED_ENTRIES: usize = 20_000_000;

/// The error of a walk that would hold more than it may; it fails the job, not an item.
fn too_large() -> io::Error {
    refusal(
        io::ErrorKind::OutOfMemory,
        "the folder on the server is too large to copy at once",
    )
}

/// The error of a folder that is not the one that was walked to.
pub(super) fn changed() -> io::Error {
    refusal(io::ErrorKind::Other, "it changed while it was being copied")
}

/// What a walk may hold, and what it holds.
pub(super) struct Budget {
    most: (usize, usize, usize),
    entries: usize,
    bytes: usize,
    walked: usize,
}

impl Budget {
    pub(super) fn new(limits: Option<(usize, usize, usize)>) -> Self {
        Self {
            most: limits.unwrap_or((HELD_ENTRIES, HELD_NAME_BYTES, WALKED_ENTRIES)),
            entries: 0,
            bytes: 0,
            walked: 0,
        }
    }

    /// A walk starts over: the scan is done, and the copy lists the same folders again.
    pub(super) fn restart(&mut self) {
        self.walked = 0;
    }

    /// Counts the raw listing `all`, and fails when holding it would pass a limit.
    fn admit(&mut self, all: &[RemoteEntry]) -> io::Result<()> {
        let (held, bytes, walked) = self.most;
        let names = all.iter().map(|entry| entry.name.len()).sum::<usize>();

        if self.entries.saturating_add(all.len()) > held
            || self.bytes.saturating_add(names) > bytes
            || self.walked.saturating_add(all.len()) > walked
        {
            return Err(too_large());
        }

        self.walked += all.len();

        Ok(())
    }

    fn hold(&mut self, listed: &Listed) {
        self.entries += listed.entries.len();
        self.bytes += listed.bytes;
    }

    /// The walk is done with `listed`, which it held.
    pub(super) fn release(&mut self, listed: &Listed) {
        self.entries = self.entries.saturating_sub(listed.entries.len());
        self.bytes = self.bytes.saturating_sub(listed.bytes);
    }
}

/// A folder of the server as the download sees it: what it holds that can be copied, and how many it left out.
pub(super) struct Listed {
    pub(super) entries: Vec<RemoteEntry>,
    pub(super) skipped: u32,
    /// The folder holds the file laid in the target folder, of any kind (see `Alias`).
    pub(super) has_probe: bool,
    /// Bytes of the names of `entries`.
    bytes: usize,
}

/// What a move holds of the folder where the job's items are, from the start.
struct Start {
    /// The folder with its links resolved.
    resolved: RemotePath,
    /// What the folder itself is: a folder, or a link the user chose (which is followed as the server resolves it).
    kind: RemoteKind,
}

/// The folder where the job's items are, and where that is for the server: what a move checks before it removes.
pub(super) struct Home {
    dir: RemotePath,
    /// `None` for a copy, which changes nothing on the server.
    start: Option<Start>,
}

impl Home {
    /// Looks at `dir` once. A move needs a server that can say where a folder is, and refuses one that cannot.
    pub(super) fn of(fs: &dyn RemoteFs, dir: &RemotePath, moving: bool) -> io::Result<Self> {
        if !moving {
            return Ok(Self {
                dir: dir.clone(),
                start: None,
            });
        }

        let resolved = match fs.realpath(dir) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                return Err(refusal(
                    io::ErrorKind::Unsupported,
                    "this server cannot tell where a folder is, so its items cannot be moved",
                ));
            }
            Err(error) => return Err(error),
        };

        let kind = match fs.lstat(dir)? {
            Some(meta) if matches!(meta.kind, RemoteKind::Dir | RemoteKind::Link) => meta.kind,
            Some(_) => {
                return Err(refusal(
                    io::ErrorKind::InvalidInput,
                    "the items are not in a folder on the server",
                ));
            }
            None => return Err(io::ErrorKind::NotFound.into()),
        };

        Ok(Self {
            dir: dir.clone(),
            start: Some(Start { resolved, kind }),
        })
    }

    /// Fails unless `folder` (the job's folder, or below it) is still where the walk went: nothing on the way to it has
    /// been swapped for a link since. A server cannot be asked to remove an item only if its folder is the one checked, so a
    /// swap between this and the removal itself goes unseen here; [`Home::confirm_after`] reports it.
    pub(super) fn confirm(&self, fs: &dyn RemoteFs, folder: &RemotePath) -> io::Result<()> {
        self.settle(
            fs,
            folder,
            (
                "Kept the original: the folder on the server changed while it was moved",
                "Kept the original: it could not be checked after the copy",
            ),
        )
    }

    /// Fails when `folder` is not where the walk went, after something was removed from it: the removal may have been
    /// done in another folder.
    pub(super) fn confirm_after(&self, fs: &dyn RemoteFs, folder: &RemotePath) -> io::Result<()> {
        self.settle(
            fs,
            folder,
            (
                "the folder on the server changed during the removal; something in another folder may have been removed",
                "It was removed, but the folder it was in could not be checked afterwards",
            ),
        )
    }

    /// Fails with the first words when `folder` is not where the walk went, and with the second when that cannot be told.
    fn settle(
        &self,
        fs: &dyn RemoteFs,
        folder: &RemotePath,
        (moved, unchecked): (&'static str, &str),
    ) -> io::Result<()> {
        match self.is_there(fs, folder) {
            Ok(true) => Ok(()),
            Ok(false) => Err(refusal(io::ErrorKind::Other, moved)),
            Err(error) => Err(in_context(unchecked, &error)),
        }
    }

    /// Whether `folder` is where the walk went. Each folder on the way is looked at by its own name (a look never follows the
    /// last name, so a server that does not resolve links shows a swap too), and the server is asked where the whole is.
    fn is_there(&self, fs: &dyn RemoteFs, folder: &RemotePath) -> io::Result<bool> {
        let Some(start) = &self.start else {
            return Ok(true);
        };

        if !is_a(fs, &self.dir, start.kind)? {
            return Ok(false);
        }

        let mut named = self.dir.clone();
        let mut expected = start.resolved.clone();

        for name in folder.components().skip(self.dir.components().count()) {
            named = named.join(name)?;
            expected = expected.join(name)?;

            if !is_a(fs, &named, RemoteKind::Dir)? {
                return Ok(false);
            }
        }

        Ok(fs.realpath(folder)? == expected)
    }
}

/// Whether the server says `path` is of `kind`, itself and not what it leads to.
fn is_a(fs: &dyn RemoteFs, path: &RemotePath, kind: RemoteKind) -> io::Result<bool> {
    Ok(fs.lstat(path)?.is_some_and(|meta| meta.kind == kind))
}

impl Down<'_> {
    /// The entries of the folder `dir`, once it is seen to be a folder (and not a link, which a listing would follow).
    pub(super) fn entries_of(
        &self,
        dir: &RemotePath,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> io::Result<Vec<RemoteEntry>> {
        match self.fs.lstat(dir)? {
            Some(meta) if meta.kind == RemoteKind::Dir => self.fs.read_dir(dir, cancel),
            Some(_) => Err(changed()),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }
}

impl Run<'_> {
    /// The entries of the server's folder `dir` that can be copied. What the listing holds stays counted in the budget until
    /// [`Budget::release`].
    pub(super) fn list(&mut self, down: &mut Down<'_>, dir: &RemotePath) -> Result<Listed, Stop> {
        let all = down
            .entries_of(dir, self.job.control.abort_flag())
            .map_err(|error| self.interrupted(error))?;

        down.budget.admit(&all)?;

        let has_probe = all.iter().any(|entry| entry.name == down.probe);
        let mut listed = Listed {
            entries: Vec::new(),
            skipped: 0,
            has_probe,
            bytes: 0,
        };

        for entry in all {
            // The server may list the folder and its parent: not items.
            if entry.name == "." || entry.name == ".." {
                continue;
            }

            let copyable = matches!(entry.meta.kind, RemoteKind::Dir | RemoteKind::File);
            if copyable && usable_here(&entry.name) {
                listed.bytes += entry.name.len();
                listed.entries.push(entry);
            } else {
                listed.skipped += 1;
            }
        }

        down.budget.hold(&listed);

        Ok(listed)
    }

    /// Counts what is to be copied below `item`, which is in `dir`, and refuses the job when the target is among it. A
    /// folder that cannot be listed is left to the copy, which reports it.
    pub(super) fn scan_remote(
        &mut self,
        down: &mut Down<'_>,
        dir: &RemotePath,
        item: &RemoteEntry,
        depth: usize,
    ) -> Result<(), Stop> {
        if item.meta.kind == RemoteKind::File {
            self.found(item.meta.size);

            return Ok(());
        }
        if depth >= MAX_DEPTH {
            return Ok(());
        }

        let path = dir.join(&item.name)?;
        let listed = match self.list(down, &path) {
            Ok(listed) => listed,
            Err(Stop::Failed(error)) if error.kind() != io::ErrorKind::OutOfMemory => return Ok(()),
            Err(stop) => return Err(stop),
        };

        let scanned = self.scan_children(down, &path, &listed, depth);
        down.budget.release(&listed);

        scanned
    }

    fn scan_children(
        &mut self,
        down: &mut Down<'_>,
        path: &RemotePath,
        listed: &Listed,
        depth: usize,
    ) -> Result<(), Stop> {
        refuse_inside(listed)?;

        for child in &listed.entries {
            self.checkpoint()?;
            self.scan_remote(down, path, child, depth + 1)?;
        }

        Ok(())
    }
}

/// Fails the job when `listed` holds the probe: the target folder is inside a folder that is copied.
pub(super) fn refuse_inside(listed: &Listed) -> Result<(), Stop> {
    if listed.has_probe {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "a folder cannot be copied or moved into itself",
        )
        .into());
    }

    Ok(())
}

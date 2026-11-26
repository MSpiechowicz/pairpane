//! Where the items of a move are on the server, and the check that a folder is still there before and after a removal.
//!
//! A server is asked to remove a path, and a folder on that path may be swapped for a link between a look and the
//! removal. So a move looks at every name on the way, from `/` down to the folder it removes from, right before and right
//! after each removal: each is looked at by its own name (a look never follows the last name, so a server that does not
//! resolve links in `realpath` shows a swap too), the kinds must be those of the start, a link must point where it
//! pointed (a link the user chose for the job's folder is followed, but not one repointed), and the server is asked where
//! the whole is. The names of one way are looked at together (see `RemoteFs::lstat_many`).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vfs::{RemoteFs, RemoteKind, RemoteMeta, RemotePath};

use super::super::copy::in_context;
use super::super::report::refusal;

/// What a name on the way to the job's folder is at the start: its kind and, for a link, where it points as written.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    kind: RemoteKind,
    target: Option<String>,
}

impl Step {
    /// Whether `now` (what the server says of `path` itself) is what this was at the start.
    fn holds(&self, fs: &dyn RemoteFs, path: &RemotePath, now: &RemoteMeta, cancel: &AtomicBool) -> io::Result<bool> {
        if now.kind != self.kind {
            return Ok(false);
        }

        match &self.target {
            Some(target) => Ok(link_target(fs, path, cancel)? == *target),
            None => Ok(true),
        }
    }
}

/// What a move holds of the folder where the job's items are, from the start.
struct Start {
    /// The folder with its links resolved.
    resolved: RemotePath,
    /// Each name from `/` down to the folder (that one last): its path and what it was.
    way: Vec<(RemotePath, Step)>,
}

/// The folder where the job's items are, and where that is for the server: what a move checks before it removes.
pub(super) struct Home {
    dir: RemotePath,
    /// `None` for a copy, which changes nothing on the server.
    start: Option<Start>,
}

impl Home {
    /// Looks at `dir` once. A move needs a server that can say where a folder is and where a link points, and refuses one
    /// that cannot.
    pub(super) fn of(fs: &dyn RemoteFs, dir: &RemotePath, moving: bool, cancel: &AtomicBool) -> io::Result<Self> {
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

        let paths = prefixes(dir)?;
        let mut way = Vec::with_capacity(paths.len());

        for (path, answer) in paths.iter().zip(fs.lstat_many(&paths, cancel)) {
            let meta = answer?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

            if !matches!(meta.kind, RemoteKind::Dir | RemoteKind::Link) {
                return Err(refusal(
                    io::ErrorKind::InvalidInput,
                    "the items are not in a folder on the server",
                ));
            }

            let target = if meta.kind == RemoteKind::Link {
                Some(link_target(fs, path, cancel)?)
            } else {
                None
            };

            way.push((
                path.clone(),
                Step {
                    kind: meta.kind,
                    target,
                },
            ));
        }

        Ok(Self {
            dir: dir.clone(),
            start: Some(Start { resolved, way }),
        })
    }

    /// Fails unless `folder` (the job's folder, or below it) is still where the walk went: nothing on the way to it has
    /// been swapped for a link since. A server cannot be asked to remove an item only if its folder is the one checked, so a
    /// swap between this and the removal itself goes unseen here; [`Home::confirm_after`] reports it. A call that is
    /// cancelled fails with `Interrupted`.
    pub(super) fn confirm(&self, fs: &dyn RemoteFs, folder: &RemotePath, cancel: &AtomicBool) -> io::Result<()> {
        self.settle(
            fs,
            folder,
            cancel,
            (
                "Kept the original: the folder on the server changed while it was moved",
                "Kept the original: it could not be checked after the copy",
            ),
        )
    }

    /// Fails when `folder` is not where the walk went, after something was removed from it: the removal may have been
    /// done in another folder.
    pub(super) fn confirm_after(&self, fs: &dyn RemoteFs, folder: &RemotePath, cancel: &AtomicBool) -> io::Result<()> {
        self.settle(
            fs,
            folder,
            cancel,
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
        cancel: &AtomicBool,
        (moved, unchecked): (&'static str, &str),
    ) -> io::Result<()> {
        match self.is_there(fs, folder, cancel) {
            Ok(true) => Ok(()),
            Ok(false) => Err(refusal(io::ErrorKind::Other, moved)),
            Err(error) => Err(in_context(unchecked, &error)),
        }
    }

    /// Whether `folder` is where the walk went: every name from `/` down to it is what it was (the way to the job's folder)
    /// or a folder (below it), and the server says the whole is where it was.
    fn is_there(&self, fs: &dyn RemoteFs, folder: &RemotePath, cancel: &AtomicBool) -> io::Result<bool> {
        let Some(start) = &self.start else {
            return Ok(true);
        };

        if cancel.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }

        let mut paths: Vec<RemotePath> = start.way.iter().map(|(path, _)| path.clone()).collect();
        let mut named = self.dir.clone();
        let mut expected = start.resolved.clone();

        for name in folder.components().skip(self.dir.components().count()) {
            named = named.join(name)?;
            expected = expected.join(name)?;
            paths.push(named.clone());
        }

        for (index, (path, answer)) in paths.iter().zip(fs.lstat_many(&paths, cancel)).enumerate() {
            let Some(meta) = answer? else {
                return Ok(false);
            };
            let holds = match start.way.get(index) {
                Some((_, step)) => step.holds(fs, path, &meta, cancel)?,
                None => meta.kind == RemoteKind::Dir,
            };

            if !holds {
                return Ok(false);
            }
        }

        Ok(fs.realpath(folder)? == expected)
    }
}

/// The paths of every name of `path`, from the first below `/` to the last: `/a`, `/a/b` for `/a/b`.
fn prefixes(path: &RemotePath) -> io::Result<Vec<RemotePath>> {
    let mut named = RemotePath::root();

    path.components()
        .map(|name| {
            named = named.join(name)?;

            Ok(named.clone())
        })
        .collect()
}

/// Where the link `path` points, as written. A server that cannot say refuses the move: a link repointed could not be seen.
fn link_target(fs: &dyn RemoteFs, path: &RemotePath, cancel: &AtomicBool) -> io::Result<String> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }

    fs.read_link(path).map_err(|error| {
        if error.kind() == io::ErrorKind::Unsupported {
            refusal(
                io::ErrorKind::Unsupported,
                "this server cannot tell where a link points, so its items cannot be moved through one",
            )
        } else {
            error
        }
    })
}

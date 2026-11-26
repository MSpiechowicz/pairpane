//! Where the items of a move are on the server, and the check that a folder is still there before and after a removal.
//!
//! A server is asked to remove a path, and a folder on that path may be swapped for a link between a look and the
//! removal. So a move looks at every name on the way, from `/` down to the folder it removes from, right before and right
//! after each removal: each is looked at by its own name (a look never follows the last name, so a server that does not
//! resolve links in `realpath` shows a swap too), the kinds must be those of the start, a link must point where it
//! pointed (a link the user chose for the job's folder is followed, but not one repointed), and the server is asked where
//! the whole is. The names of one way are looked at together (see `RemoteFs::lstat_many`).
//!
//! A server that does not resolve links in `realpath` says the folder is where its name says, however many links are on
//! the way, so a swap of a folder that a link leads through goes unseen by that call. The move therefore also looks at
//! each name of the folder the server says (the real way), and refuses to move through a link where that is itself a link:
//! it cannot tell where the link really leads.
//!
//! Known bound: the looks of one check go out up to 64 at a time, so a way of `n` names costs about `n / 64` round trips
//! for each check, and a move of a tree `n` folders deep about `n * n / 64` looks in all.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::vfs::{RemoteFs, RemoteKind, RemoteMeta, RemotePath};

use super::super::copy::in_context;
use super::super::report::refusal;

/// How long the check after a removal goes on once the job is cancelled, before it is given up and the removal is reported as
/// not checked.
pub(super) const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// How often the wait for a cancel looks at the flag (the flag has no way to wake a waiter).
const CANCEL_POLL: Duration = Duration::from_millis(20);

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
    /// Whether `resolved` is not the folder itself (the server resolved a link in it).
    apart: bool,
    /// Each name from `/` down to `resolved` when it is apart (else empty): all folders.
    real: Vec<RemotePath>,
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

        let mut paths = prefixes(dir)?;
        let ways = paths.len();
        let apart = resolved != *dir;

        if apart {
            paths.extend(prefixes(&resolved)?);
        }

        let answers = looks(fs, &paths, cancel)?;
        let mut metas = Vec::with_capacity(paths.len());

        for answer in answers {
            metas.push(answer?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?);
        }

        // Where the server says the folder is is a way of folders: a link there is a server that does not resolve links
        // (when it says the folder is where it was asked, that way is the one looked at first).
        let real = if apart { &metas[ways..] } else { &metas[..ways] };

        for meta in real {
            match meta.kind {
                RemoteKind::Dir => {}
                RemoteKind::Link => {
                    return Err(refusal(
                        io::ErrorKind::Unsupported,
                        "this server does not say where a linked folder really is, so its items cannot be moved through a link",
                    ));
                }
                _ => {
                    return Err(refusal(
                        io::ErrorKind::InvalidInput,
                        "the items are not in a folder on the server",
                    ));
                }
            }
        }

        let mut way = Vec::with_capacity(ways);

        for (path, meta) in paths.iter().zip(&metas).take(ways) {
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
            start: Some(Start {
                resolved,
                apart,
                real: paths.split_off(ways),
                way,
            }),
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
    /// done in another folder. A cancel (`abort` is the flag of the job) does not interrupt it at first: a removal that went
    /// elsewhere must be reported even when the job is cancelled right after. But a server that stalls must not hold a
    /// cancelled job for the many requests of the check, so `grace` after the cancel the check is given up, and fails
    /// (with the kind `Other`, so that it is reported, not taken for the cancel) as a check that could not be made.
    pub(super) fn confirm_after(
        &self,
        fs: &dyn RemoteFs,
        folder: &RemotePath,
        (abort, grace): (&AtomicBool, Duration),
    ) -> io::Result<()> {
        const UNCHECKED: &str = "It was removed, but the folder it was in could not be checked afterwards";

        let given_up = AtomicBool::new(false);
        let (finished, over) = mpsc::channel::<()>();

        let checked = thread::scope(|scope| {
            let given_up = &given_up;

            scope.spawn(move || {
                if cancelled_before_end(&over, abort) && over.recv_timeout(grace) == Err(RecvTimeoutError::Timeout) {
                    given_up.store(true, Ordering::Release);
                }
            });

            let checked = self.settle(
                fs,
                folder,
                given_up,
                (
                    "the folder on the server changed during the removal; something in another folder may have been removed",
                    UNCHECKED,
                ),
            );
            drop(finished);

            checked
        });

        checked.map_err(|error| {
            if error.kind() == io::ErrorKind::Interrupted {
                io::Error::other(format!(
                    "{UNCHECKED}: the job was cancelled and the server did not answer in time"
                ))
            } else {
                error
            }
        })
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
        let mut real = Vec::new();

        for name in folder.components().skip(self.dir.components().count()) {
            named = named.join(name)?;
            expected = expected.join(name)?;
            paths.push(named.clone());

            if start.apart {
                real.push(expected.clone());
            }
        }

        // Where the server says the folder is: a folder there is a link now when something was swapped under a link.
        paths.extend(start.real.iter().cloned());
        paths.extend(real);

        for (index, (path, answer)) in paths.iter().zip(looks(fs, &paths, cancel)?).enumerate() {
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

        if cancel.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }

        Ok(fs.realpath(folder)? == expected)
    }
}

/// Waits until `abort` is set (`true`) or the check is over (`false`, once the sender of `over` is dropped).
fn cancelled_before_end(over: &Receiver<()>, abort: &AtomicBool) -> bool {
    loop {
        match over.recv_timeout(CANCEL_POLL) {
            Err(RecvTimeoutError::Timeout) if abort.load(Ordering::Acquire) => return true,
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// [`RemoteFs::lstat_many`] of `paths`, which fails when the server answers other than once for each (the answers are
/// paired with the paths, and a missing one would pass as a check not made).
fn looks(
    fs: &dyn RemoteFs,
    paths: &[RemotePath],
    cancel: &AtomicBool,
) -> io::Result<Vec<io::Result<Option<RemoteMeta>>>> {
    let answers = fs.lstat_many(paths, cancel);

    if answers.len() != paths.len() {
        return Err(refusal(
            io::ErrorKind::InvalidData,
            "the server answered a different number of looks than were asked",
        ));
    }

    Ok(answers)
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

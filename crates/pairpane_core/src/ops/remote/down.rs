//! A download: the items of a server folder written into a folder of this computer.
//!
//! Before the first write the job looks at the server twice over: whether the server's folder is the target folder (or the
//! target is inside what is copied) is told by a file it lays in the target and looks for on the server (`Alias`), and what
//! there is to copy is counted. Then the items are walked, a folder made (with its mode and time last) and each file handed
//! to `down_file`. What fails is reported against its item; a server that is gone, or does not answer three items in a row,
//! fails the job. A move removes a folder from the server when it is empty. What the walk looks at on the server is in
//! `walk`, and what the job has put here itself in `own`.

use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::listing::platform;
use crate::model::JobPhase;
use crate::vfs::{RemoteEntry, RemoteFs, RemoteKind, RemotePath};

use super::super::RemoteJob;
use super::super::copy::in_context;
use super::super::meta;
use super::super::open;
use super::super::partial::Partial;
use super::super::report::refusal;
use super::super::transfer::{MAX_DEPTH, Run, Stop};
use super::home::{CANCEL_GRACE, Home};
use super::own::{Own, same_name};
use super::usable_here;
use super::walk::{Budget, Listed, refuse_inside};

/// Items in a row that fail because the server does not answer, after which the job gives up on it.
const MAX_STRIKES: u32 = 3;

/// The permission bits of a folder whose server states none.
const DEFAULT_FOLDER_MODE: u32 = 0o755;

/// What a download carries from one item to the next.
pub(super) struct Down<'j> {
    pub(super) fs: &'j dyn RemoteFs,
    /// The name of the file laid in the target folder (see [`Alias`]).
    pub(super) probe: &'j str,
    /// Items in a row that failed for want of an answer.
    strikes: u32,
    /// Where the items are, and where that is for the server.
    pub(super) home: Home,
    /// What the walk holds.
    pub(super) budget: Budget,
    /// What the job has written here.
    pub(super) own: Own,
}

/// A file laid in the target folder, with a name nothing else has: found in a folder of the server, it shows that this
/// folder is the target, or holds it. Removed when dropped, however the job ends.
struct Alias {
    name: String,
    _file: Partial,
}

impl Run<'_> {
    pub(super) fn download(&mut self, job: &RemoteJob) -> Result<(), Stop> {
        let request = &self.job.request;

        platform::check_folder(&request.to).map_err(|reason| refusal(io::ErrorKind::InvalidInput, reason))?;
        let to = self.target_folder(&request.to)?;

        let alias = self.lay_alias(&to)?;
        let mut down = Down {
            fs: &*job.fs,
            probe: &alias.name,
            strikes: 0,
            home: Home::of(&*job.fs, &job.dir, self.moving(), self.job.control.abort_flag())
                .map_err(|error| self.interrupted(error))?,
            budget: Budget::new(self.inner.hooks.walk_limits),
            own: Own::default(),
        };

        // The target is made and the probe laid; nothing of the server is written yet, and a refusal here undoes both.
        refuse_if_alias(&down, &job.dir.join(down.probe)?)?;
        let items = self.look_up(&mut down, &job.dir)?;

        self.phase(JobPhase::Scanning);
        for item in &items {
            self.checkpoint()?;
            self.scan_remote(&mut down, &job.dir, item, 0)?;
        }

        down.budget.restart();
        self.phase(JobPhase::Copying);
        for item in &items {
            self.checkpoint()?;
            let path = job.dir.join(&item.name)?;

            let done = self.download_node(&mut down, (&path, &to), item, &item.name, 0);
            self.download_done(&mut down, &item.name, done)?;
        }

        Ok(())
    }

    /// Lays the file that tells an alias in the target folder `to`. Its name is random and says nothing of this computer:
    /// the server sees it when the target is its own folder.
    fn lay_alias(&self, to: &Path) -> Result<Alias, Stop> {
        let mut token = [0_u8; 8];
        getrandom::fill(&mut token).map_err(|_| io::Error::other("no random name could be made"))?;

        let name = format!(".pairpane-{:016x}-probe", u64::from_le_bytes(token));
        let path = to.join(&name);

        drop(open::create_new(&path)?);

        Ok(Alias {
            name,
            _file: Partial::new(path),
        })
    }

    /// What the server says of each item of the job; one that is gone fails, and one that is not a file or a folder, or
    /// has a name that cannot be used here, is skipped.
    fn look_up(&mut self, down: &mut Down<'_>, dir: &RemotePath) -> Result<Vec<RemoteEntry>, Stop> {
        let job = self.job;
        let mut items = Vec::new();

        for name in &job.request.names {
            self.checkpoint()?;

            if !usable_here(name) {
                self.skipped += 1;
                continue;
            }

            let seen = match dir.join(name) {
                Ok(path) => down
                    .fs
                    .lstat(&path)
                    .and_then(|meta| meta.ok_or_else(|| io::ErrorKind::NotFound.into())),
                Err(error) => Err(error),
            };

            match seen {
                Ok(meta) if matches!(meta.kind, RemoteKind::Dir | RemoteKind::File) => {
                    down.strikes = 0;
                    items.push(RemoteEntry {
                        name: name.clone(),
                        meta,
                    });
                }
                Ok(_) => self.skipped += 1,
                Err(error) => self.download_failed(down, name, error)?,
            }
        }

        Ok(items)
    }

    /// The flag of a cancel and the time the check after a removal goes on once it is raised (see [`Home::confirm_after`]).
    pub(super) fn after_check(&self) -> (&AtomicBool, Duration) {
        (
            self.job.control.abort_flag(),
            self.inner.hooks.check_grace.unwrap_or(CANCEL_GRACE),
        )
    }

    /// What the check after the removal of `rel` that failed with `error` ends the item with. On a cancelled job the removal
    /// is reported first (a cancel must not hide it) and the job is cancelled; else the item fails with `error`.
    pub(super) fn removed_unchecked(&mut self, rel: &str, error: io::Error) -> Stop {
        if self.job.control.is_cancelled() {
            self.fail(rel, &error);

            Stop::Cancelled
        } else {
            Stop::Failed(error)
        }
    }

    /// Settles the item at `rel`, which came to `done`. A failure is reported and the job goes on, unless the server is
    /// gone or has stopped answering for several items in a row.
    fn download_done<T>(&mut self, down: &mut Down<'_>, rel: &str, done: Result<T, Stop>) -> Result<(), Stop> {
        match done {
            Ok(_) => {
                down.strikes = 0;

                Ok(())
            }
            Err(Stop::Cancelled) => Err(Stop::Cancelled),
            Err(Stop::Failed(error)) => self.download_failed(down, rel, error),
        }
    }

    /// [`Run::download_done`] for an item that failed with `error`.
    fn download_failed(&mut self, down: &mut Down<'_>, rel: &str, error: io::Error) -> Result<(), Stop> {
        match error.kind() {
            // A server that is gone, and a walk that would hold more than it may, end the job: the next item fares no better.
            io::ErrorKind::NotConnected | io::ErrorKind::OutOfMemory => return Err(Stop::Failed(error)),
            io::ErrorKind::TimedOut | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionRefused => {
                down.strikes += 1;
            }
            _ => down.strikes = 0,
        }

        self.fail(rel, &error);

        if down.strikes >= MAX_STRIKES {
            return Err(refusal(io::ErrorKind::TimedOut, "the server stopped answering").into());
        }

        Ok(())
    }

    /// Downloads `item` (the entry of the server folder whose path is `remote`) into the folder `to`.
    fn download_node(
        &mut self,
        down: &mut Down<'_>,
        (remote, to): (&RemotePath, &Path),
        item: &RemoteEntry,
        rel: &str,
        depth: usize,
    ) -> Result<(), Stop> {
        if item.meta.kind == RemoteKind::Dir {
            return self.download_folder(down, (remote, to), item, rel, depth);
        }

        self.begin_file(rel, item.meta.size);
        let done = self.download_file(down, remote, to, &item.name, rel);
        self.end_file();

        done
    }

    fn download_folder(
        &mut self,
        down: &mut Down<'_>,
        (remote, to): (&RemotePath, &Path),
        item: &RemoteEntry,
        rel: &str,
        depth: usize,
    ) -> Result<(), Stop> {
        if depth >= MAX_DEPTH {
            return Err(refusal(io::ErrorKind::InvalidInput, "the folders are nested too deeply").into());
        }

        // Listed before the folder is made here: one that cannot be read leaves no empty folder behind.
        let listed = self.list(down, remote)?;
        let done = self.download_below(down, (remote, &self.local(to, &item.name)), &listed, (item, rel), depth);

        down.budget.release(&listed);

        done
    }

    /// The contents `listed` of the folder `item` of the server, whose path is `remote`, written into `target`.
    fn download_below(
        &mut self,
        down: &mut Down<'_>,
        (remote, target): (&RemotePath, &Path),
        listed: &Listed,
        (item, rel): (&RemoteEntry, &str),
        depth: usize,
    ) -> Result<(), Stop> {
        refuse_inside(listed)?;
        self.skipped += listed.skipped;

        let created = self.make_folder(target, true)?;

        if created {
            self.mark_own(&mut down.own, target);
        } else if self.is_own(&down.own, target) {
            // Another folder of the server has this name here: its items are not merged into this one.
            return Err(same_name().into());
        }

        for child in &listed.entries {
            self.checkpoint()?;
            let path = remote.join(&child.name)?;
            let child_rel = format!("{rel}/{}", child.name);

            let done = self.download_node(down, (&path, target), child, &child_rel, depth + 1);
            self.download_done(down, &child_rel, done)?;
        }

        // Last, as adding the contents changes the folder's time (a folder that was there already is left as it is).
        if created {
            let mode = item.meta.mode.unwrap_or(DEFAULT_FOLDER_MODE);

            if let Err(error) = meta::apply_entry_folder(target, Some(mode), item.meta.modified_ms) {
                self.fail(rel, &error);
            }
        }

        // Its copy is whole by now: it is kept whatever the removal of the server's folder ends with, so a cancel during
        // that removal (once the server has removed it) does not take the only copy left of an empty folder.
        if created {
            self.created.finish(target);
        }

        if self.moving() {
            self.remove_folder(down, remote, rel)?;
        }

        Ok(())
    }

    /// Removes the folder `remote` from the server once a move emptied it: what was skipped or failed keeps it. Only when
    /// the folder it is in is still where the walk went (see [`Home::confirm`]). A folder that is kept for want of an
    /// answer is reported, and a cancel ends the job.
    fn remove_folder(&mut self, down: &Down<'_>, remote: &RemotePath, rel: &str) -> Result<(), Stop> {
        self.checkpoint()?;

        let Some(parent) = remote.parent() else {
            return Ok(());
        };
        let abort = self.job.control.abort_flag();

        if let Err(error) = down.home.confirm(down.fs, &parent, abort) {
            return self.report(rel, error);
        }

        let left = match down.entries_of(remote, abort) {
            Ok(left) => left,
            Err(error) => {
                let what =
                    "The folder was copied, but what is left in it could not be listed, so it stays on the server";

                return self.report(rel, in_context(what, &error));
            }
        };

        if !left.iter().all(|entry| entry.name == "." || entry.name == "..") {
            return Ok(());
        }

        if let Err(error) = down.fs.remove_dir(remote) {
            let what = if error.kind() == io::ErrorKind::NotFound {
                "The folder was copied, but it was already gone from the server"
            } else {
                "The folder was copied, but it may still be on the server"
            };

            self.fail(rel, &in_context(what, &error));

            return Ok(());
        }

        match down.home.confirm_after(down.fs, &parent, self.after_check()) {
            Ok(()) => Ok(()),
            Err(error) => match self.removed_unchecked(rel, error) {
                Stop::Failed(error) => self.report(rel, error),
                stopped => Err(stopped),
            },
        }
    }
}

/// Fails the job when the server's file `probe` is there: the target folder is the folder of the items.
fn refuse_if_alias(down: &Down<'_>, probe: &RemotePath) -> Result<(), Stop> {
    if down.fs.lstat(probe)?.is_some() {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the folder of the items is the target folder",
        )
        .into());
    }

    Ok(())
}

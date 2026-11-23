//! One job from start to end, on its worker: look at the paths, move top-level items by rename where that
//! works, count what is left, copy it, and clean up after a cancel or a failure.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{Answer, IpcError, ItemError, JobPhase, OpKind, Outcome};

use super::Task;
use super::control::Cancelled;
use super::engine::Inner;
use super::identity::FileId;
use super::job::Job;
use super::plan::Child;
use super::progress::Progress;
use super::rename;
use super::report;
use super::sync_terms::Terms;

/// Folders nested deeper than this are not walked (the walk recurses, so this bounds its stack).
pub(crate) const MAX_DEPTH: usize = 1_024;

/// Item errors reported with their paths; later ones are only counted.
const MAX_REPORTED_ERRORS: usize = 100;

/// What a job came to, for [`crate::model::OpsEvent::Ended`].
pub(super) struct Finish {
    pub(super) outcome: Outcome,
    pub(super) skipped: u32,
    pub(super) errors: Vec<ItemError>,
    pub(super) more_errors: u32,
}

impl Finish {
    /// A job that never ran: cancelled while it waited.
    pub(super) fn cancelled() -> Self {
        Self::of(Outcome::Cancelled)
    }

    /// A job that could not run at all.
    pub(super) fn failed(error: IpcError) -> Self {
        Self::of(Outcome::Failed { error })
    }

    fn of(outcome: Outcome) -> Self {
        Self {
            outcome,
            skipped: 0,
            errors: Vec::new(),
            more_errors: 0,
        }
    }
}

/// Why a step stopped: the job was cancelled, or this step failed. A failed item is reported and the job goes
/// on; a failure before the items (a path that does not resolve) fails the job.
#[derive(Debug)]
pub(super) enum Stop {
    Cancelled,
    Failed(io::Error),
}

impl From<Cancelled> for Stop {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

impl From<io::Error> for Stop {
    fn from(error: io::Error) -> Self {
        Self::Failed(error)
    }
}

/// The folders this job created, removed again (when still empty, newest first) unless the job finished.
#[derive(Default)]
pub(super) struct Created {
    folders: Vec<PathBuf>,
    keep: bool,
}

impl Created {
    pub(super) fn push(&mut self, folder: PathBuf) {
        self.folders.push(folder);
    }

    /// `folder` is complete: it stays, whatever happens to the job from here.
    pub(super) fn finish(&mut self, folder: &Path) {
        if let Some(index) = self.folders.iter().rposition(|made| made == folder) {
            self.folders.remove(index);
        }
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        if self.keep {
            return;
        }

        // `remove_dir` only removes an empty folder: what a finished copy put there stays.
        for folder in self.folders.iter().rev() {
            let _ = fs::remove_dir(folder);
        }
    }
}

/// The state of one job on its worker. The steps are methods spread over the sibling modules (`plan`, `copy`,
/// `conflict`, `verify`, `progress`).
pub(super) struct Run<'a> {
    pub(super) inner: &'a Inner,
    pub(super) job: &'a Job,
    pub(super) progress: Progress,
    /// The answer kept for every conflict that is left ("apply to all").
    pub(super) policy: Option<Answer>,
    /// What the item of a synchronization that is being written may do at its target (see `sync_terms`).
    pub(super) terms: Option<Terms>,
    pub(super) skipped: u32,
    errors: Vec<ItemError>,
    more_errors: u32,
    pub(super) created: Created,
    /// Reused between files: the buffer of the chunked copy.
    pub(super) buffer: Vec<u8>,
    /// Folders walked so far (by device and inode), so a folder reachable twice is copied once.
    pub(super) visited: HashSet<FileId>,
    /// The target folder, which the walk never enters.
    pub(super) target_id: Option<FileId>,
    /// Numbers the temporary files of this job.
    pub(super) sequence: u32,
}

/// Runs `job` to its end and reports what became of it. Everything it created and did not finish is removed
/// before it returns.
pub(super) fn execute(inner: &Inner, job: &Job) -> Finish {
    let mut run = Run {
        inner,
        job,
        progress: Progress::new(),
        policy: None,
        terms: None,
        skipped: 0,
        errors: Vec::new(),
        more_errors: 0,
        created: Created::default(),
        buffer: Vec::new(),
        visited: HashSet::new(),
        target_id: None,
        sequence: 0,
    };

    assert!(!inner.hooks.panic_on_start, "the test asked this job to panic");

    let result = match &job.request.task {
        Task::Transfer => run.run(),
        task => run.run_task(task),
    };

    let outcome = match result {
        Ok(()) => {
            run.created.keep = true;
            run.tick(true);
            Outcome::Done
        }
        Err(Stop::Cancelled) => {
            job.control.close();
            run.flush_renamed();
            Outcome::Cancelled
        }
        Err(Stop::Failed(error)) => {
            run.flush_renamed();
            Outcome::Failed {
                error: report::ipc_error(&error),
            }
        }
    };

    Finish {
        outcome,
        skipped: run.skipped,
        errors: std::mem::take(&mut run.errors),
        more_errors: run.more_errors,
    }
}

impl Run<'_> {
    /// A multi-rename that ends early leaves its view with the exact count of the items it went through (progress is
    /// throttled), which the page reads to tell whether anything was renamed and can be undone.
    fn flush_renamed(&mut self) {
        if matches!(self.job.request.task, Task::Rename { .. }) {
            self.tick(true);
        }
    }

    pub(super) fn moving(&self) -> bool {
        self.job.request.kind == OpKind::Move
    }

    fn run(&mut self) -> Result<(), Stop> {
        self.checkpoint()?;
        self.phase(JobPhase::Preparing);
        let (to, mut items) = self.prepare()?;

        if self.moving() {
            self.phase(JobPhase::Renaming);
            items = self.rename_items(&to, items)?;
        }

        if items.is_empty() {
            return Ok(());
        }

        self.phase(JobPhase::Scanning);
        self.scan(&items)?;

        self.phase(JobPhase::Copying);
        self.copy_items(&to, &items)
    }

    /// Moves each top-level item by renaming it, which copies no bytes. What cannot be renamed (the name is
    /// taken, or the target is on another disk) is returned for the copy; any other error fails the item.
    fn rename_items(&mut self, to: &Path, items: Vec<Child>) -> Result<Vec<Child>, Stop> {
        let mut left = Vec::new();

        for item in items {
            self.checkpoint()?;

            match self.rename_noreplace(&item.path, &to.join(&item.name)) {
                Ok(()) => self.renamed(),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists || rename::crosses_devices(&error) => {
                    left.push(item);
                }
                Err(error) => self.fail(&item.rel(), &error),
            }
        }

        Ok(left)
    }

    /// [`rename::noreplace`], which fails like a rename between two disks when a test asks for that.
    pub(super) fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.refuse_across_disks()?;

        rename::noreplace(from, to)
    }

    /// [`rename::replace`], which fails like a rename between two disks when a test asks for that.
    pub(super) fn rename_replace(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.refuse_across_disks()?;

        rename::replace(from, to)
    }

    fn refuse_across_disks(&self) -> io::Result<()> {
        if self.inner.hooks.cross_device {
            return Err(io::Error::from(io::ErrorKind::CrossesDevices));
        }

        Ok(())
    }

    /// Copies the items into `to`, one after the other. A failed item is reported and the next one goes on.
    fn copy_items(&mut self, to: &Path, items: &[Child]) -> Result<(), Stop> {
        self.visited.clear();

        for item in items {
            self.checkpoint()?;
            let rel = item.rel();

            match self.copy_node(to, item, &rel, 0) {
                Ok(_) => {}
                Err(Stop::Failed(error)) => self.fail(&rel, &error),
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
            }
        }

        Ok(())
    }

    /// How many items failed so far.
    pub(super) fn failures(&self) -> usize {
        self.errors.len().saturating_add(self.more_errors as usize)
    }

    /// Records that the item at `rel` failed; the job goes on.
    pub(super) fn fail(&mut self, rel: &str, error: &io::Error) {
        if self.errors.len() >= MAX_REPORTED_ERRORS {
            self.more_errors = self.more_errors.saturating_add(1);
            return;
        }

        self.errors.push(ItemError {
            path: rel.to_owned(),
            error: report::ipc_error(error),
        });
    }
}

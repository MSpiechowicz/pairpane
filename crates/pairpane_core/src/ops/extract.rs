//! Extracting an archive as a job: the archive is a file of the job's source folder, and the job writes the chosen
//! entries (folders with all below them) into the target folder.
//!
//! The archive is opened without following a link and, when the caller brings the index it showed, checked to be the file
//! that index was made from; without one the index is built here. What is written, and how, is `extract_write`. A name
//! that the archive code refuses (see [`crate::archive::normalize`]), a link and a special entry are never written; they
//! are counted as skipped. An entry that fails (a method that is not handled, a wrong checksum, more data than it
//! declares) is reported and the next one goes on; a password that is missing or wrong, or a disk that has no room left,
//! ends the job. A cancel also stops the reading of the archive itself (a big compressed tar is decoded up to the entry
//! that is wanted), through the archive code's own cancel flag.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::archive::{self, ArchiveIndex, Fingerprint, Flow, HEAD_BYTES, IndexEntry, Input, PasswordFault, Selected};
use crate::model::JobPhase;

use super::control::Control;
use super::extract_write::Landing;
use super::open;
use super::origin::Origin;
use super::plan::{Child, Kind};
use super::report::refusal;
use super::transfer::{Run, Stop};

/// The archive `name` of the folder `folder` (which the caller knows as `shown`), opened without following a link, and its
/// state.
pub(super) fn open_archive(folder: &Path, shown: &Path, name: &str) -> Result<(Child, File, Fingerprint), Stop> {
    let found = Child::lookup(folder, name)?;
    if found.kind != Kind::File {
        return Err(refusal(io::ErrorKind::InvalidInput, "the archive is not a regular file").into());
    }

    let file = open::source(&found.path, &found.meta)?;
    let print = Fingerprint::of(&shown.join(name), &file.metadata()?);

    Ok((found, file, print))
}

/// The error of an archive that is not the file the caller's index was made from.
pub(super) fn changed() -> io::Error {
    refusal(io::ErrorKind::Other, "the archive changed; open it again")
}

/// How often the watcher looks whether the job was cancelled.
const WATCH_EVERY: Duration = Duration::from_millis(50);

/// What an extraction knows about its archive, brought by the caller: the index it listed (so the archive is not read
/// again to find the entries), the state of the file that index was made from, and the password the user gave.
///
/// Equal when they hold the same index and password (by identity), so a [`super::Request`] can be compared. `Debug` says
/// only whether each is there.
#[derive(Clone, Default)]
pub struct ArchiveJob {
    pub index: Option<Arc<ArchiveIndex>>,
    /// The state of the archive file when `index` was made, taken for the path `from` joined with the archive's name
    /// (see [`Fingerprint::of`]). An index without it is not trusted: the job asks to open the archive again.
    pub fingerprint: Option<Fingerprint>,
    pub password: Option<Arc<archive::Password>>,
}

impl PartialEq for ArchiveJob {
    fn eq(&self, other: &Self) -> bool {
        same(self.index.as_ref(), other.index.as_ref())
            && same(self.password.as_ref(), other.password.as_ref())
            && self.fingerprint == other.fingerprint
    }
}

impl Eq for ArchiveJob {}

fn same<T>(a: Option<&Arc<T>>, b: Option<&Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

impl fmt::Debug for ArchiveJob {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("ArchiveJob")
            .field("index", &self.index.is_some())
            .field("fingerprint", &self.fingerprint.is_some())
            .field("password", &self.password.is_some())
            .finish()
    }
}

impl Run<'_> {
    /// Runs the extraction of the job (the archive is the request's one name). See the module's text.
    pub(super) fn extract(&mut self, inner: &str, entries: &[String], source: &ArchiveJob) -> Result<(), Stop> {
        let job = self.job;
        let request = &job.request;
        let name = request
            .names
            .first()
            .ok_or_else(|| refusal(io::ErrorKind::InvalidInput, "no archive"))?;

        let from = self.open_folders(&[&request.from, &request.to])?;
        let (found, file, print) = open_archive(&from, &request.from, name)?;
        let target = self.target_folder(&request.to)?;
        let cancel = AtomicBool::new(false);
        // Read once: every file written gets the same marker (see `origin`).
        let origin = Origin::of(&file, &found.path);

        let input = Input {
            file: &file,
            path: Some(&found.path),
        };
        let password = source.password.as_deref();

        watched(&job.control, &cancel, || {
            let index = self.indexed(name, source, &print, &input, &cancel)?;

            let selection = index
                .select(inner, entries)
                .ok_or_else(|| refusal(io::ErrorKind::NotFound, "the folder or item is not in the archive"))?;
            self.skipped = self
                .skipped
                .saturating_add(selection.skipped)
                .saturating_add(selection.refused);

            if password.is_none() && selection.any_encrypted() {
                return Err(archive::password_error(PasswordFault::Needed).into());
            }

            for item in &selection.items {
                self.checkpoint()?;

                if item.entry.kind == archive::EntryType::File {
                    self.found(item.entry.size);
                }
            }

            self.phase(JobPhase::Extracting);
            let mut reader: Box<dyn archive::EntrySource + '_> = match &self.inner.hooks.entry_source {
                Some(make) => make(&index),
                None => archive::open_source(&index, &input, &cancel, password).map_err(|e| self.stopped(e))?,
            };

            self.write_all(&mut *reader, &selection.items, &selection.ordinals(), target, origin)
        })
    }

    /// The index of the archive `name` that the job works on: the one the caller brought when the archive is still in the
    /// state it was in (`print` is its state now), none when it is not (the caller must open it again), and, when the caller
    /// brought none, one built here.
    pub(super) fn indexed(
        &mut self,
        name: &str,
        source: &ArchiveJob,
        print: &Fingerprint,
        input: &Input<'_>,
        cancel: &AtomicBool,
    ) -> Result<Arc<ArchiveIndex>, Stop> {
        match &source.index {
            Some(index) if source.fingerprint.as_ref() == Some(print) => Ok(Arc::clone(index)),
            Some(_) => Err(changed().into()),
            None => {
                self.phase(JobPhase::Scanning);
                let format = archive::detect(name, &input.file.read_chunk(0, HEAD_BYTES)?)
                    .ok_or_else(|| refusal(io::ErrorKind::InvalidInput, "this is not an archive that can be opened"))?;
                let password = source.password.as_deref();

                Ok(Arc::new(
                    archive::build_index(format, input, cancel, password).map_err(|e| self.stopped(e))?,
                ))
            }
        }
    }

    /// Hands the entries `items` (ordinals `wanted`) from `reader` to the writing, into `target`.
    fn write_all(
        &mut self,
        reader: &mut dyn archive::EntrySource,
        items: &[Selected<'_>],
        wanted: &[u32],
        target: std::path::PathBuf,
        origin: Origin,
    ) -> Result<(), Stop> {
        if wanted.is_empty() {
            return Ok(());
        }

        let mut landing = Landing::new(target, origin);
        let mut stopped = None;

        let walked = reader.extract(wanted, &mut |entry: &IndexEntry, data: &mut dyn Read| {
            let Ok(at) = items.binary_search_by_key(&entry.ordinal, |item| item.entry.ordinal) else {
                return Ok(Flow::Continue);
            };
            let rel = items[at].relative;

            match self.write_item(&mut landing, entry, rel, data) {
                Ok(()) => Ok(Flow::Continue),
                Err(Stop::Failed(error)) if !self.ends_job(&error) => {
                    self.fail(rel, &error);

                    Ok(Flow::Continue)
                }
                Err(stop) => {
                    stopped = Some(stop);

                    Ok(Flow::Stop)
                }
            }
        });

        if let Some(stop) = stopped {
            return Err(self.settled(stop));
        }
        walked.map_err(|error| self.stopped(error))?;

        self.finish_folders(landing);

        Ok(())
    }

    /// Whether `error` of one entry ends the job: a cancel (the archive code answers the cancel flag with an error), a
    /// password that is missing or wrong, or no room left on the disk.
    fn ends_job(&self, error: &io::Error) -> bool {
        self.job.control.is_cancelled()
            || archive::password_fault(error).is_some()
            || error.kind() == io::ErrorKind::StorageFull
    }

    /// `stop`, which is a cancel when the job was cancelled (the error that ended the entry was the archive code's reply
    /// to the cancel flag).
    fn settled(&self, stop: Stop) -> Stop {
        match stop {
            Stop::Failed(error) => self.stopped(error),
            cancelled => cancelled,
        }
    }

    /// The stop an `error` of the archive code comes to: a cancel when the job was cancelled, else the error.
    pub(super) fn stopped(&self, error: io::Error) -> Stop {
        if self.job.control.is_cancelled() {
            Stop::Cancelled
        } else {
            Stop::Failed(error)
        }
    }
}

/// Runs `work` while a watcher raises `cancel` as soon as the job is cancelled, so the reading of the archive (which
/// checkpoints do not reach) stops too.
pub(super) fn watched<T>(control: &Control, cancel: &AtomicBool, work: impl FnOnce() -> T) -> T {
    let finished = AtomicBool::new(false);

    thread::scope(|scope| {
        let watcher = scope.spawn(|| {
            while !finished.load(Ordering::Acquire) {
                if control.is_cancelled() {
                    cancel.store(true, Ordering::Relaxed);

                    return;
                }

                thread::park_timeout(WATCH_EVERY);
            }
        });

        let result = work();

        finished.store(true, Ordering::Release);
        watcher.thread().unpark();

        result
    })
}

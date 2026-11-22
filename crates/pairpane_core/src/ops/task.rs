//! The file tools as jobs: how each starts, and what they share (the folders, the copy loop, the no-replace commit).
//!
//! A tool job looks at the paths like a copy does, then hands over to its own module: `checksum` and
//! `checksum_verify`, `split`, `combine`, `attrs`. What a tool creates is a temporary file first (see `partial`),
//! put in place with a rename that refuses to replace, so a name that is taken is never overwritten.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::model::JobPhase;

use super::plan::Child;
use super::report::refusal;
use super::transfer::{Run, Stop};
use super::volume;
use super::{Task, name_fits};

/// Bytes moved between two checkpoints.
const CHUNK: usize = 1024 * 1024;

/// What a tool job works on: the folder the items are in, the folder it writes into (the same one for a tool that
/// works in place), and the items that are there.
pub(super) struct Scene {
    pub(super) from: PathBuf,
    pub(super) to: PathBuf,
    pub(super) items: Vec<Child>,
}

impl Run<'_> {
    /// Runs the tool of the job's [`Task`] (a transfer is not one: it has its own `run`).
    pub(super) fn run_task(&mut self, task: &Task) -> Result<(), Stop> {
        self.checkpoint()?;
        self.phase(JobPhase::Preparing);

        if let Task::Extract { inner, entries, source } = task {
            return self.extract(inner, entries, source);
        }

        let scene = self.prepare_task(task)?;

        match task {
            Task::Checksum { algorithm, output } => self.write_checksums(&scene, *algorithm, output),
            Task::VerifyChecksum => self.verify_checksums(&scene),
            Task::Split { part_size } => self.split_file(&scene, *part_size),
            Task::Combine => self.combine_parts(&scene),
            Task::Attributes(change) => self.change_attributes(&scene, change),
            Task::Transfer | Task::Extract { .. } => {
                Err(refusal(io::ErrorKind::InvalidInput, "this job cannot start with these items").into())
            }
        }
    }

    /// Checks the paths, looks up the items (one that is gone is reported and left out) and, for a tool that writes
    /// into another folder, makes that folder when it is missing.
    fn prepare_task(&mut self, task: &Task) -> Result<Scene, Stop> {
        let request = &self.job.request;
        let elsewhere = matches!(task, Task::Split { .. } | Task::Combine);

        let mut folders = vec![request.from.as_path()];
        if elsewhere {
            folders.push(&request.to);
        }
        // A checksum file and an attribute change look their items up by handle, in the folder they hold open.
        let (from, items) = if matches!(task, Task::Checksum { .. } | Task::Attributes(_)) {
            (self.open_folders(&folders)?, Vec::new())
        } else {
            self.open_items(&folders)?
        };

        let to = if elsewhere {
            self.target_folder(&request.to)?
        } else {
            from.clone()
        };

        Ok(Scene { from, to, items })
    }

    /// The folder `target`, made (with the folders above it) when it is missing.
    pub(super) fn target_folder(&mut self, target: &Path) -> Result<PathBuf, Stop> {
        let (existing, missing) = volume::existing_part(target)?
            .ok_or_else(|| refusal(io::ErrorKind::NotFound, "the target folder cannot be reached"))?;
        let to = existing.join(&missing);

        self.create_folders(&existing, &missing)?;
        if !fs::metadata(&to)?.is_dir() {
            return Err(refusal(io::ErrorKind::InvalidInput, "the target is not a folder").into());
        }

        Ok(to)
    }

    /// Moves up to `limit` bytes from `from` to `to`, counting them in `crc` and in the job's progress, and stopping
    /// between chunks for a pause or a cancel. Returns how many bytes there were: fewer than `limit` at the end of
    /// `from`.
    pub(super) fn pump(
        &mut self,
        from: &mut (impl Read + ?Sized),
        to: &mut File,
        limit: u64,
        crc: &mut crc32fast::Hasher,
    ) -> Result<u64, Stop> {
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.resize(CHUNK, 0);
        let mut done = 0_u64;

        while done < limit {
            self.checkpoint()?;

            let want = usize::try_from(limit - done).map_or(buffer.len(), |left| left.min(buffer.len()));
            let read = match from.read(&mut buffer[..want]) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };

            crc.update(&buffer[..read]);
            to.write_all(&buffer[..read])?;
            done += read as u64;
            self.add_bytes(read as u64);
        }

        self.buffer = buffer;

        Ok(done)
    }

    /// Puts the finished temporary file `temp` at `target`, which must not exist (a rename that refuses to replace).
    pub(super) fn commit_new(&self, temp: &Path, target: &Path) -> Result<(), Stop> {
        self.rename_noreplace(temp, target).map_err(Stop::from)
    }
}

/// Creates the new file `path` for writing, with the usual permissions of a new file; fails when anything is there
/// (a link too, which is never followed).
pub(super) fn create_plain(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Fails with "already exists" when anything is at `path`, a link that leads nowhere included.
pub(super) fn require_free(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// `name` as text, which a name written into a file or built from another one must be.
pub(super) fn as_text(name: &std::ffi::OsStr) -> io::Result<&str> {
    name.to_str()
        .ok_or_else(|| refusal(io::ErrorKind::InvalidInput, "the name is not valid text"))
}

/// A file name made of `base` and a `suffix`, which must still fit the limit of a name.
pub(super) fn derived_name(base: &str, suffix: &str) -> io::Result<String> {
    let name = format!("{base}{suffix}");

    if name_fits(&name) {
        Ok(name)
    } else {
        Err(refusal(io::ErrorKind::InvalidInput, "the file name would be too long"))
    }
}

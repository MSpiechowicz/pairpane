//! A target that exists already: what to do about it, and asking the user.
//!
//! The first conflict of a job sends a question and waits, without a timeout, for the answer (a cancel always
//! ends the wait). An answer given "for all" is kept and decides every later conflict without asking.

use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use crate::model::{Answer, ConflictInfo, FileFacts, JobPhase, OpsEvent};

use super::MAX_NAME_UNITS;
use super::identity::same_place;
use super::report::refusal;
use super::transfer::{Run, Stop};
use super::validate::{char_units, name_units};

/// Most names tried for `rename` before giving up.
const MAX_RENAMES: u32 = 10_000;

/// Where a file or link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Place {
    /// The name is free: the system refuses to replace anything that shows up there in the meantime.
    New(PathBuf),
    /// Replace what is there.
    Replace(PathBuf),
    /// Leave the item alone.
    Skip,
}

impl Place {
    /// The target path; empty for `Skip`.
    pub(super) fn path(&self) -> &Path {
        match self {
            Self::New(path) | Self::Replace(path) => path,
            Self::Skip => Path::new(""),
        }
    }
}

/// What the dialog shows about one side.
fn facts(meta: &Metadata) -> FileFacts {
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| match time.duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_millis()).ok(),
            Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
        });

    FileFacts {
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified,
        dir: meta.is_dir(),
    }
}

/// The first free name beside `target`: `a (2).txt`, then `a (3).txt`, and so on. A stem that would make the name
/// too long for the filesystem is cut short, never in the middle of a char.
pub(super) fn free_name(target: &Path) -> io::Result<PathBuf> {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(refusal(io::ErrorKind::InvalidInput, "the target has no name"));
    };
    let name = Path::new(name);
    let stem = name.file_stem().unwrap_or(name.as_os_str());

    for number in 2..=MAX_RENAMES {
        let mut ending = format!(" ({number})");
        if let Some(extension) = name.extension() {
            ending.push('.');
            ending.push_str(&extension.to_string_lossy());
        }

        let Some(room) = MAX_NAME_UNITS.checked_sub(name_units(&ending)) else {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "the name has no room for a number",
            ));
        };
        let mut candidate = shorten(stem, room);
        candidate.push(format!(" ({number})"));
        if let Some(extension) = name.extension() {
            candidate.push(".");
            candidate.push(extension);
        }

        let path = parent.join(candidate);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(path),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
    }

    Err(refusal(io::ErrorKind::AlreadyExists, "no free name was found"))
}

/// `stem`, cut to at most `room` units of a file name (see [`name_units`]) by whole chars. One that is not text is
/// cut as its lossy text, which only happens when it is that long.
fn shorten(stem: &OsStr, room: usize) -> OsString {
    let text = stem.to_string_lossy();

    if name_units(&text) <= room {
        return stem.to_owned();
    }

    let mut used = 0;
    let kept: String = text
        .chars()
        .take_while(|char| {
            used += char_units(*char);
            used <= room
        })
        .collect();

    kept.into()
}

impl Run<'_> {
    /// Whether two paths are one file, by the identity the filesystem gives them. One that cannot be read counts as
    /// the same file, so nothing is replaced on a guess.
    pub(super) fn same_file(&self, a: &Metadata, a_path: &Path, b: &Metadata, b_path: &Path) -> bool {
        if self.inner.hooks.blind_identity {
            return false;
        }

        same_place(self.id_of_meta(a_path, a), self.id_of_meta(b_path, b))
    }

    /// Decides where the file or link `rel` (read as `source`, at `source_path`) goes, for the wanted
    /// `target`: there when the name is free, otherwise as the answer says.
    ///
    /// # Errors
    ///
    /// A folder at the target: a file cannot replace it, and nothing is asked.
    pub(super) fn place(
        &mut self,
        rel: &str,
        source: &Metadata,
        source_path: &Path,
        target: &Path,
    ) -> Result<Place, Stop> {
        let existing = match fs::symlink_metadata(target) {
            Ok(existing) => existing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Place::New(target.to_owned())),
            Err(error) => return Err(error.into()),
        };

        if existing.is_dir() {
            return Err(refusal(io::ErrorKind::AlreadyExists, "a folder has the name of this file").into());
        }

        let answer = match self.policy {
            Some(answer) => answer,
            None => self.ask(rel, source, source_path, &existing, target)?,
        };

        let replace = match answer {
            Answer::Overwrite => true,
            Answer::OverwriteOlder => {
                matches!((existing.modified(), source.modified()), (Ok(old), Ok(new)) if old < new)
            }
            Answer::Rename => return Ok(Place::New(free_name(target)?)),
            Answer::Skip | Answer::Compare => false,
        };

        // Replacing a file with itself would lose it.
        if !replace || self.same_file(source, source_path, &existing, target) {
            return Ok(Place::Skip);
        }

        Ok(Place::Replace(target.to_owned()))
    }

    /// Sends the question and waits for its final answer. `compare` is answered on the way, and the question
    /// stays open.
    fn ask(
        &mut self,
        rel: &str,
        source: &Metadata,
        source_path: &Path,
        existing: &Metadata,
        target: &Path,
    ) -> Result<Answer, Stop> {
        let suggested = free_name(target)?
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let question = self.job.control.open(ConflictInfo {
            job: self.job.id,
            question: 0,
            name: rel.to_owned(),
            source: facts(source),
            target: facts(existing),
            suggested,
            comparable: true,
        });

        self.phase(JobPhase::Waiting);
        self.inner.emit(OpsEvent::Conflict {
            conflict: question.clone(),
        });

        loop {
            let (reply, waited) = match self.job.control.wait_reply() {
                Ok(reply) => reply,
                Err(cancelled) => {
                    self.job.control.close();

                    return Err(cancelled.into());
                }
            };
            self.exclude(waited);

            if reply.answer != Answer::Compare {
                if reply.all {
                    self.policy = Some(reply.answer);
                }
                self.phase(JobPhase::Copying);

                return Ok(reply.answer);
            }

            let started = Instant::now();
            let identical = self.same_content(source_path, source, target)?;
            self.exclude(started.elapsed());

            self.inner.emit(OpsEvent::Compared {
                id: self.job.id,
                question: question.question,
                identical,
            });
        }
    }
}

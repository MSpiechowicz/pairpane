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
use super::Task;
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

/// What a new file's entry comes to when an entry of the archive has its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decision {
    /// The entry that is there is left out and the new one written.
    Replace,
    /// The new file is not added.
    Skip,
    /// The new file is added under this path instead.
    Rename(String),
}

/// What is put at the target: a file or a link on the disk, or an entry of an archive (which only has its facts: it
/// cannot be hashed for a compare, and is never the same file as the target).
pub(super) enum Origin<'a> {
    Disk { meta: &'a Metadata, path: &'a Path },
    Entry(&'a FileFacts),
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

/// Whether the existing `target` is older than what replaces it. Without both times it is not.
fn older(existing: &Metadata, source: &Origin<'_>) -> bool {
    match source {
        Origin::Disk { meta, .. } => matches!((existing.modified(), meta.modified()), (Ok(old), Ok(new)) if old < new),
        Origin::Entry(entry) => replaces_older(&facts(existing), entry),
    }
}

/// Whether the entry `existing` is older than `source`, which would replace it. Without both times it is not.
pub(super) fn replaces_older(existing: &FileFacts, source: &FileFacts) -> bool {
    matches!((existing.modified, source.modified), (Some(old), Some(new)) if old < new)
}

/// The endings of archive names that are made of two extensions: the number goes before them (`a (2).tar.zst`), not
/// between them. The other archive names (`.zip`, `.7z`, `.tgz`) have the one extension that is told by the rule for any
/// name.
const COMPOUND_ENDINGS: [&str; 4] = [".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2"];

/// `name` as its stem and its extension: the last extension, or the two of a compound archive ending (any case).
fn split_name(name: &OsStr) -> (&OsStr, Option<&OsStr>) {
    if let Some(text) = name.to_str() {
        for ending in COMPOUND_ENDINGS {
            let cut = text.len().saturating_sub(ending.len());

            if cut > 0 && text.get(cut..).is_some_and(|tail| tail.eq_ignore_ascii_case(ending)) {
                return (OsStr::new(&text[..cut]), Some(OsStr::new(&text[cut + 1..])));
            }
        }
    }

    let path = Path::new(name);

    (path.file_stem().unwrap_or(name), path.extension())
}

/// The first free name beside `target`: `a (2).txt`, then `a (3).txt`, and so on (`a (2).tar.zst` for an archive with
/// two extensions). A stem that would make the name too long for the filesystem is cut short, never in the middle of a
/// char.
pub(super) fn free_name(target: &Path) -> io::Result<PathBuf> {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(refusal(io::ErrorKind::InvalidInput, "the target has no name"));
    };

    let free = numbered_name(name, |candidate| match fs::symlink_metadata(parent.join(candidate)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
        Ok(_) => Ok(true),
    })?;

    Ok(parent.join(free))
}

/// [`free_name`] for the path `path` of an entry of an archive (`/` separated): the number goes in its last part, and
/// `taken` says whether a path is in use.
pub(super) fn free_entry_name(path: &str, taken: impl Fn(&str) -> bool) -> io::Result<String> {
    let (folder, name) = path
        .rsplit_once('/')
        .map_or(("", path), |(folder, name)| (folder, name));

    let free = numbered_name(OsStr::new(name), |candidate| {
        let candidate = candidate.to_string_lossy();

        Ok(taken(&if folder.is_empty() {
            candidate.into_owned()
        } else {
            format!("{folder}/{candidate}")
        }))
    })?;
    let free = free.to_string_lossy();

    Ok(if folder.is_empty() {
        free.into_owned()
    } else {
        format!("{folder}/{free}")
    })
}

/// The first of `name (2)`, `name (3)`, ... (the number before the extension) that `taken` says is not.
fn numbered_name(name: &OsStr, mut taken: impl FnMut(&OsStr) -> io::Result<bool>) -> io::Result<OsString> {
    let (stem, extension) = split_name(name);

    for number in 2..=MAX_RENAMES {
        let mut ending = format!(" ({number})");
        if let Some(extension) = extension {
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
        if let Some(extension) = extension {
            candidate.push(".");
            candidate.push(extension);
        }

        if !taken(&candidate)? {
            return Ok(candidate);
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
        self.place_origin(
            rel,
            &Origin::Disk {
                meta: source,
                path: source_path,
            },
            target,
        )
    }

    /// Like [`Run::place`] for an entry of an archive that has the `source` facts: the question it may ask has no
    /// compare, and an older target is told from it by the entry's time.
    pub(super) fn place_entry(&mut self, rel: &str, source: &FileFacts, target: &Path) -> Result<Place, Stop> {
        self.place_origin(rel, &Origin::Entry(source), target)
    }

    fn place_origin(&mut self, rel: &str, source: &Origin<'_>, target: &Path) -> Result<Place, Stop> {
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
            None => {
                let suggested = free_name(target)?
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();

                self.ask(rel, source, &facts(&existing), (Some(target), suggested))?
            }
        };

        let replace = match answer {
            Answer::Overwrite => true,
            Answer::OverwriteOlder => older(&existing, source),
            Answer::Rename => return Ok(Place::New(free_name(target)?)),
            Answer::Skip | Answer::Compare => false,
        };

        // Replacing a file with itself would lose it.
        let itself = matches!(source, Origin::Disk { meta, path } if self.same_file(meta, path, &existing, target));

        if !replace || itself {
            return Ok(Place::Skip);
        }

        Ok(Place::Replace(target.to_owned()))
    }

    /// What to do about the entry `rel` of an archive that a new file would replace: `existing` is the facts of the entry
    /// that is there, `source` those of the new file, and `free` makes the name that is free to rename the new file to.
    /// Asks the user unless an answer was given for all.
    pub(super) fn decide_entry(
        &mut self,
        rel: &str,
        source: &FileFacts,
        existing: &FileFacts,
        free: impl Fn() -> io::Result<String>,
    ) -> Result<Decision, Stop> {
        let answer = match self.policy {
            Some(answer) => answer,
            None => self.ask(rel, &Origin::Entry(source), existing, (None, free()?))?,
        };

        Ok(match answer {
            Answer::Overwrite => Decision::Replace,
            Answer::OverwriteOlder if replaces_older(existing, source) => Decision::Replace,
            Answer::Rename => Decision::Rename(free()?),
            _ => Decision::Skip,
        })
    }

    /// The phase the job is in while it works, which it returns to after a question.
    fn working_phase(&self) -> JobPhase {
        match self.job.request.task {
            Task::Extract { .. } => JobPhase::Extracting,
            Task::Pack { .. } | Task::AddToArchive { .. } => JobPhase::Packing,
            _ => JobPhase::Copying,
        }
    }

    /// Sends the question and waits for its final answer. `compare` is answered on the way, and the question
    /// stays open.
    fn ask(
        &mut self,
        rel: &str,
        source: &Origin<'_>,
        existing: &FileFacts,
        (target, suggested): (Option<&Path>, String),
    ) -> Result<Answer, Stop> {
        let question = self.job.control.open(ConflictInfo {
            job: self.job.id,
            question: 0,
            name: rel.to_owned(),
            source: match source {
                Origin::Disk { meta, .. } => facts(meta),
                Origin::Entry(entry) => (*entry).clone(),
            },
            target: existing.clone(),
            suggested,
            comparable: matches!(source, Origin::Disk { .. }),
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
                self.phase(self.working_phase());

                return Ok(reply.answer);
            }

            // A source that is not a file on the disk offers no compare: a stale one is not answered.
            let (Origin::Disk { meta, path }, Some(target)) = (source, target) else {
                continue;
            };

            let started = Instant::now();
            let identical = self.same_content(path, meta, target)?;
            self.exclude(started.elapsed());

            self.inner.emit(OpsEvent::Compared {
                id: self.job.id,
                question: question.question,
                identical,
            });
        }
    }
}

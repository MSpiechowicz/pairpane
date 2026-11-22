//! Synchronizing as a job: the items the page chose from a comparison of two folders are copied, each the way its plan says.
//!
//! The scan that made the plans is over, so the job trusts none of it. Each item is looked at again right before it is
//! copied: the source must still be what the scan saw (its kind, and for a file its size and its time to the
//! millisecond), and so must the target (the same file, or still not there; a folder that is copied whole must still not
//! exist at the target). An item that fails the look is skipped and counted, never copied over a change the user did not
//! see. Nothing is ever deleted, a link is never followed (one on the way to an item is a refusal, and a link inside a
//! folder that is copied whole is copied as a link, as a copy does), and a special file is skipped.
//!
//! Every folder on the way to an item is entered one name at a time with no link followed, in the source and in the target,
//! and the folders missing in the target are made. A file replaces what is at its target (the way every copy does: a
//! temporary file, then a rename, with the time kept), because the page asked for it with the confirmation it showed.
//! Items from an archive are in `sync_archive`.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use crate::listing::{modified_millis, platform};
use crate::model::{Answer, EntryKind, JobPhase, SyncDirection, SyncFacts};

use super::extract::ArchiveJob;
use super::plan::{Child, is_inside, real};
use super::report::refusal;
use super::transfer::{Run, Stop};
use super::{SyncPlan, SyncRoot};

/// An archive that items are copied from, and where it is.
pub(super) struct ArchiveSide<'a> {
    /// The folder that holds the archive file (resolved).
    pub(super) folder: PathBuf,
    pub(super) name: &'a str,
    pub(super) inner: &'a str,
    pub(super) source: &'a ArchiveJob,
}

/// One side of the job, as the worker resolved it.
enum Side<'a> {
    Disk(PathBuf),
    Archive(ArchiveSide<'a>),
}

impl<'a> Side<'a> {
    /// The side `root` is, held in `folder` (the folder of the archive file for an archive, the folder itself otherwise).
    fn of(root: &'a SyncRoot, folder: PathBuf) -> Self {
        match root {
            SyncRoot::Folder => Self::Disk(folder),
            SyncRoot::Archive { name, inner, source } => Self::Archive(ArchiveSide {
                folder,
                name,
                inner,
                source,
            }),
        }
    }
}

impl Run<'_> {
    /// Runs the synchronization of the job: see the module's text.
    pub(super) fn sync(&mut self, left: &SyncRoot, right: &SyncRoot, plans: &[SyncPlan]) -> Result<(), Stop> {
        let job = self.job;
        let request = &job.request;

        let from = self.root_folder(&request.from)?;
        let to = self.root_folder(&request.to)?;
        let (left, right) = match (left, right) {
            (_, SyncRoot::Folder) => (Side::of(left, from), Side::of(right, to)),
            (SyncRoot::Folder, SyncRoot::Archive { .. }) => (Side::of(left, to), Side::of(right, from)),
            (SyncRoot::Archive { .. }, SyncRoot::Archive { .. }) => return Err(unsupported().into()),
        };
        if let (Side::Disk(one), Side::Disk(other)) = (&left, &right) {
            refuse_nested(one, other)?;
        }

        // The page confirmed the overwrites: a file that is at its target (and still the file the scan saw) is replaced.
        self.policy = Some(Answer::Overwrite);

        let items: Vec<(&str, &SyncPlan, &Side<'_>, &Path)> = request
            .names
            .iter()
            .zip(plans)
            .filter_map(|(rel, plan)| {
                let (source, target) = match plan.direction {
                    SyncDirection::ToRight => (&left, &right),
                    SyncDirection::ToLeft => (&right, &left),
                };
                let Side::Disk(target) = target else { return None };

                Some((rel.as_str(), plan, source, target.as_path()))
            })
            .collect();

        self.phase(JobPhase::Scanning);
        for (rel, plan, source, _) in &items {
            self.checkpoint()?;

            if let Side::Disk(root) = source {
                self.count_item(root, rel, plan)?;
            }
        }

        self.phase(JobPhase::Copying);
        let mut from_archive = Vec::new();
        for (rel, plan, source, target) in items {
            self.checkpoint()?;

            match source {
                Side::Disk(root) => {
                    let done = self.sync_item(root, target, rel, plan);
                    self.settle(rel, done)?;
                }
                Side::Archive(archive) => from_archive.push((archive, target, rel, plan)),
            }
        }

        if let Some((archive, target, ..)) = from_archive.first() {
            let rows = from_archive
                .iter()
                .map(|(_, _, rel, plan)| (*rel, *plan))
                .collect::<Vec<_>>();

            self.sync_from_archive(archive, target, &rows)?;
        }

        Ok(())
    }

    /// The folder `path` a side is in: it must pass the folder rules and be a folder now.
    fn root_folder(&self, path: &Path) -> Result<PathBuf, Stop> {
        platform::check_folder(path).map_err(|reason| refusal(io::ErrorKind::InvalidInput, reason))?;

        let actual = platform::local_path(path)?;
        if !fs::metadata(&actual)?.is_dir() {
            return Err(refusal(io::ErrorKind::InvalidInput, "the source is not a folder").into());
        }

        Ok(actual)
    }

    /// Counts what the item `rel` will move, so the progress has its total.
    fn count_item(&mut self, root: &Path, rel: &str, plan: &SyncPlan) -> Result<(), Stop> {
        if plan.source.kind != EntryKind::Dir {
            self.found(plan.source.size);

            return Ok(());
        }

        // What cannot be read now is reported by the copy, which reads it again.
        if let Ok(Some(child)) = lookup_below(root, rel) {
            self.scan(&[child])?;
        }

        Ok(())
    }

    /// Copies the item `rel` from `source_root` to `target_root` when both are still as the plan saw them; counts it as
    /// skipped when not.
    fn sync_item(&mut self, source_root: &Path, target_root: &Path, rel: &str, plan: &SyncPlan) -> Result<(), Stop> {
        let Some(child) = lookup_below(source_root, rel)? else {
            return self.changed();
        };
        if !same_facts(Some(&facts_of(&child)), Some(&plan.source)) {
            return self.changed();
        }

        let Some((existing, missing)) = target_place(target_root, rel, plan.target.as_ref())? else {
            return self.changed();
        };

        self.create_folders(&existing, &missing)?;
        self.copy_node(&existing.join(&missing), &child, rel, 0)?;

        Ok(())
    }

    /// An item left alone because it is not as the scan saw it.
    pub(super) fn changed(&mut self) -> Result<(), Stop> {
        self.skipped = self.skipped.saturating_add(1);

        Ok(())
    }
}

fn unsupported() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "this job cannot start with these items")
}

/// Refuses two folders that are one, or one inside the other: copying between them would never end.
fn refuse_nested(one: &Path, other: &Path) -> Result<(), Stop> {
    let (one, other) = (real(one)?, real(other)?);

    if is_inside(&one, &other) || is_inside(&other, &one) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the two folders are the same, or one is inside the other",
        )
        .into());
    }

    Ok(())
}

/// What a copy of `meta` is as the plans state it. A link is `other`, which no plan holds.
pub(super) fn facts(meta: &Metadata) -> SyncFacts {
    let file_type = meta.file_type();
    let kind = if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };

    SyncFacts {
        kind,
        size: if kind == EntryKind::File { meta.len() } else { 0 },
        modified: modified_millis(meta),
    }
}

fn facts_of(child: &Child) -> SyncFacts {
    facts(&child.meta)
}

/// Whether `seen` is what `wanted` says: both missing, or the same kind and, for a file, the same size and the same time to
/// the millisecond. A folder's time is not looked at: it changes with whatever is put in it.
pub(super) fn same_facts(seen: Option<&SyncFacts>, wanted: Option<&SyncFacts>) -> bool {
    match (seen, wanted) {
        (None, None) => true,
        (Some(seen), Some(wanted)) => {
            seen.kind == wanted.kind
                && seen.kind != EntryKind::Other
                && (seen.kind == EntryKind::Dir || (seen.size == wanted.size && seen.modified == wanted.modified))
        }
        _ => false,
    }
}

/// Splits the path `rel` of an item into the folders on the way to it (`""` for none) and its name.
pub(super) fn split_item(rel: &str) -> (&str, &str) {
    rel.rsplit_once('/').unwrap_or(("", rel))
}

/// The item `rel` below `root`, found by entering each folder on the way with no link followed. `None` when the item or
/// a folder on the way is gone, or the way is a link or a file now.
fn lookup_below(root: &Path, rel: &str) -> io::Result<Option<Child>> {
    let (parents, name) = split_item(rel);
    let mut folder = root.to_owned();

    for part in parents.split('/').filter(|part| !part.is_empty()) {
        folder.push(part);

        match fs::symlink_metadata(&folder) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }

    match Child::lookup(&folder, name) {
        Ok(child) => Ok(Some(child)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Where the item `rel` lands below `root`: the deepest folder on the way that exists and the names below it that do not,
/// when the target is what `wanted` says (`None`: not there). `Ok(None)` when it is not. A link or a file where a folder
/// should be is an error: nothing is made through it.
pub(super) fn target_place(
    root: &Path,
    rel: &str,
    wanted: Option<&SyncFacts>,
) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let (parents, name) = split_item(rel);
    let mut existing = root.to_owned();
    let mut missing = PathBuf::new();

    for part in parents.split('/').filter(|part| !part.is_empty()) {
        if !missing.as_os_str().is_empty() {
            missing.push(part);
            continue;
        }

        let next = existing.join(part);
        match fs::symlink_metadata(&next) {
            Ok(meta) if meta.is_dir() => existing = next,
            Ok(_) => {
                return Err(refusal(
                    io::ErrorKind::AlreadyExists,
                    "a file or link has the name of a folder",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing.push(part),
            Err(error) => return Err(error),
        }
    }

    let seen = if missing.as_os_str().is_empty() {
        match fs::symlink_metadata(existing.join(name)) {
            Ok(meta) => Some(facts(&meta)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };

    Ok(same_facts(seen.as_ref(), wanted).then_some((existing, missing)))
}

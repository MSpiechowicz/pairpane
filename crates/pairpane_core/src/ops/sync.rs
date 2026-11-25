//! Synchronizing as a job: the items the page chose from a comparison of two folders are copied, each the way its plan says.
//!
//! The scan that made the plans is over, so the job trusts none of it. Each item is looked at again right before it is
//! copied: the source must still be what the scan saw (its kind, and for a file its size and its time to the
//! millisecond), and so must the target (the same file, or still not there; a folder that is copied whole must still not
//! exist at the target). An item that fails the look is skipped and counted, never copied over a change the user did not
//! see. Nothing is deleted by a copy, a link is never followed (one on the way to an item is a refusal, and a link inside a
//! folder that is copied whole is copied as a link, as a copy does), and a special file is skipped.
//!
//! Every folder on the way to an item is entered one name at a time with no link followed, in the source and in the target,
//! and the folders missing in the target are made. A file is written to a temporary file and renamed into place, with its
//! time kept; what it may replace is decided for the item, not for the job (see `sync_terms`): a file the scan saw at the
//! target is replaced only while it is still that very file, and where the scan saw nothing nothing is replaced.
//!
//! On Unix a file's target folders are entered, and its temporary file made and renamed, through the handle of each folder
//! (see `sync_write`), so a folder swapped for a link meanwhile leads nowhere. On Windows, and for a folder that is copied
//! whole, the steps go by path, the race the other copies document. Items from an archive are in `sync_archive`; copies into
//! an archive and removals from one are applied in one rewrite of it (`sync_into`), after the copies out of it.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use crate::listing::{modified_millis, platform};
use crate::model::{EntryKind, JobPhase, PaneId, SyncDirection, SyncFacts};

use super::extract::{ArchiveJob, open_archive};
use super::plan::{Child, Kind, is_inside, real};
use super::report::refusal;
use super::sync_remove::Removal;
use super::sync_terms::{Terms, seen_at};
use super::transfer::{Run, Stop};
use super::{SyncLog, SyncPlan, SyncRemove, SyncRoot};

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
    /// Runs the synchronization of the job: see the module's text. The removals (see `sync_remove`) come after every copy,
    /// so a job that is cancelled or fails has removed nothing it had not copied first.
    pub(super) fn sync(
        &mut self,
        (left, right): (&SyncRoot, &SyncRoot),
        plans: &[SyncPlan],
        (removals, permanent): (&[SyncRemove], bool),
        log: &SyncLog,
    ) -> Result<(), Stop> {
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

        let mut into_archive: Vec<(&str, &SyncPlan)> = Vec::new();
        let mut items: Vec<(&str, &SyncPlan, &Side<'_>, &Path)> = Vec::new();
        for (rel, plan) in request.names.iter().zip(plans) {
            let (source, target) = match plan.direction {
                SyncDirection::ToRight => (&left, &right),
                SyncDirection::ToLeft => (&right, &left),
            };

            match target {
                Side::Disk(target) => items.push((rel.as_str(), plan, source, target.as_path())),
                Side::Archive(_) => into_archive.push((rel.as_str(), plan)),
            }
        }

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
                    let before = self.before_item(log, target, rel, plan);
                    let done = self.sync_item(root, target, rel, plan);
                    let settled = self.settle(rel, done);
                    // Written down even when the job stops here: a cancel can come after the item is in place.
                    self.after_item(log, (target, rel), plan, before, None);
                    settled?;
                }
                Side::Archive(archive) => from_archive.push((archive, target, rel, plan)),
            }
        }

        if let Some((archive, target, ..)) = from_archive.first() {
            let rows = from_archive
                .iter()
                .map(|(_, _, rel, plan)| (*rel, *plan))
                .collect::<Vec<_>>();

            let before: Vec<_> = rows
                .iter()
                .map(|(rel, plan)| self.before_item(log, target, rel, plan))
                .collect();
            let mut wrote = HashSet::new();
            let extracted = self.sync_from_archive(archive, target, &rows, &mut wrote);
            for ((rel, plan), before) in rows.iter().zip(before) {
                self.after_item(log, (target, rel), plan, before, Some(wrote.contains(rel)));
            }
            extracted?;
        }

        let names_of_removals = &request.names[plans.len().min(request.names.len())..];
        let archive_side = [&left, &right].into_iter().find_map(|side| match side {
            Side::Archive(archive) => Some(archive),
            Side::Disk(_) => None,
        });
        if let Some(archive) = archive_side {
            let disk = [&left, &right].into_iter().find_map(|side| match side {
                Side::Disk(folder) => Some(folder.as_path()),
                Side::Archive(_) => None,
            });
            let archive_pane = if matches!(left, Side::Archive(_)) {
                PaneId::Left
            } else {
                PaneId::Right
            };
            let removed_from_archive: Vec<(&str, &SyncRemove)> = names_of_removals
                .iter()
                .zip(removals)
                .filter(|(_, removal)| removal.side == archive_pane)
                .map(|(rel, removal)| (rel.as_str(), removal))
                .collect();

            self.sync_into_archive(archive, disk, &into_archive, &removed_from_archive)?;
        }

        // An archive the folder is mirrored from is not read for the removals, so it must still be the file the comparison
        // read: a removal of a name it may have by now is not made (data loss audit, #68; #75).
        let archive_unchanged = archive_side.is_none_or(|archive| self.archive_unchanged(archive));
        let mut kept_for_archive = 0;
        let removed: Vec<Removal<'_>> = names_of_removals
            .iter()
            .zip(removals)
            .filter_map(|(rel, removal)| {
                let (side, other) = match removal.side {
                    PaneId::Left => (&left, &right),
                    PaneId::Right => (&right, &left),
                };
                let Side::Disk(root) = side else { return None };
                let source = match other {
                    Side::Disk(folder) => Some(folder.as_path()),
                    Side::Archive(_) if !archive_unchanged => {
                        kept_for_archive += 1;
                        return None;
                    }
                    Side::Archive(_) => None,
                };

                Some(Removal {
                    rel: rel.as_str(),
                    removal,
                    root: root.as_path(),
                    source,
                })
            })
            .collect();

        let archive = [&left, &right].into_iter().find_map(|side| match side {
            Side::Archive(archive) => Some(archive.folder.join(archive.name)),
            Side::Disk(_) => None,
        });

        for _ in 0..kept_for_archive {
            self.changed()?;
        }

        self.sync_removals(&removed, permanent, archive.as_deref())
    }

    /// Whether the archive of `side` is still the file the comparison read: what the job was given of it says, and
    /// nothing given is not trusted.
    fn archive_unchanged(&self, side: &ArchiveSide<'_>) -> bool {
        let Some(read) = side.source.fingerprint.as_ref() else {
            return false;
        };

        open_archive(&side.folder, &self.job.request.from, side.name).is_ok_and(|(_, _, now)| now == *read)
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

        let done = match child.kind {
            // A folder is copied whole to where there is none: one at the target, then or by now, is left alone.
            Kind::Dir if plan.target.is_some() => self.changed(),
            Kind::Dir => self.sync_by_path(&child, target_root, rel, plan),
            #[cfg(unix)]
            _ => self.sync_file(&child, target_root, rel, plan),
            #[cfg(not(unix))]
            _ => self.sync_by_path(&child, target_root, rel, plan),
        };
        self.set_terms(None);

        done
    }

    /// The steps of [`Run::sync_item`] that go by path: the target is looked at, the folders missing are made, and `child` is
    /// copied on the terms the look gave.
    fn sync_by_path(&mut self, child: &Child, target_root: &Path, rel: &str, plan: &SyncPlan) -> Result<(), Stop> {
        let Some(spot) = target_place(target_root, rel, plan.target.as_ref())? else {
            return self.changed();
        };

        self.set_terms(Some(spot.terms));
        self.create_folders(&spot.existing, &spot.missing)?;
        self.copy_node(&spot.existing.join(&spot.missing), child, rel, 0)?;

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
pub(super) fn lookup_below(root: &Path, rel: &str) -> io::Result<Option<Child>> {
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

/// Where an item lands: the deepest folder on the way that exists, the names below it that do not, and the terms the item is
/// written on.
pub(super) struct Spot {
    pub(super) existing: PathBuf,
    pub(super) missing: PathBuf,
    pub(super) terms: Terms,
}

/// Where the item `rel` lands below `root`, when the target is what `wanted` says (`None`: not there). `Ok(None)` when it is
/// not. A link or a file where a folder should be is an error: nothing is made through it.
pub(super) fn target_place(root: &Path, rel: &str, wanted: Option<&SyncFacts>) -> io::Result<Option<Spot>> {
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
        seen_at(&existing.join(name))?
    } else {
        None
    };

    Ok(Terms::judge(wanted, seen).map(|terms| Spot {
        existing,
        missing,
        terms,
    }))
}

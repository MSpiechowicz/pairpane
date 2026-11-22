//! The items of a synchronization that come out of an archive: they are extracted into the other folder, by the same
//! writing an extraction does (a temporary file next to the target, the time kept, a rename into place), with the checks
//! `sync` makes of every item.
//!
//! An item is looked at in the archive's index (its kind, size and time must be what the scan saw) and at its target
//! before the extraction starts, and a file once more right before it is written. The archive's own file is never replaced
//! by an entry of it, and a link or special entry (also inside a folder that is copied whole) is not written, only counted.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::archive::{self, EntryType, Input};
use crate::model::{EntryKind, JobPhase, SyncFacts};

use super::SyncPlan;
use super::extract::{open_archive, watched};
use super::extract_write::Landing;
use super::origin::Origin;
use super::plan::Child;
use super::sync::{ArchiveSide, same_facts, target_place};
use super::sync_terms::Terms;
use super::transfer::{Run, Stop};

impl Run<'_> {
    /// Extracts the `rows` (paths below the archive's folder and what the scan saw of each) into the folder `target`.
    pub(super) fn sync_from_archive(
        &mut self,
        side: &ArchiveSide<'_>,
        target: &Path,
        rows: &[(&str, &SyncPlan)],
    ) -> Result<(), Stop> {
        let job = self.job;
        let (found, file, print) = open_archive(&side.folder, &job.request.from, side.name)?;
        let cancel = AtomicBool::new(false);
        // Read once: every file written gets the same marker (see `origin`).
        let origin = Origin::of(&file, &found.path);
        let input = Input {
            file: &file,
            path: Some(&found.path),
        };
        let password = side.source.password.as_deref();

        watched(&job.control, &cancel, || {
            let index = self.indexed(side.name, side.source, &print, &input, &cancel)?;

            let ready = self.archive_rows(&index, side, (&found, target), rows)?;
            if ready.is_empty() {
                return Ok(());
            }

            let names: Vec<String> = ready.iter().map(|(rel, _)| (*rel).to_owned()).collect();
            let selection = self.select_for(&index, (side.inner, &names), password)?;

            self.phase(JobPhase::Extracting);
            let mut reader = self.entry_source(&index, &input, &cancel, password)?;

            // The terms of each entry are set as it comes (see `sync_terms`), looking at its target right then: a file that is a
            // row of its own on those of the plan, and what is in a folder that is a row on those of a folder made for it.
            let plans: HashMap<&str, &SyncPlan> = ready.iter().copied().collect();
            let mut stale: HashSet<&str> = HashSet::new();
            let mut gate = |run: &mut Run<'_>, landing: &mut Landing, entry: &archive::IndexEntry, rel: &str| {
                let Some((row, plan)) = owner(&plans, rel) else {
                    return Ok(true);
                };

                if plan.source.kind == EntryKind::Dir {
                    return run.enter_folder_row(landing, row, &mut stale);
                }
                if entry.kind != EntryType::File {
                    return Ok(true);
                }

                let Some(spot) = target_place(target, rel, plan.target.as_ref())? else {
                    run.changed()?;

                    return Ok(false);
                };
                run.set_terms(Some(spot.terms));

                Ok(true)
            };

            self.write_all(
                &mut *reader,
                &selection.items,
                &selection.ordinals(),
                (target.to_owned(), origin),
                &mut gate,
            )
        })
    }

    /// The rows that can be extracted: the entry is still what the scan saw, and so is the target, which is not the archive
    /// file itself. The others are counted as skipped (one that cannot be told because of an error fails alone).
    fn archive_rows<'r>(
        &mut self,
        index: &archive::ArchiveIndex,
        side: &ArchiveSide<'_>,
        (archive, target): (&Child, &Path),
        rows: &[(&'r str, &'r SyncPlan)],
    ) -> Result<Vec<(&'r str, &'r SyncPlan)>, Stop> {
        let mut ready = Vec::with_capacity(rows.len());

        for &(rel, plan) in rows {
            self.checkpoint()?;

            let seen = index.find(side.inner, rel).map(|child| entry_facts(&child));
            if !same_facts(seen.as_ref(), Some(&plan.source)) {
                self.changed()?;
                continue;
            }

            match target_place(target, rel, plan.target.as_ref()) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    self.changed()?;
                    continue;
                }
                Err(error) => {
                    self.fail(rel, &error);
                    continue;
                }
            }

            if self.is_archive_itself(archive, &target.join(rel)) {
                self.changed()?;
                continue;
            }

            ready.push((rel, plan));
        }

        Ok(ready)
    }

    /// The gate of an entry of the folder that is the row `row`: its first entry makes the folder, which must be new (else the
    /// row is left alone, counted once, and so is what is in it); what goes in is written on the terms of a new folder.
    fn enter_folder_row<'r>(
        &mut self,
        landing: &mut Landing,
        row: &'r str,
        stale: &mut HashSet<&'r str>,
    ) -> Result<bool, Stop> {
        if stale.contains(row) {
            return Ok(false);
        }

        if landing.made(row).is_none() {
            let made = self.folder_for(landing, row).map(|_| landing.made(row));

            match made {
                Ok(Some(true)) => {}
                Ok(_) => {
                    stale.insert(row);
                    self.changed()?;

                    return Ok(false);
                }
                Err(error) => {
                    stale.insert(row);

                    return Err(error);
                }
            }
        }

        self.set_terms(Some(Terms::Absent));

        Ok(true)
    }

    /// Whether `path` is the archive file `archive` (it would be replaced by an entry of itself).
    fn is_archive_itself(&self, archive: &Child, path: &Path) -> bool {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => self.same_file(&archive.meta, &archive.path, &meta, path),
            Err(_) => false,
        }
    }
}

/// The row of the plans that `rel` is, or is inside (the folder that is copied whole): its path and its plan.
fn owner<'r>(plans: &HashMap<&'r str, &'r SyncPlan>, rel: &str) -> Option<(&'r str, &'r SyncPlan)> {
    let own = plans.get_key_value(rel);
    let above = rel
        .match_indices('/')
        .find_map(|(at, _)| plans.get_key_value(&rel[..at]));

    own.or(above).map(|(row, plan)| (*row, *plan))
}

/// What the entry `child` of the index is as the plans state it: a link or special entry is `other`, which no plan holds.
fn entry_facts(child: &archive::Child<'_>) -> SyncFacts {
    let (kind, size, modified) = match child.entry {
        _ if child.dir => (EntryKind::Dir, 0, None),
        Some(entry) if entry.kind == EntryType::File => (EntryKind::File, entry.size, entry.modified),
        _ => (EntryKind::Other, 0, None),
    };

    SyncFacts { kind, size, modified }
}

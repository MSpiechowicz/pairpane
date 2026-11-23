//! The changes of a synchronization to an archive: the items copied into it from the other folder, and those a mirror
//! removes from it, all applied in one rewrite of the archive, as adding files to an archive does (see `add`): the archive
//! is checked to be the file the caller's index was made from at the start and once more right before it is replaced, the
//! new archive is written to a temporary file beside it, and the old one is never written.
//!
//! Each item is looked at again first, and is left alone (counted as skipped) when it is not what the scan saw:
//!
//! - A copy: the source item, found below its folder by handle with no link followed, must be of the kind, size and time
//!   the scan saw. The archive must have, at the item's path, what the scan saw there: nothing (a folder is only ever
//!   copied whole to where there is nothing), or a file of that size and time, which the new file replaces (every entry
//!   of that path is left out). A file of the archive where a folder on the way to the item would be fails the item.
//! - A removal: the entry must be of the kind, size and time the scan saw, and a folder must still hold the files and bytes
//!   it did; it goes with all that is below it. The archive is the file the scan read, by its fingerprint, so this is what
//!   the scan was shown.
//!
//! An archive that is not written (rar, a single gzip file), whose index is not whole or lacks the folder compared, or whose
//! encrypted entries have no good password, fails the job before anything is written. Nothing changed at all leaves the
//! archive as it is. The archive itself, or a folder that holds it, is never copied into it (it is counted as skipped).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::archive::{self, ArchiveIndex, EntryType, Input, rewrite};
use crate::model::EntryKind;

use super::add::{Change, Opened, check_changeable, paths_of};
use super::extract::{open_archive, watched};
use super::held::Held;
use super::pack::Plan;
use super::plan::{is_inside, real};
use super::report::refusal;
use super::sync::{ArchiveSide, same_facts};
use super::sync_archive::entry_facts;
use super::sync_remove::facts_of;
use super::transfer::{Run, Stop};
use super::tree::Spot;
use super::{SyncPlan, SyncRemove};

impl Run<'_> {
    /// Applies the `copies` (paths below the folder `source`, and what the scan saw of each) and the `removals` to the
    /// archive of `side` in one rewrite (see the module's text).
    pub(super) fn sync_into_archive(
        &mut self,
        side: &ArchiveSide<'_>,
        source: Option<&std::path::Path>,
        copies: &[(&str, &SyncPlan)],
        removals: &[(&str, &SyncRemove)],
    ) -> Result<(), Stop> {
        if copies.is_empty() && removals.is_empty() {
            return Ok(());
        }

        let job = self.job;
        let (found, file, print) = open_archive(&side.folder, &job.request.from, side.name)?;
        let opened = Opened {
            name: side.name,
            shown: &job.request.from,
            folder: side.folder.clone(),
            path: found.path,
            print,
        };
        let cancel = AtomicBool::new(false);

        watched(&job.control, &cancel, || {
            let input = Input {
                file: &file,
                path: Some(&opened.path),
            };
            let index = self.indexed(side.name, side.source, &opened.print, &input, &cancel)?;
            check_changeable(&index, side.inner)?;
            self.check_password(&index, &file, &cancel, side.source.password.as_deref())?;

            let prefix = if side.inner.is_empty() {
                String::new()
            } else {
                format!("{}/", side.inner)
            };
            let mut left_out = Vec::new();
            let plan = match source {
                Some(source) if !copies.is_empty() => {
                    let archive = real(&opened.path)?;
                    self.plan_copies(&index, (source, &prefix, &archive), copies, &mut left_out)?
                }
                _ => None,
            };
            let removed = self.plan_removals(&index, &prefix, removals, &mut left_out)?;

            let entries = plan.as_ref().map_or(0, |plan| plan.entries.len());
            if entries == 0 && left_out.is_empty() {
                return Ok(());
            }

            left_out.sort_unstable();
            left_out.dedup();
            let change = Change {
                index: &index,
                password: side.source.password.as_deref(),
                copy: rewrite::bytes_to_copy(&index, &left_out),
                left_out,
            };
            let plan = match plan {
                Some(plan) => plan,
                None => Plan::new(self, &Arc::new(Held::root(&opened.folder)?), prefix, self.pack_limits()),
            };

            self.replace_archive(&opened, file, &plan, &change, &cancel)?;

            // What was removed from the archive is gone only now.
            for _ in 0..removed {
                self.renamed();
            }

            Ok(())
        })
    }

    /// Plans the copies into the archive of `index` from the folder `source`, under `prefix`; adds to `left_out` the entries
    /// the new files replace.
    fn plan_copies(
        &mut self,
        index: &ArchiveIndex,
        (source, prefix, archive): (&std::path::Path, &str, &std::path::Path),
        copies: &[(&str, &SyncPlan)],
        left_out: &mut Vec<u32>,
    ) -> Result<Option<Plan>, Stop> {
        let root = Arc::new(Held::root(source)?);
        let real_source = real(source)?;
        let (most_entries, most_names) = self.pack_limits();
        let names: usize = index.entries().iter().map(|entry| entry.path.len()).sum();
        let limits = (
            most_entries.saturating_sub(index.entries().len()),
            most_names.saturating_sub(names),
        );
        let mut plan = Plan::new(self, &root, prefix.to_owned(), limits);
        let paths = paths_of(index);

        self.visited.clear();
        for &(rel, wanted) in copies {
            self.checkpoint()?;

            let inner = prefix.trim_end_matches('/');
            if let Some(blocked) = file_on_the_way(index, inner, rel) {
                self.fail(rel, &blocked);
                continue;
            }

            let target = index.find(inner, rel).map(|child| entry_facts(&child));
            if !same_facts(target.as_ref(), wanted.target.as_ref()) {
                self.changed()?;
                continue;
            }

            // The archive itself, or a folder that holds it, would be packed into the new archive.
            if is_inside(archive, &real_source.join(rel)) {
                self.changed()?;
                continue;
            }

            let found = match self.find(&root, rel) {
                Ok(found) => found,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.changed()?;
                    continue;
                }
                Err(error) => {
                    self.fail(rel, &error);
                    continue;
                }
            };
            if !same_facts(Some(&facts_of(&found.node)), Some(&wanted.source)) {
                self.changed()?;
                continue;
            }

            let at = Spot {
                parent: &found.parent,
                node: &found.node,
                rel,
                trail: &found.trail,
            };
            let before = plan.entries.len();
            self.walk_spot(&at, found.depth, &mut plan)?;

            // The file the scan saw there goes when its new one is planned.
            if wanted.target.is_some() && plan.entries.len() > before {
                left_out.extend(paths.get(format!("{prefix}{rel}").as_str()).into_iter().flatten());
            }
        }

        Ok(Some(plan))
    }

    /// Adds to `left_out` the entries of each of `removals` (under `prefix`) that is still what the scan saw, with all below
    /// it, and gives how many there are; the others are counted as skipped.
    fn plan_removals(
        &mut self,
        index: &ArchiveIndex,
        prefix: &str,
        removals: &[(&str, &SyncRemove)],
        left_out: &mut Vec<u32>,
    ) -> Result<usize, Stop> {
        let mut removed = 0;
        let inner = prefix.trim_end_matches('/');

        for &(rel, removal) in removals {
            self.checkpoint()?;

            let seen = index.find(inner, rel).map(|child| entry_facts(&child));
            if !same_facts(seen.as_ref(), Some(&removal.seen)) {
                self.changed()?;
                continue;
            }

            let path = format!("{prefix}{rel}");
            let below = format!("{path}/");
            let entries: Vec<&archive::IndexEntry> = index
                .entries()
                .iter()
                .filter(|entry| entry.path == path || entry.path.starts_with(&below))
                .collect();

            if removal.seen.kind == EntryKind::Dir {
                let files = entries.iter().filter(|entry| entry.kind == EntryType::File);
                let totals = files.fold((0_u64, 0_u64), |(count, bytes), entry| {
                    (count + 1, bytes.saturating_add(entry.size))
                });
                if totals != (removal.files, removal.bytes) {
                    self.changed()?;
                    continue;
                }
            }

            left_out.extend(entries.iter().map(|entry| entry.ordinal));
            removed += 1;
        }

        Ok(removed)
    }
}

/// The refusal of an item whose way into the archive goes through a file of the archive (`a` is a file, `a/b` is copied).
fn file_on_the_way(index: &ArchiveIndex, inner: &str, rel: &str) -> Option<std::io::Error> {
    rel.match_indices('/')
        .map(|(at, _)| &rel[..at])
        .find(|above| index.find(inner, above).is_some_and(|child| !child.dir))
        .map(|_| {
            refusal(
                std::io::ErrorKind::AlreadyExists,
                "a file of the archive has the name of a folder on the way",
            )
        })
}

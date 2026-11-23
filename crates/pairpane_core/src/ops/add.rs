//! Adding files to an archive as a job: the items of the source folder (folders with all below them) are put into the
//! archive, a file of the target folder, under its folder `inner`, by writing a whole new archive beside it and replacing
//! the old one when the new one is complete (`archive::rewrite` writes it; the old file is never written).
//!
//! The archive is opened without following a link and checked to be the file the caller's index was made from (see
//! [`Run::indexed`]); one that changed is refused with "the archive changed; open it again", at the start and once more just
//! before it is replaced. An archive that is not written (a RAR, a single gzip file), whose index is not whole (too large,
//! or with names that are refused, which a rewrite could not keep) or that lacks the folder is refused, and so is an
//! archive with encrypted entries when the password is missing or wrong (the new entries are sealed with it: a zip's is
//! judged by decrypting its smallest encrypted file whole, not only by its check byte). What a rewrite would lose is
//! refused as well (see `archive::rewrite`): data after the end of a tar, before the first entry of a zip, or a tar's
//! global settings that would change the added files.
//!
//! The items are planned as a pack plans them (no link is followed, names in order, what the archive code would refuse
//! fails the item), under `inner`. A file or a folder that the archive has as the other kind fails that item (with all
//! below it for a folder). A folder the archive has already is merged: it stays as it is and what is in it is added. A file
//! the archive has already asks (see `conflict`): overwrite leaves the old entry (all that have its name) out of the new
//! archive, skip leaves the new file out, rename adds it under a name that neither the archive nor the job uses, and
//! overwrite older replaces the entry only when the new file's time is later.
//!
//! The new archive is written to a temporary file next to the old one (the disk keeps its reserve; it is made readable by
//! its owner alone on Unix, and gets the permission bits and the group of the old one at once, through its open handle,
//! before a byte is written), flushed to the disk, and put in place with the replacing rename. The owner, hard links,
//! extended attributes and ACLs of the old one are not kept. A cancel or any failure
//! removes the temporary file and leaves the old archive as it was. The progress counts the bytes of the new files and
//! the bytes taken from the old archive.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::archive::rewrite::{self, Old};
use crate::archive::{
    self, ArchiveIndex, Fingerprint, Input, Password, PasswordFault, cancelled, password_error,
    unverified_password_error,
};
use crate::listing::platform;
use crate::model::{ArchiveFormat, FileFacts, JobPhase};

use super::conflict::{Decision, free_entry_name};
use super::extract::{ArchiveJob, changed, open_archive, watched};
use super::held::{Held, Node};
use super::pack::{OUTPUT_BUFFER, Output, Plan, Source, count_empty, entry_of, sync_output};
use super::partial::Partial;
use super::report::refusal;
use super::task::create_private;
use super::transfer::{Run, Stop};
use super::tree::{Spot, Visitor};

/// The walk that plans the items of an add job: the plan of a pack job, and the clashes with what the archive has.
struct Adding<'i> {
    plan: Plan,
    index: &'i ArchiveIndex,
    /// The folder of the archive the items go into.
    inner: &'i str,
}

impl Adding<'_> {
    /// Fails when the archive has, at the path of `at`, the other kind of entry: a folder where a file is added, or a file
    /// where a folder is.
    fn check_kind(&self, at: &Spot<'_>, folder: bool) -> Result<(), Stop> {
        let text = match self.index.find(self.inner, at.rel) {
            Some(existing) if existing.dir && !folder => "a folder of the archive has the name of this file",
            Some(existing) if !existing.dir && folder => "a file of the archive has the name of this folder",
            _ => return Ok(()),
        };

        Err(refusal(io::ErrorKind::AlreadyExists, text).into())
    }
}

impl Visitor for Adding<'_> {
    fn file(&mut self, run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop> {
        self.check_kind(at, false)?;

        self.plan.file(run, at)
    }

    fn folder(&mut self, run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop> {
        self.check_kind(at, true)?;

        self.plan.folder(run, at)
    }

    fn fatal(&self) -> bool {
        self.plan.fatal()
    }
}

/// The archive an add job (or a synchronization into an archive) changes, as the job opened it.
pub(super) struct Opened<'a> {
    pub(super) name: &'a str,
    /// The archive's folder as the request names it (its state is told by that path), the folder resolved, and the
    /// archive's path.
    pub(super) shown: &'a std::path::Path,
    pub(super) folder: PathBuf,
    pub(super) path: PathBuf,
    pub(super) print: Fingerprint,
}

/// What the new archive is made of: the old one, minus the entries left out, and the planned new entries.
pub(super) struct Change<'a> {
    pub(super) index: &'a ArchiveIndex,
    pub(super) password: Option<&'a Password>,
    pub(super) left_out: Vec<u32>,
    /// The bytes that are taken from the old archive, about.
    pub(super) copy: u64,
}

impl Run<'_> {
    /// Runs the add job of the archive `name` in the request's target folder (see the module's text).
    pub(super) fn add_to_archive(&mut self, name: &str, inner: &str, source: &ArchiveJob) -> Result<(), Stop> {
        let job = self.job;
        let request = &job.request;

        let (root, items) = self.sorted_items()?;
        let folder = platform::local_path(&request.to)?;
        if !fs::metadata(&folder)?.is_dir() {
            return Err(refusal(io::ErrorKind::InvalidInput, "the target is not a folder").into());
        }

        let (found, file, print) = open_archive(&folder, &request.to, name)?;
        let opened = Opened {
            name,
            shown: &request.to,
            folder,
            path: found.path,
            print,
        };
        let cancel = AtomicBool::new(false);

        watched(&job.control, &cancel, || {
            self.change_archive(&opened, file, inner, source, (&root, &items), &cancel)
        })
    }

    fn change_archive(
        &mut self,
        opened: &Opened<'_>,
        archive: File,
        inner: &str,
        source: &ArchiveJob,
        (root, items): (&Arc<Held>, &[Node]),
        cancel: &AtomicBool,
    ) -> Result<(), Stop> {
        let input = Input {
            file: &archive,
            path: Some(&opened.path),
        };
        let index = self.indexed(opened.name, source, &opened.print, &input, cancel)?;
        check_changeable(&index, inner)?;

        let password = source.password.as_deref();
        // Judging a password may read a whole entry: the job shows it is working.
        self.phase(JobPhase::Scanning);
        self.check_password(&index, &archive, cancel, password)?;

        let mut plan = self.plan_additions(root, items, &index, inner)?;
        if plan.entries.is_empty() {
            return Err(refusal(io::ErrorKind::InvalidInput, "there is nothing to add").into());
        }

        let left_out = self.settle_clashes(&mut plan, &index, inner)?;
        // Everything was skipped or merged: the archive stays as it is.
        if plan.entries.is_empty() && left_out.is_empty() {
            return Ok(());
        }

        let change = Change {
            index: &index,
            password,
            copy: rewrite::bytes_to_copy(&index, &left_out),
            left_out,
        };
        self.replace_archive(opened, archive, &plan, &change, cancel)
    }

    /// Writes the new archive of `change` and `plan` beside the old one and puts it in place when the old one is still the
    /// one that was read (see the module's text).
    pub(super) fn replace_archive(
        &mut self,
        opened: &Opened<'_>,
        archive: File,
        plan: &Plan,
        change: &Change<'_>,
        cancel: &AtomicBool,
    ) -> Result<(), Stop> {
        self.found(change.copy);

        self.phase(JobPhase::Packing);
        let (temp, file) = self.make_temp(&opened.folder, create_private)?;
        let mut guard = Partial::new(temp.clone());
        if let Some(hook) = &self.inner.hooks.after_temp {
            hook(&temp);
        }
        self.keep_mode(&archive, &file)?;

        self.write_changed(plan, change, (opened, &archive, file), cancel)?;
        self.checkpoint()?;

        // Not held any more when it is replaced (Windows does not replace a file that another handle has open).
        drop(archive);

        // The archive may have been replaced or written to while the new one was made: it is only replaced if it is the
        // one that was read.
        self.check_unchanged(opened)?;
        self.rename_replace(&temp, &opened.path)?;
        guard.committed = true;

        Ok(())
    }

    /// Fails when `password` is missing for an archive with encrypted content, or does not open it: the new entries are
    /// sealed with it, so it is checked as far as `archive::confirm_password` can (a whole entry of a zip, not only its
    /// check byte, which a wrong password passes one time in 256).
    pub(super) fn check_password(
        &self,
        index: &ArchiveIndex,
        file: &File,
        cancel: &AtomicBool,
        password: Option<&Password>,
    ) -> Result<(), Stop> {
        if !rewrite::is_encrypted(index) {
            return Ok(());
        }

        let Some(password) = password else {
            return Err(password_error(PasswordFault::Needed).into());
        };

        match archive::confirm_password(index, file, cancel, password) {
            Ok(true) => Ok(()),
            Ok(false) => Err(unverified_password_error().into()),
            Err(error) => Err(self.stopped(error)),
        }
    }

    /// Walks the `items` of `root` and plans the entries to add under the folder `inner` of the archive of `index`.
    fn plan_additions(
        &mut self,
        root: &Arc<Held>,
        items: &[Node],
        index: &ArchiveIndex,
        inner: &str,
    ) -> Result<Plan, Stop> {
        let (most_entries, most_names) = self.pack_limits();
        let names: usize = index.entries().iter().map(|entry| entry.path.len()).sum();
        // What the archive has already is part of what it may hold.
        let limits = (
            most_entries.saturating_sub(index.entries().len()),
            most_names.saturating_sub(names),
        );

        let prefix = if inner.is_empty() {
            String::new()
        } else {
            format!("{inner}/")
        };
        let mut adding = Adding {
            plan: Plan::new(self, root, prefix, limits),
            index,
            inner,
        };
        self.walk_items(root, items, &mut adding)?;

        Ok(adding.plan)
    }

    /// Decides what becomes of the new entries whose path the archive has already (see the module's text): the entries of
    /// `plan` that are not written any more are removed from it (a file that is skipped counts as finished), the ones that
    /// are renamed get their new name. Gives the ordinals (ascending) of the entries of the old archive that are left out.
    fn settle_clashes(&mut self, plan: &mut Plan, index: &ArchiveIndex, inner: &str) -> Result<Vec<u32>, Stop> {
        let skip = if inner.is_empty() { 0 } else { inner.len() + 1 };
        let mut taken: HashSet<String> = plan.entries.iter().map(|planned| planned.name.clone()).collect();
        let mut by_path: Option<HashMap<&str, Vec<u32>>> = None;
        let mut left_out = Vec::new();
        let mut kept = Vec::with_capacity(plan.entries.len());

        for mut planned in std::mem::take(&mut plan.entries) {
            self.checkpoint()?;

            let Some(existing) = index.find("", &planned.name) else {
                kept.push(planned);

                continue;
            };
            let Source::File { size, .. } = planned.source else {
                // A folder the archive has: the two are merged.
                continue;
            };

            let facts = |size, modified| FileFacts {
                size,
                modified,
                dir: false,
            };
            let old = existing.entry;
            let decision = self.decide_entry(
                &planned.name[skip..],
                &facts(size, planned.modified),
                &facts(old.map_or(0, |entry| entry.size), old.and_then(|entry| entry.modified)),
                || {
                    free_entry_name(&planned.name, |path| {
                        taken.contains(path) || index.find("", path).is_some()
                    })
                },
            )?;

            match decision {
                Decision::Replace => {
                    let paths = by_path.get_or_insert_with(|| paths_of(index));
                    left_out.extend(paths.get(planned.name.as_str()).into_iter().flatten());
                    kept.push(planned);
                }
                Decision::Rename(name) => {
                    taken.insert(name.clone());
                    planned.name = name;
                    kept.push(planned);
                }
                Decision::Skip => {
                    self.skipped += 1;
                    // Its bytes were counted when it was found: it is finished without being read.
                    self.begin_file(&planned.name, size);
                    self.end_file();
                }
            }
        }

        plan.entries = kept;
        left_out.sort_unstable();
        left_out.dedup();

        Ok(left_out)
    }

    /// Writes the new archive into `file` (in the archive's folder) and flushes it to the disk.
    fn write_changed(
        &mut self,
        plan: &Plan,
        change: &Change<'_>,
        (opened, archive, file): (&Opened<'_>, &File, File),
        cancel: &AtomicBool,
    ) -> Result<(), Stop> {
        let run = RefCell::new(&mut *self);
        let stop = |error: io::Error| run.borrow().stopped(error);

        run.borrow_mut().begin_file(opened.name, change.copy);
        let mut left = change.copy;
        let mut progress = |bytes: u64| -> io::Result<()> {
            let mut run = run.borrow_mut();

            if run.checkpoint().is_err() {
                cancel.store(true, Ordering::Relaxed);

                return Err(cancelled());
            }

            // The bytes counted are about the size: no more than that are counted.
            let counted = bytes.min(left);
            left -= counted;
            run.add_bytes(counted);

            Ok(())
        };

        let output = Output::new(file, &run, &opened.folder);
        let entries = plan
            .entries
            .iter()
            .map(|planned| entry_of(&run, cancel, planned))
            .collect();
        let old = Old {
            index: change.index,
            file: archive,
            cancel,
            password: change.password,
            left_out: &change.left_out,
            progress: &mut progress,
        };

        let written = rewrite::rewrite(BufWriter::with_capacity(OUTPUT_BUFFER, output), old, entries).map_err(stop)?;
        sync_output(written).map_err(stop)?;

        for planned in &plan.entries {
            count_empty(&run, planned);
        }

        // The old archive's own part of the work is finished.
        let mut run = run.borrow_mut();
        run.begin_file(opened.name, 0);
        run.end_file();

        Ok(())
    }

    /// Fails when the file at the path of the archive is not the one that was opened (see the module's text).
    fn check_unchanged(&self, opened: &Opened<'_>) -> Result<(), Stop> {
        let (_, _, now) = open_archive(&opened.folder, opened.shown, opened.name)?;
        if now != opened.print {
            return Err(changed().into());
        }

        Ok(())
    }
}

/// Refuses the archive of `index` when it cannot be rewritten (see the module's text) or has no folder `inner`.
pub(super) fn check_changeable(index: &ArchiveIndex, inner: &str) -> Result<(), Stop> {
    if matches!(index.format(), ArchiveFormat::Gz | ArchiveFormat::Rar) {
        return Err(refusal(io::ErrorKind::Unsupported, "this archive cannot be changed").into());
    }
    if index.truncated() {
        return Err(refusal(io::ErrorKind::Unsupported, "the archive is too large to be changed").into());
    }
    if index.refused() > 0 {
        return Err(refusal(
            io::ErrorKind::Unsupported,
            "the archive has names that cannot be kept, so it cannot be changed",
        )
        .into());
    }
    if !index.has_dir(inner) {
        return Err(refusal(io::ErrorKind::NotFound, "the folder is not in the archive").into());
    }

    Ok(())
}

/// The ordinals of the entries of `index`, by their path.
pub(super) fn paths_of(index: &ArchiveIndex) -> HashMap<&str, Vec<u32>> {
    let mut paths: HashMap<&str, Vec<u32>> = HashMap::new();

    for entry in index.entries() {
        paths.entry(entry.path.as_str()).or_default().push(entry.ordinal);
    }

    paths
}

impl Run<'_> {
    /// Gives the new archive `temp` the permission bits and the group of the old one, which `archive` is open (Unix). It is
    /// done through the handle of the new file, never its path, which someone may have swapped for a link by now. The
    /// group is set first: a user who is not in it cannot, and then the group's bits are cleared, so the new archive is not
    /// open to a group it did not have. The owner, hard links, extended attributes and ACLs of the old file are not kept.
    #[cfg(unix)]
    fn keep_mode(&self, archive: &File, temp: &File) -> Result<(), Stop> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let old = archive.metadata()?;
        let mut mode = old.permissions().mode() & 0o777;

        let group = rustix::fs::Gid::from_raw(old.gid());
        if self.inner.hooks.refuse_group || rustix::fs::fchown(temp, None, Some(group)).is_err() {
            mode &= !0o070;
        }

        temp.set_permissions(fs::Permissions::from_mode(mode))?;

        Ok(())
    }

    /// Windows keeps the permissions of a file with the file's own attributes, which a new file has as it is made.
    #[cfg(not(unix))]
    fn keep_mode(&self, _archive: &File, _temp: &File) -> Result<(), Stop> {
        Ok(())
    }
}

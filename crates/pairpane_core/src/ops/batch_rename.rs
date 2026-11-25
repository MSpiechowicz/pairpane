//! Renaming many items of one folder in one job, which the multi-rename and its undo are: names are swapped, passed
//! around a cycle or shifted along a chain, and nothing is ever replaced.
//!
//! The renames go in two passes, so that a name is never needed before it is free:
//!
//! 1. Every item whose new name is the name of another item of the job (or its own name in another case, where names
//!    ignore case) is first moved aside, to a temporary name that nothing has (`.pairpane-rename-…-` and its own name, see
//!    [`aside_name`], so one left there by a crash still says what it was): the rename refuses
//!    to replace, so a taken name is skipped over. Then every other item goes straight to its new name. Each step can be
//!    cancelled, and a cancel puts the items that are aside back under their own names.
//! 2. Then each item that is aside goes to its new name, which is free now unless someone made it so in between. This
//!    step is not cancelled, as it would leave the items half way. One that cannot go is put back under its own name; and
//!    if that fails as well, the failure of the item says it was left under the temporary name.
//!
//! Every step is a rename in the folder held open (see `held_rename`), and none replaces what is at the new name. The
//! pairs that were renamed go into the job's [`RenameLog`], which the engine closes when the job ends. An undo job (`undo`) is the same with the pairs reversed,
//! where an item that is gone or a name that is taken is skipped and counted, as the state of the folder changed since.

use std::collections::HashSet;
use std::io;
use std::path::Path;

use crate::model::JobPhase;

use super::held::Held;
use super::partial::run_token;
use super::rename::aside_name;
use super::rename_log::RenameLog;
use super::report::refusal;
use super::transfer::{Run, Stop};
use super::validate::fold_case;

/// Temporary names tried for one item before giving up.
const MAX_ASIDE_TRIES: u32 = 100;

/// One item to rename, and its temporary name while it is aside.
struct Step {
    from: String,
    to: String,
    aside: Option<String>,
}

impl Run<'_> {
    /// Renames the items of the job to `targets`, as described above.
    pub(super) fn rename_batch(&mut self, targets: &[String], undo: bool, log: &RenameLog) -> Result<(), Stop> {
        let job = self.job;
        let folder = self.open_folders(&[job.request.from.as_path()])?;
        let held = Held::root(&folder)?;
        Self::check_folder_identity(&held, undo, log)?;

        self.phase(JobPhase::Renaming);
        self.expect_items(job.request.names.len());

        let mut steps = self.look_up_steps(&held, &job.request.names, targets, undo);
        let leaving: HashSet<String> = steps.iter().map(|step| fold_case(&step.from)).collect();
        let (first, direct): (Vec<usize>, Vec<usize>) =
            (0..steps.len()).partition(|&index| leaving.contains(&fold_case(&steps[index].to)));

        let mut aside = Vec::new();
        for index in first {
            self.pause_point(&held, &steps, &aside)?;

            match self.move_aside(&held, &folder, &steps[index].from) {
                Ok(name) => {
                    steps[index].aside = Some(name);
                    aside.push(index);
                }
                Err(error) => self.refuse(undo, &steps[index].from, &error),
            }
        }

        for index in direct {
            self.pause_point(&held, &steps, &aside)?;
            let step = &steps[index];

            match held.rename_noreplace(&step.from, &step.to) {
                Ok(()) => {
                    log.push(&step.from, &step.to);
                    self.item_done(&step.from);
                }
                Err(error) => self.refuse(undo, &step.from, &error),
            }
        }

        // The last place to stop: from here the items that are aside go to their names.
        self.pause_point(&held, &steps, &aside)?;
        for index in aside {
            self.finish_aside(&held, &steps[index], undo, log);
        }

        Ok(())
    }

    /// A rename keeps the identity of the folder it opened in `log`. An undo goes on only in that same folder, so it never
    /// renames in whatever the remembered path leads to now (a link to another folder, say), and is refused when the
    /// identity of either cannot be read.
    fn check_folder_identity(held: &Held, undo: bool, log: &RenameLog) -> Result<(), Stop> {
        let here = held.identity();

        if !undo {
            log.set_folder(here);

            return Ok(());
        }

        match (log.folder(), here) {
            (Some(was), Some(now)) if was == now => Ok(()),
            _ => Err(refusal(io::ErrorKind::Other, "The folder changed; nothing was undone.").into()),
        }
    }

    /// Looks up the items; one that is gone is left out (and reported, or skipped by an undo).
    fn look_up_steps(&mut self, held: &Held, names: &[String], targets: &[String], undo: bool) -> Vec<Step> {
        let mut steps = Vec::with_capacity(names.len());

        for (from, to) in names.iter().zip(targets) {
            match held.look(from) {
                Ok(_) => steps.push(Step {
                    from: from.clone(),
                    to: to.clone(),
                    aside: None,
                }),
                Err(error) => self.refuse(undo, from, &error),
            }
        }

        steps
    }

    /// Stops here when the job is cancelled, and waits here while it is paused. A cancel puts the items that are `aside`
    /// back first.
    fn pause_point(&mut self, held: &Held, steps: &[Step], aside: &[usize]) -> Result<(), Stop> {
        if let Err(cancelled) = self.checkpoint() {
            for step in aside.iter().map(|&index| &steps[index]) {
                self.put_back(held, step);
            }

            return Err(cancelled.into());
        }

        Ok(())
    }

    /// Moves the item `from` to a temporary name that is free, and returns it.
    fn move_aside(&mut self, held: &Held, folder: &Path, from: &str) -> io::Result<String> {
        for _ in 0..MAX_ASIDE_TRIES {
            self.sequence += 1;
            let name = aside_name(
                &format!(".pairpane-rename-{}-{}-{}", run_token(), self.job.id.0, self.sequence),
                from,
            );

            match held.rename_noreplace(from, &name) {
                Ok(()) => {
                    // A test puts something at a new name here.
                    if let Some(hook) = &self.inner.hooks.after_aside {
                        hook(&folder.join(&name));
                    }

                    return Ok(name);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }

        Err(refusal(io::ErrorKind::Other, "no free temporary name was found"))
    }

    /// Moves an item that is aside to its new name, or puts it back when that fails.
    fn finish_aside(&mut self, held: &Held, step: &Step, undo: bool, log: &RenameLog) {
        let Some(name) = &step.aside else {
            return;
        };

        match held.rename_noreplace(name, &step.to) {
            Ok(()) => {
                log.push(&step.from, &step.to);
                self.item_done(&step.from);
            }
            Err(error) => {
                if self.put_back(held, step) {
                    self.refuse(undo, &step.from, &error);
                }
            }
        }
    }

    /// Moves the item that is aside back under its own name. When that cannot be done the item's failure says where it
    /// is, and this returns `false`.
    fn put_back(&mut self, held: &Held, step: &Step) -> bool {
        let Some(name) = &step.aside else {
            return true;
        };

        if held.rename_noreplace(name, &step.from).is_ok() {
            return true;
        }

        self.fail(
            &step.from,
            &io::Error::other(format!("the item was left under the name {name}")),
        );
        self.item_done(&step.from);

        false
    }

    /// An item could not be renamed: an undo skips one whose name or whose old name changed (it is not the folder it left),
    /// a rename reports it. Either way the item is over.
    fn refuse(&mut self, undo: bool, name: &str, error: &io::Error) {
        let changed = matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists);

        if undo && changed {
            self.skipped += 1;
        } else {
            self.fail(name, error);
        }

        self.item_done(name);
    }

    /// An item is over, however it came out: the progress counts it.
    fn item_done(&mut self, name: &str) {
        self.begin_file(name, 0);
        self.end_file();

        if let Some(hook) = &self.inner.hooks.after_item {
            hook(name);
        }
    }
}

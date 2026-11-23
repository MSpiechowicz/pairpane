//! The status of a folder, as the helper reads it: gix opens exactly the repository that was looked at, the three comparisons
//! limited to the folder are merged into marks, and every source of rules is counted in one budget. This runs in the helper
//! process (`child`), confined to the folders of the [`Layout`]; the unit tests run it in their own.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use gix::bstr::{BString, ByteSlice};
use gix::dir::walk::EmissionMode;

use super::budget::{Budget, Charged};
use super::compare::{BoundedCompare, NoSubmodules};
use super::ignored::Quiet;
use super::marks::Marks;
use super::path::prefix_of;
use super::request::Request;
use super::staged::{self, Head};
use super::visit::{Changes, Walker};
use super::watch::{Meter, Stop, guarded};
use super::{GitError, open};
use crate::model::GitStatus;

/// The status of the direct children of the folder of `request`; see [`super::status`]. `cancel` stops it as the app's exit
/// flag does.
pub(super) fn read(request: &Request, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    let stop = Stop::default();
    let limits = request.limits;

    let result = guarded(&stop, limits.time, cancel, || {
        let mut meter = Meter::new(&stop, limits.entries);

        Read { request, stop: &stop }.run(&mut meter)
    });

    // A flag that was raised is the answer, whatever error the interrupted read ended in.
    match stop.reason() {
        Some(reason) => Err(reason),
        None => result,
    }
}

pub(super) fn not_a_repository() -> GitStatus {
    GitStatus {
        repo: false,
        marks: Vec::new(),
        note: None,
    }
}

/// What one status reads.
struct Read<'a> {
    request: &'a Request,
    stop: &'a Stop,
}

impl Read<'_> {
    fn run(&self, meter: &mut Meter<'_>) -> Result<GitStatus, GitError> {
        let layout = &self.request.layout;
        let real = self.request.folder.as_path();

        let Some(repository) = open::open(&layout.dot_git)? else {
            return Ok(not_a_repository());
        };
        // What gix opened must be what was looked at: the same git folder, common folder and work tree.
        if !layout.is_opened(&repository) {
            tracing::debug!("git status: gix opened another place than the one that was looked at");

            return Ok(not_a_repository());
        }
        // A bare repository has no work tree, so none of its folders is in one.
        let Some(workdir) = repository.workdir() else {
            return Ok(not_a_repository());
        };
        let Some(prefix) = prefix_of(real, workdir) else {
            return Ok(not_a_repository());
        };
        let mut marks = Marks::new(&prefix);

        let head = staged::head(&repository, self.stop)?;
        let index = self.index(&repository)?;
        let state: &gix::index::State = &index;

        // The entries of the index inside the folder are what is checked; a folder with more than the cap is too big. Each of
        // the three steps (the index range, the walk of `HEAD` and the work tree) has the whole cap for itself, so that a
        // repository of 100,000 files is not too big for its root only because it has two things to count for each file; the
        // time limit bounds the whole.
        let range = state.prefixed_entries_range(marks.prefix().as_bstr());
        if !meter.spend(range.map_or(0, |range| range.len())) {
            return Err(GitError::TooBig);
        }

        // The rules of the repository itself were counted by the parent; the ones of the work tree on the way from its root to
        // the folder are counted here, and the walk and the comparison count the rest in the same budget.
        let budget = Budget::new(workdir, &repository, state, &prefix, layout.info_rules, self.stop);
        match chain(&budget, &prefix) {
            Charged::Fine => {}
            Charged::Odd => return Ok(not_a_repository()),
            Charged::Large | Charged::Over => return Err(GitError::TooBig),
        }

        self.compare(&repository, state, &head, &budget, &mut marks, meter)?;
        let marks = marks.finish(real, meter).map_err(|error| match self.stop.reason() {
            Some(reason) => reason,
            None => GitError::Folder(error.kind()),
        })?;

        Ok(GitStatus {
            repo: true,
            marks,
            note: None,
        })
    }

    fn compare(
        &self,
        repository: &gix::Repository,
        state: &gix::index::State,
        head: &Head,
        budget: &Budget,
        marks: &mut Marks,
        meter: &mut Meter<'_>,
    ) -> Result<(), GitError> {
        let layout = &self.request.layout;

        // `:(literal)` keeps the characters of a folder's name (`*`, `[`, `?`) from being read as a pattern.
        let prefix = marks.prefix().trim_end_with(|c| c == '/').to_vec();
        let patterns: Vec<BString> = if prefix.is_empty() {
            Vec::new()
        } else {
            let mut pattern = b":(literal)".to_vec();
            pattern.extend_from_slice(&prefix);

            vec![pattern.into()]
        };
        let mut run = Run {
            repository,
            workdir: &layout.workdir,
            folder: self.request.folder.as_path(),
            state,
            head,
            patterns,
            max_compare_bytes: self.request.limits.compare_bytes,
            budget,
            marks,
            stop: self.stop,
            meter,
        };

        run.meter.next_phase();
        run.staged()?;
        run.meter.next_phase();
        run.changed()?;
        run.walked()?;
        run.ignored_children()
    }

    /// The decoded index of the repository. It is decoded for this status alone: the helper is a process of its own for each
    /// status, so there is nothing to keep it in between.
    fn index(&self, repository: &gix::Repository) -> Result<gix::index::File, GitError> {
        let path = repository.index_path();

        match fs::metadata(&path) {
            Ok(_) => repository.open_index().map_err(|error| self.stop.explain(&error)),
            // No index yet (nothing was added): an empty one.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let state = gix::index::State::new(repository.object_hash());

                Ok(gix::index::File::from_state(state, path))
            }
            Err(error) => Err(GitError::Folder(error.kind())),
        }
    }
}

/// Counts the rules of the work tree's root and of each folder from it down to the folder `prefix` (repository-relative, `/`
/// separated) in `budget`: gix reads each of them for any entry of the folder. `Odd` is a source that must not be opened, and
/// `Large` and `Over` are too much.
fn chain(budget: &Budget, prefix: &str) -> Charged {
    let mut folder = String::new();
    let names = std::iter::once("").chain(prefix.split('/').filter(|name| !name.is_empty()));

    for name in names {
        if !name.is_empty() {
            if !folder.is_empty() {
                folder.push('/');
            }
            folder.push_str(name);
        }

        match budget.charge(folder.as_bytes().as_bstr()) {
            Charged::Fine => {}
            other => return other,
        }
    }

    Charged::Fine
}

/// The three comparisons of one status, all limited to the same pathspec and merging into the same marks.
struct Run<'a, 'm> {
    repository: &'a gix::Repository,
    workdir: &'a Path,
    /// The real path of the folder the status is for.
    folder: &'a Path,
    state: &'a gix::index::State,
    head: &'a Head,
    patterns: Vec<BString>,
    max_compare_bytes: u64,
    /// The rules budget of the status.
    budget: &'a Budget,
    marks: &'a mut Marks,
    stop: &'a Stop,
    meter: &'a mut Meter<'m>,
}

impl Run<'_, '_> {
    /// The index against `HEAD`: the files that are staged.
    fn staged(&mut self) -> Result<(), GitError> {
        if self.stop.is_stopped() {
            return Ok(());
        }

        staged::staged(
            self.repository,
            self.state,
            &self.head.tree,
            self.marks,
            self.stop,
            self.meter,
        )
    }

    /// The work tree against the index, for the tracked files: the files that are modified or in conflict.
    fn changed(&mut self) -> Result<(), GitError> {
        if self.stop.is_stopped() {
            return Ok(());
        }

        let options = gix::status::index_worktree::Options {
            // No directory walk here (it is `walked`), no rename tracking, one thread.
            dirwalk_options: None,
            rewrites: None,
            sorting: None,
            thread_limit: Some(1),
        };
        let mut visitor = Changes {
            marks: &mut *self.marks,
            meter: &mut *self.meter,
        };

        self.repository
            .index_worktree_status(
                self.state,
                &self.patterns,
                &mut visitor,
                BoundedCompare {
                    root: self.workdir,
                    state: self.state,
                    max_bytes: self.max_compare_bytes,
                    budget: self.budget,
                    stop: self.stop,
                },
                NoSubmodules,
                &mut gix::progress::Discard,
                &self.stop.interrupt,
                options,
            )
            .map_err(|error| self.stop.explain(&error))?;

        Ok(())
    }

    /// The directory walk: the files and folders that are untracked or ignored.
    fn walked(&mut self) -> Result<(), GitError> {
        if self.stop.is_stopped() {
            return Ok(());
        }

        let explain = |error: gix::Error| self.stop.explain(&error);
        // A folder whose files are all of one kind is one entry, not one for each file. Nested repositories are walked as the
        // folders they are, and not looked at as repositories: with `recurse_repositories` off the walk checks every untracked
        // folder for a `.git`, which follows a `gitdir:` pointer and reads its `HEAD` before anything of it was looked at.
        // Their `.git` is pruned as always, and a submodule (a gitlink of the index) is left out by the `Walker`.
        let options = self
            .repository
            .dirwalk_options()
            .map_err(explain)?
            .emit_untracked(EmissionMode::CollapseDirectory)
            .emit_ignored(Some(EmissionMode::CollapseDirectory))
            .emit_tracked(false)
            .emit_pruned(false)
            .emit_empty_directories(false)
            .recurse_repositories(true);
        let mut walker = Walker {
            marks: &mut *self.marks,
            meter: &mut *self.meter,
            budget: self.budget,
        };

        self.repository
            .dirwalk(self.state, &self.patterns, &self.stop.interrupt, options, &mut walker)
            .map_err(explain)?;

        Ok(())
    }

    /// When the folder is untracked as a whole, the walk says so with one entry that holds everything below it, and says nothing
    /// of the ignored files in it: see [`Quiet`].
    fn ignored_children(&mut self) -> Result<(), GitError> {
        if self.stop.is_stopped() || !self.marks.is_untracked_as_a_whole() {
            return Ok(());
        }

        Quiet {
            repository: self.repository,
            state: self.state,
            folder: self.folder,
            budget: self.budget,
            marks: &mut *self.marks,
            stop: self.stop,
            meter: &mut *self.meter,
        }
        .run()
    }
}

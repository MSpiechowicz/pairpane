//! The status of a folder: find the repository, then merge the three comparisons limited to the folder into marks.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gix::bstr::{BString, ByteSlice};
use gix::dir::walk::EmissionMode;

use super::cache::{self, Cache, Key};
use super::compare::{BoundedCompare, NoSubmodules};
use super::files;
use super::marks::Marks;
use super::path::prefix_of;
use super::staged::{self, Head};
use super::visit::{Changes, Walker};
use super::watch::{Meter, Stop, guarded};
use super::{GitError, Limits, locate, open};
use crate::model::GitStatus;

/// The status of the direct children of `folder`; see [`super::status`].
pub(super) fn status(folder: &Path, limits: Limits, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    // The user's home folder is where the search for a repository ends (it is the last folder looked in).
    let home = std::env::home_dir().and_then(|home| gix::path::realpath(home).ok());

    status_with(folder, home, limits, cancel, &cache::SHARED)
}

/// Like [`status`], with the folder the search for a repository must not go above given (`None`: the filesystem root).
#[cfg(test)]
pub(super) fn status_below(
    folder: &Path,
    ceiling: Option<PathBuf>,
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<GitStatus, GitError> {
    status_with(folder, ceiling, limits, cancel, &cache::SHARED)
}

/// Like [`status_below`], with the cache of decoded indexes given.
pub(super) fn status_with(
    folder: &Path,
    ceiling: Option<PathBuf>,
    limits: Limits,
    cancel: &AtomicBool,
    cache: &Cache,
) -> Result<GitStatus, GitError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    // Links are resolved first: the work tree has to be found as a parent of where the folder really is.
    let real = gix::path::realpath(folder).map_err(|error| {
        tracing::debug!(%error, "git status: the folder cannot be resolved");

        GitError::Folder(if error.is_not_found() {
            io::ErrorKind::NotFound
        } else {
            io::ErrorKind::Other
        })
    })?;
    match fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(GitError::Folder(io::ErrorKind::NotADirectory)),
        Err(error) => return Err(GitError::Folder(error.kind())),
    }
    let stop = Stop::default();

    let result = guarded(&stop, limits.time, cancel, || {
        let mut meter = Meter::new(&stop, limits.entries);
        let read = Read {
            real: &real,
            ceiling: ceiling.as_deref(),
            limits,
            cache,
            stop: &stop,
        };

        read.run(&mut meter)
    });

    // A flag that was raised is the answer, whatever error the interrupted read ended in.
    match stop.reason() {
        Some(reason) => Err(reason),
        None => result,
    }
}

fn not_a_repository() -> GitStatus {
    GitStatus {
        repo: false,
        marks: Vec::new(),
    }
}

/// What one status reads.
struct Read<'a> {
    real: &'a Path,
    ceiling: Option<&'a Path>,
    limits: Limits,
    cache: &'a Cache,
    stop: &'a Stop,
}

impl Read<'_> {
    fn run(&self, meter: &mut Meter<'_>) -> Result<GitStatus, GitError> {
        // The files of the repository are looked at, and what they point to, before gix reads one.
        let Some(dot_git) = locate::find(self.real, self.ceiling)? else {
            return Ok(not_a_repository());
        };
        let Some(repository) = open::open(&dot_git)? else {
            return Ok(not_a_repository());
        };
        // A bare repository has no work tree, so none of its folders is in one.
        let Some(workdir) = repository.workdir() else {
            return Ok(not_a_repository());
        };
        let Some(prefix) = prefix_of(self.real, workdir) else {
            return Ok(not_a_repository());
        };
        if rules_files_are_risky(workdir, &prefix) {
            return Ok(not_a_repository());
        }
        let mut marks = Marks::new(&prefix);

        self.compare(&repository, workdir, &prefix, &mut marks, meter)?;
        let marks = marks
            .finish(self.real, meter)
            .map_err(|error| match self.stop.reason() {
                Some(reason) => reason,
                None => GitError::Folder(error.kind()),
            })?;

        Ok(GitStatus { repo: true, marks })
    }

    fn compare(
        &self,
        repository: &gix::Repository,
        workdir: &Path,
        prefix: &str,
        marks: &mut Marks,
        meter: &mut Meter<'_>,
    ) -> Result<(), GitError> {
        let head = staged::head(repository, self.stop)?;
        let index = self.index(repository, &head)?;
        let state: &gix::index::State = &index;

        // The entries of the index inside the folder are what is checked; a folder with more than the cap is too big.
        let range = state.prefixed_entries_range(marks.prefix().as_bstr());
        if !meter.spend(range.map_or(0, |range| range.len())) {
            return Ok(());
        }

        // `:(literal)` keeps the characters of a folder's name (`*`, `[`, `?`) from being read as a pattern.
        let patterns: Vec<BString> = if prefix.is_empty() {
            Vec::new()
        } else {
            vec![format!(":(literal){prefix}").into()]
        };
        let mut run = Run {
            repository,
            workdir,
            state,
            head: &head,
            patterns,
            max_compare_bytes: self.limits.compare_bytes,
            marks,
            stop: self.stop,
            meter,
        };

        run.staged()?;
        run.changed()?;
        run.walked()
    }

    /// The decoded index of the repository, from the cache when it was decoded lately.
    fn index(&self, repository: &gix::Repository, head: &Head) -> Result<Arc<gix::index::File>, GitError> {
        let path = repository.index_path();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            // No index yet (nothing was added): an empty one.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let state = gix::index::State::new(repository.object_hash());

                return Ok(Arc::new(gix::index::File::from_state(state, path)));
            }
            Err(error) => return Err(GitError::Folder(error.kind())),
        };
        let key = Key {
            git_dir: repository.git_dir().to_path_buf(),
            head: head.commit,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        };

        self.cache.get(key, || {
            repository.open_index().map_err(|error| self.stop.explain(&error))
        })
    }
}

/// Whether a rules file (`.gitignore`, `.gitattributes`) of the work tree's root, of the folder or of a folder between them
/// is one gix would be held up by (see [`files::risky_rules_file`]): gix reads each of them for any entry of the folder.
fn rules_files_are_risky(workdir: &Path, prefix: &str) -> bool {
    let mut folder = workdir.to_path_buf();
    let names = std::iter::once("").chain(prefix.split('/').filter(|name| !name.is_empty()));

    for name in names {
        if !name.is_empty() {
            folder.push(name);
        }

        if [".gitignore", ".gitattributes"]
            .iter()
            .any(|file| files::risky_rules_file(&folder.join(file)))
        {
            return true;
        }
    }

    false
}

/// The three comparisons of one status, all limited to the same pathspec and merging into the same marks.
struct Run<'a, 'm> {
    repository: &'a gix::Repository,
    workdir: &'a Path,
    state: &'a gix::index::State,
    head: &'a Head,
    patterns: Vec<BString>,
    max_compare_bytes: u64,
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
                BoundedCompare::new(self.workdir, self.state, self.max_compare_bytes),
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
        // A folder whose files are all of one kind is one entry, not one for each file. Nested repositories are not entered.
        let options = self
            .repository
            .dirwalk_options()
            .map_err(explain)?
            .emit_untracked(EmissionMode::CollapseDirectory)
            .emit_ignored(Some(EmissionMode::CollapseDirectory))
            .emit_tracked(false)
            .emit_pruned(false)
            .emit_empty_directories(false)
            .recurse_repositories(false);
        let mut walker = Walker {
            marks: &mut *self.marks,
            meter: &mut *self.meter,
            workdir: self.workdir,
        };

        self.repository
            .dirwalk(self.state, &self.patterns, &self.stop.interrupt, options, &mut walker)
            .map_err(explain)?;

        Ok(())
    }
}

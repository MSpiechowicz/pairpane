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
use super::files::{self, Rules};
use super::marks::Marks;
use super::path::prefix_of;
use super::staged::{self, Head};
use super::visit::{Changes, Walker};
use super::watch::{Meter, Stop, guarded};
use super::{GitError, Limits, locate, open};
use crate::model::{GitMark, GitStatus};

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
        let Some(layout) = locate::find(self.real, self.ceiling)? else {
            return Ok(not_a_repository());
        };
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
        let Some(prefix) = prefix_of(self.real, workdir) else {
            return Ok(not_a_repository());
        };
        let Some(rules) = rules_of_the_chain(workdir, &prefix) else {
            return Ok(not_a_repository());
        };
        if rules > files::RULES_TOTAL_BYTES {
            return Err(GitError::TooBig);
        }
        let mut marks = Marks::new(&prefix);

        self.compare(&repository, workdir, &prefix, rules, &mut marks, meter)?;
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
        rules: u64,
        marks: &mut Marks,
        meter: &mut Meter<'_>,
    ) -> Result<(), GitError> {
        let head = staged::head(repository, self.stop)?;
        let index = self.index(repository, &head)?;
        let state: &gix::index::State = &index;

        // The entries of the index inside the folder are what is checked; a folder with more than the cap is too big. Each of
        // the three steps (the index range, the walk of `HEAD` and the work tree) has the whole cap for itself, so that a
        // repository of 100,000 files is not too big for its root only because it has two things to count for each file; the
        // time limit bounds the whole.
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
            folder: self.real,
            state,
            head: &head,
            patterns,
            max_compare_bytes: self.limits.compare_bytes,
            rules,
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
        let key = Key::of(repository.git_dir().to_path_buf(), head.commit, &metadata);

        self.cache.get(key, || {
            repository.open_index().map_err(|error| self.stop.explain(&error))
        })
    }
}

/// How many bytes the rules files (`.gitignore`, `.gitattributes`) of the work tree's root, of the folder and of a folder
/// between them add up to, or `None` when one is a file gix would be held up by (see [`files::rules_file`]): gix reads each of
/// them for any entry of the folder.
fn rules_of_the_chain(workdir: &Path, prefix: &str) -> Option<u64> {
    let mut folder = workdir.to_path_buf();
    let names = std::iter::once("").chain(prefix.split('/').filter(|name| !name.is_empty()));
    let mut total = 0_u64;

    for name in names {
        if !name.is_empty() {
            folder.push(name);
        }

        for file in [".gitignore", ".gitattributes"] {
            match files::rules_file(&folder.join(file)) {
                Rules::Risky => return None,
                Rules::Bytes(bytes) => total += bytes,
            }
        }
    }

    Some(total)
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
    /// The bytes of the rules files of the folder and its parents, which the walk adds to.
    rules: u64,
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
            workdir: self.workdir,
            rules: self.rules,
        };

        self.repository
            .dirwalk(self.state, &self.patterns, &self.stop.interrupt, options, &mut walker)
            .map_err(explain)?;

        Ok(())
    }

    /// When the folder is untracked as a whole, the walk says so with one entry that holds everything below it, and says nothing
    /// of the ignored files in it. What the direct children are is then asked of the ignore rules (the same ones that the walk
    /// used), and the ones they leave out are ignored, not untracked.
    fn ignored_children(&mut self) -> Result<(), GitError> {
        if self.stop.is_stopped() || !self.marks.is_untracked_as_a_whole() {
            return Ok(());
        }

        let explain = |error: gix::Error| self.stop.explain(&error);
        let mut excludes = self
            .repository
            .excludes(
                self.state,
                None,
                gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(explain)?;

        for entry in fs::read_dir(self.folder).map_err(|error| GitError::Folder(error.kind()))? {
            let entry = entry.map_err(|error| GitError::Folder(error.kind()))?;
            if !self.meter.spend(1) {
                return Ok(());
            }

            let name = entry.file_name();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let Ok(name) = gix::path::os_str_into_bstr(&name) else {
                continue;
            };
            if name == ".git" {
                continue;
            }
            let mode = if file_type.is_dir() {
                gix::index::entry::Mode::DIR
            } else if file_type.is_symlink() {
                gix::index::entry::Mode::SYMLINK
            } else {
                gix::index::entry::Mode::FILE
            };

            let mut path = self.marks.prefix().to_vec();
            path.extend_from_slice(name);
            let path = BString::from(path);
            let ignored = excludes
                .at_entry(path.as_bstr(), Some(mode))
                .map_err(explain)?
                .is_excluded();
            if ignored {
                self.marks.note(path.as_bstr(), GitMark::Ignored);
            }
        }

        Ok(())
    }
}

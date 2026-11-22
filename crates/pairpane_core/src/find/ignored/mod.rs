//! Leaving out what `.ignore`, `.gitignore` and `.git/info/exclude` say to leave out, read by this crate and not by
//! `ignore`'s walker: that one opens an ignore file by itself, following a link, waiting on a pipe and reading a line of
//! any length, which a hostile folder turns into a search that cannot be stopped.
//!
//! Each file is opened with [`open_listed`] (below the folder searched, no link followed on the way or at the file, a
//! pipe never waited on, regular files only) and read within [`MAX_BYTES`], [`MAX_LINES`] and [`MAX_LINE`]. A file that
//! is over a limit, cannot be read or is not a regular file is left out and counted as skipped. A `.git` that is a file
//! (a worktree or a submodule, which points somewhere else) marks a repository and is never read.
//!
//! The rules are ripgrep's: `.gitignore` and `.git/info/exclude` count inside a git repository only (a folder with a
//! `.git`, found by looking at the name without following a link, at or above the folder searched), `.ignore` anywhere;
//! the files of a deeper folder come before those of a shallower one, and `.ignore` comes before `.gitignore`, which
//! comes before the exclude file.

mod budget;
mod cache;
mod parse;

use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::BuildHasher;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use ignore::Match;
use ignore::gitignore::{Gitignore, Glob};

use crate::ops::open_listed;

use budget::{Budget, Charge, Turns};
use cache::{Held, Kept, Slot, forget_older_half, forget_to_free};
use parse::{MAX_BYTES, count_lines, listed, parse, read_limited};

/// The most lines of an ignore file.
pub(super) const MAX_LINES: usize = 2_000;

/// The longest line of an ignore file, in bytes.
pub(super) const MAX_LINE: usize = 4096;

/// The most lines of ignore files that are in use at once, in all the folders of a search: each line is compiled into
/// rules that take far more memory than the text does, so what a hostile tree can make the search hold is bounded here.
pub(super) const HELD_LINES: usize = 50_000;

/// The most bytes of ignore files that are in use at once.
pub(super) const HELD_BYTES: usize = 8 * 1024 * 1024;

/// The most folders remembered at once (those with rules and those without); past it the older half of those that nothing
/// uses is dropped and read again when needed.
pub(super) const KEPT: usize = 4096;

/// An ignore file with more lines than this waits for a turn to be compiled.
const BIG: usize = 200;

/// How many folders the walk threads hold on to, one each, while they test the entries of a folder.
const PINS: usize = 16;

/// The rules of one ignore file, with the share of the budget they hold until they are dropped.
struct Rules {
    gitignore: Gitignore,
    charge: Charge,
}

/// The rules that apply in a folder: its own files and, through `parent`, those of the folders above. A folder that has no
/// rules and is not a repository has no layer: that of the nearest folder above that has one stands for it, so a chain of
/// layers is as long as the number of folders with rules, not as deep as the tree.
struct Layer {
    parent: Option<Arc<Layer>>,
    dir: PathBuf,
    ignore: Option<Rules>,
    git: Option<Rules>,
    exclude: Option<Rules>,
    /// The folder holds a `.git`.
    repository: bool,
    /// The folder is in a git repository: it holds a `.git` or a folder above does.
    in_repository: bool,
    /// When the folder, or one below it, was last asked for.
    used: AtomicU64,
}

/// The numbers the threads that look rules up are given in turn, to tell them apart.
static THREADS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static THREAD: usize = THREADS.fetch_add(1, Ordering::Relaxed);
}

/// What `.ignore` and git ignore files say about the paths of one search.
///
/// The rules of a folder live as long as something uses them: the folder's place in `kept`, a folder below that has them
/// for a parent, a walk thread's pin, or a lookup. Only rules that nothing but `kept` holds are ever forgotten, so what
/// is forgotten is freed at once, and the rules of a folder above one in use are never read a second time. The rules of
/// the folder searched, and of those above it, are held by `top` for the whole search.
///
/// A folder without rules has a place in `kept` that only points (weakly) to the layer that stands for it, so the places
/// of the folders of a deep tree with no ignore files take next to nothing and are forgotten freely, whereas a folder
/// with rules is only forgotten when nothing uses it.
///
/// What cannot be promised: a folder's rules that nothing uses at the moment (a walk thread looks at only the last folder
/// it was given) can be forgotten for room and read again later, and if the room is gone by then the file is left out and
/// counted, as is any file that does not fit; a file left out may fit, and apply, when it is read again.
pub(crate) struct Ignores {
    root: PathBuf,
    top: Arc<Layer>,
    kept: RwLock<Kept>,
    /// One folder for each walk thread (shared by the threads that hash to the same place): the rules of the folder whose
    /// entries a thread is testing, and so of the folders above it, are not forgotten for room for a file below.
    pins: Vec<Mutex<Option<Arc<Layer>>>>,
    clock: AtomicU64,
    budget: Arc<Budget>,
    turns: Turns,
    /// The files already counted as left out (by a hash of their path), so that reading a folder again counts nothing.
    counted: Mutex<HashSet<u64>>,
    hasher: RandomState,
    skipped: Arc<AtomicU64>,
    /// How many files were compiled.
    #[cfg(test)]
    compiled: AtomicU64,
}

impl Ignores {
    /// The rules for a search of `root`, with those of the folders above it. Each file that is left out is added to
    /// `skipped`, once.
    pub(crate) fn new(root: &Path, skipped: Arc<AtomicU64>) -> Self {
        let mut ignores = Self {
            root: root.to_owned(),
            top: Arc::new(Layer::empty(root)),
            kept: RwLock::new(Kept::new()),
            pins: (0..PINS).map(|_| Mutex::new(None)).collect(),
            clock: AtomicU64::new(0),
            budget: Arc::new(Budget::default()),
            turns: Turns::new(),
            counted: Mutex::new(HashSet::new()),
            hasher: RandomState::new(),
            skipped,
            #[cfg(test)]
            compiled: AtomicU64::new(0),
        };

        let mut above: Vec<&Path> = root.ancestors().skip(1).collect();
        above.reverse();

        let mut layer: Option<Arc<Layer>> = None;
        for dir in above.into_iter().chain([root]) {
            if let Some(read) = ignores.read(layer.as_ref(), dir) {
                layer = Some(Arc::new(read));
            }
        }
        if let Some(layer) = layer {
            ignores.top = layer;
        }

        ignores
    }

    /// How many ignore files were compiled.
    #[cfg(test)]
    pub(super) fn compiles(&self) -> u64 {
        self.compiled.load(Ordering::Relaxed)
    }

    /// The most lines and bytes of ignore files that were held at once.
    #[cfg(test)]
    pub(super) fn peak(&self) -> (usize, usize) {
        self.budget.peak()
    }

    /// The most folders that were remembered at once.
    #[cfg(test)]
    pub(super) fn remembered_peak(&self) -> usize {
        self.kept.read().map_or(0, |kept| kept.peak)
    }

    /// Whether the rules leave out `path` (below the folder searched), a folder when `is_dir`.
    pub(crate) fn leaves_out(&self, path: &Path, is_dir: bool) -> bool {
        let Some(dir) = path.parent() else {
            return false;
        };

        let layer = self.layer(dir);
        let mut ignore = Match::None;
        let mut git = Match::None;
        let mut exclude = Match::None;
        let mut past_repository = false;

        let mut next = Some(&layer);
        while let Some(layer) = next {
            let Ok(relative) = path.strip_prefix(&layer.dir) else {
                break;
            };

            if ignore.is_none() {
                ignore = matched(&layer.ignore, relative, is_dir);
            }

            // Git's files are only read inside a repository, and the search for them ends at its top folder.
            if !past_repository {
                if git.is_none() {
                    git = matched(&layer.git, relative, is_dir);
                }
                if exclude.is_none() {
                    exclude = matched(&layer.exclude, relative, is_dir);
                }
            }

            past_repository = past_repository || layer.repository;
            next = layer.parent.as_ref();
        }

        ignore.or(git).or(exclude).is_ignore()
    }

    /// The rules of the folder `dir`, read when they are not remembered yet. The folder and those above it count as used
    /// now, and the folder is the one this thread holds on to.
    fn layer(&self, dir: &Path) -> Arc<Layer> {
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        let layer = self.find(dir, tick);

        let mut next = Some(&layer);
        while let Some(above) = next {
            above.used.store(tick, Ordering::Relaxed);
            next = above.parent.as_ref();
        }

        self.pin(&layer);

        layer
    }

    /// The rules that apply in the folder `dir`, asked for at `tick`. A folder is read once at a time: a thread that finds
    /// it being read waits for the result.
    fn find(&self, dir: &Path, tick: u64) -> Arc<Layer> {
        if dir == self.root || !dir.starts_with(&self.root) {
            return Arc::clone(&self.top);
        }

        let slot = self.slot(dir);
        slot.used.store(tick, Ordering::Relaxed);

        let mut held = slot.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(layer) = held.layer() {
            return layer;
        }

        let parent = dir
            .parent()
            .map_or_else(|| Arc::clone(&self.top), |above| self.find(above, tick));

        // A folder with no rules of its own is the layer above.
        let Some(layer) = self.read(Some(&parent), dir) else {
            *held = Held::Shared(Arc::downgrade(&parent));

            return parent;
        };

        let layer = Arc::new(layer);
        *held = Held::Own(Arc::clone(&layer));

        layer
    }

    /// The place of the folder `dir`, made when there is none. Looking one up only reads the map; making one forgets the
    /// older half of the idle ones first when the map is full.
    fn slot(&self, dir: &Path) -> Arc<Slot> {
        if let Ok(kept) = self.kept.read()
            && let Some(slot) = kept.slots.get(dir)
        {
            return Arc::clone(slot);
        }

        let Ok(mut kept) = self.kept.write() else {
            return Arc::new(Slot::new());
        };
        if let Some(slot) = kept.slots.get(dir) {
            return Arc::clone(slot);
        }

        let gone = if kept.slots.len() >= kept.trim_at {
            let gone = forget_older_half(&mut kept.slots);

            // What is in use cannot be forgotten: wait for some more places before looking again.
            kept.trim_at = KEPT.max(kept.slots.len() + KEPT / 8);

            gone
        } else {
            Vec::new()
        };

        let slot = Arc::new(Slot::new());
        kept.slots.insert(dir.to_owned(), Arc::clone(&slot));
        #[cfg(test)]
        {
            kept.peak = kept.peak.max(kept.slots.len());
        }

        // The rules are freed outside the lock.
        drop(kept);
        drop(gone);

        slot
    }

    /// Makes `layer` the folder this thread holds on to.
    fn pin(&self, layer: &Arc<Layer>) {
        let place = THREAD.with(|place| *place) % self.pins.len();
        let mut pinned = self.pins[place].lock().unwrap_or_else(PoisonError::into_inner);

        if pinned.as_ref().is_some_and(|pinned| Arc::ptr_eq(pinned, layer)) {
            return;
        }

        let before = pinned.replace(Arc::clone(layer));

        drop(pinned);
        drop(before);
    }

    /// A share of the budget for a file. When there is none, the rules that nothing uses are forgotten, the ones used
    /// longest ago first and no more than it takes, but only when they hold enough to make room: `None` otherwise, and
    /// the file is left out. The rules of the folders in use (those above the file's own among them) are never given up.
    fn reserve(&self, lines: usize, bytes: usize) -> Option<Charge> {
        loop {
            let (more_lines, more_bytes) = match self.budget.take(lines, bytes) {
                Ok(charge) => return Some(charge),
                Err(more) => more,
            };
            let gone = {
                let mut kept = self.kept.write().ok()?;

                forget_to_free(&mut kept.slots, more_lines, more_bytes)
            };

            if gone.is_empty() {
                return None;
            }

            drop(gone);
        }
    }

    /// The layer of the folder `dir`, below `parent`; `None` when the folder has no rules and is not a repository, and
    /// there is a `parent` to stand for it.
    fn read(&self, parent: Option<&Arc<Layer>>, dir: &Path) -> Option<Layer> {
        let git_entry = fs::symlink_metadata(dir.join(".git")).ok();
        let repository = git_entry.is_some();
        let in_repository = repository || parent.is_some_and(|parent| parent.in_repository);

        let ignore = self.file(dir, ".ignore");
        let git = if in_repository {
            self.file(dir, ".gitignore")
        } else {
            None
        };

        // Only a `.git` folder holds an exclude file; a `.git` file points elsewhere and is not followed.
        let exclude = if git_entry.is_some_and(|entry| entry.is_dir()) {
            self.file(dir, ".git/info/exclude")
        } else {
            None
        };

        if parent.is_some() && ignore.is_none() && git.is_none() && exclude.is_none() && !repository {
            return None;
        }

        Some(Layer {
            parent: parent.cloned(),
            dir: dir.to_owned(),
            ignore,
            git,
            exclude,
            repository,
            in_repository,
            used: AtomicU64::new(self.clock.fetch_add(1, Ordering::Relaxed)),
        })
    }

    /// The rules in the file `name` (names joined with `/`) of the folder `dir`: `None` when there is none, when it holds
    /// no rule, or when it cannot be used (which is counted).
    fn file(&self, dir: &Path, name: &str) -> Option<Rules> {
        // Most folders have none: one look at the name tells, where opening it step by step costs a call for each folder
        // on the way (and what is opened is checked as before).
        if fs::symlink_metadata(dir.join(name)).is_err_and(|error| error.kind() == io::ErrorKind::NotFound) {
            return None;
        }

        let opened = match dir.strip_prefix(&self.root) {
            Ok(below) => match listed(below, name) {
                Some(listed) => open_listed(&self.root, &listed),
                None => return self.left_out(dir, name),
            },
            Err(_) => open_listed(dir, name),
        };

        let (file, metadata) = match opened {
            Ok(opened) => opened,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => return self.left_out(dir, name),
        };

        if metadata.len() > MAX_BYTES {
            return self.left_out(dir, name);
        }

        let Some(text) = read_limited(file) else {
            return self.left_out(dir, name);
        };
        let bytes = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&text);

        let Some(lines) = count_lines(bytes) else {
            return self.left_out(dir, name);
        };
        let Some(charge) = self.reserve(lines, bytes.len()) else {
            return self.left_out(dir, name);
        };

        // The rules of a big file take much memory while they are built: only a few are built at a time.
        let _turn = (lines > BIG).then(|| self.turns.wait());

        #[cfg(test)]
        self.compiled.fetch_add(1, Ordering::Relaxed);

        match parse(dir, bytes) {
            Some(gitignore) if gitignore.is_empty() => None,
            Some(gitignore) => Some(Rules { gitignore, charge }),
            None => self.left_out(dir, name),
        }
    }

    /// Counts the file `name` of `dir` as left out, the first time only: a folder read again (after it was forgotten, or
    /// by two threads) counts nothing more.
    fn left_out(&self, dir: &Path, name: &str) -> Option<Rules> {
        let key = self.hasher.hash_one((dir, name));
        let first = self.counted.lock().unwrap_or_else(PoisonError::into_inner).insert(key);

        if first {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }

        None
    }
}

impl Layer {
    fn empty(dir: &Path) -> Self {
        Self {
            parent: None,
            dir: dir.to_owned(),
            ignore: None,
            git: None,
            exclude: None,
            repository: false,
            in_repository: false,
            used: AtomicU64::new(0),
        }
    }

    /// The lines and bytes of the budget the rules of this folder hold, not those of the folders above.
    fn charged(&self) -> (usize, usize) {
        [&self.ignore, &self.git, &self.exclude]
            .into_iter()
            .flatten()
            .fold((0, 0), |(lines, bytes), rules| {
                (lines + rules.charge.lines, bytes + rules.charge.bytes)
            })
    }
}

fn matched<'a>(rules: &'a Option<Rules>, relative: &Path, is_dir: bool) -> Match<&'a Glob> {
    match rules {
        Some(rules) => rules.gitignore.matched(relative, is_dir),
        None => Match::None,
    }
}

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
mod layer;
mod parse;
mod read;

use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use ignore::Match;

use budget::{Budget, Charge, Turns};
use cache::{Held, Kept, Slot, forget_down_to, forget_to_free};
use layer::{Layer, matched};

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
/// uses is dropped, again and again until the map is below the bound, and read again when needed.
pub(super) const KEPT: usize = 4096;

/// What a layer of folder rules costs the budget besides its path, in bytes: the layer, its place and its entry in the map.
/// With this, the bytes of an ignore file and the paths, no more layers than `HELD_BYTES / LAYER_BYTES` are held at once,
/// whatever they hold.
const LAYER_BYTES: usize = 1024;

/// How many bytes more than a layer needs are forgotten to make room for it, when there is that much to forget: making
/// room costs a look at every remembered folder, and layers come one after another down a deep tree.
const ROOM: usize = HELD_BYTES / 16;

/// How many lookups are made before the memory is looked at again for room for a layer, after it was found in use.
const PATIENCE: u64 = 256;

/// An ignore file with more lines than this waits for a turn to be compiled.
const BIG: usize = 200;

/// How many folders the walk threads hold on to, one each, while they test the entries of a folder.
const PINS: usize = 16;

/// The numbers the threads that look rules up are given in turn, to tell them apart.
static THREADS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static THREAD: usize = THREADS.fetch_add(1, Ordering::Relaxed);
}

/// What `.ignore` and git ignore files say about the paths of one search.
///
/// The rules of a folder live as long as something uses them: the folder's place in `kept`, a folder below that has them
/// for a parent, a walk thread's pin, or a lookup. Only rules that nothing but `kept` (and idle folders below) holds are
/// forgotten for room. The rules of the folder searched, and of those above it, are held by `top` for the whole search.
///
/// Every folder with rules, and every folder with a `.git` that is not below one that is only a `.git`, has a layer, and
/// every layer is charged to the budget: its rules and, besides, [`LAYER_BYTES`] and its path twice (in the layer and in
/// the map). So a tree with rules in every folder cannot make the search hold more than the budget either: the layers of
/// idle chains of folders are forgotten, the deepest first, when there is no room for a new one, and when there is nothing
/// to forget the folder shares the layer of the nearest one above that was kept, and its rules are counted as left out.
/// A `.git` is never lost with them: a folder with one, when its layer has no room, gets a boundary that holds nothing and
/// costs nothing, so the rules of the folders above still end there and the files of the folders below are still read as
/// those of a repository. A chain of `.git` folders is one layer, as the ones below the first add nothing.
///
/// A folder without a layer has a place in `kept` that only points (weakly) to the layer that stands for it, so the places
/// of the folders of a deep tree with no ignore files take next to nothing and are forgotten freely.
///
/// What cannot be promised: a folder's rules that nothing uses at the moment (a walk thread looks at only the last folder
/// it was given) can be forgotten for room and read again later, and if the room is gone by then the file is left out and
/// counted, as is any file that does not fit; a file left out may fit, and apply, when it is read again. A lookup can also
/// keep rules that were just forgotten, or a thread read a folder that another has just forgotten: such a copy is charged
/// to the budget like any other, so the bound holds, but the folder may be read, and compiled, more than once.
pub(crate) struct Ignores {
    root: PathBuf,
    top: Arc<Layer>,
    kept: RwLock<Kept>,
    /// One folder for each walk thread (shared by the threads that hash to the same place): the rules of the folder whose
    /// entries a thread is testing, and so of the folders above it, are not forgotten for room for a file below.
    pins: Vec<Mutex<Option<Arc<Layer>>>>,
    clock: AtomicU64,
    /// The time (of `clock`) before which no new layer is made, after one found no room and nothing to forget.
    starved_until: AtomicU64,
    budget: Arc<Budget>,
    turns: Turns,
    /// The files already counted as left out (by a hash of their path), so that reading a folder again counts nothing.
    counted: Mutex<HashSet<u64>>,
    hasher: RandomState,
    skipped: Arc<AtomicU64>,
    /// How many files were compiled.
    #[cfg(test)]
    compiled: AtomicU64,
    /// How many layers were refused for lack of room.
    #[cfg(test)]
    refused: AtomicU64,
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
            starved_until: AtomicU64::new(0),
            budget: Arc::new(Budget::default()),
            turns: Turns::new(),
            counted: Mutex::new(HashSet::new()),
            hasher: RandomState::new(),
            skipped,
            #[cfg(test)]
            compiled: AtomicU64::new(0),
            #[cfg(test)]
            refused: AtomicU64::new(0),
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

    /// How many layers were refused for lack of room.
    #[cfg(test)]
    pub(super) fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// Refuses every new layer until the clock reaches `until`, as a budget held in use by other folders does.
    #[cfg(test)]
    pub(super) fn starve(&self, until: u64) {
        self.starved_until.store(until, Ordering::Relaxed);
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

        // The rules are those of the folder searched and of the folders above and below it.
        if !path.starts_with(&self.root) {
            return false;
        }

        let layer = self.layer(dir);
        let mut ignore = Match::None;
        let mut git = Match::None;
        let mut exclude = Match::None;
        let mut past_repository = false;

        let mut next = Some(&layer);
        while let Some(layer) = next {
            // A layer with a `.git` and no rules has nothing to match: it only ends the reach of git's files. Every layer
            // is of a folder above the path (the path is below the folder searched), so the path is below it.
            if layer.has_rules() {
                let Ok(relative) = path.strip_prefix(&layer.dir) else {
                    debug_assert!(false, "{path:?} is not below {:?}", layer.dir);

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
            }

            past_repository = past_repository || layer.repository;
            next = layer.parent.as_ref();
        }

        ignore.or(git).or(exclude).is_ignore()
    }

    /// The rules of the folder `dir`, read when they are not remembered yet. The folder counts as used now (those above it
    /// are no older than it when the idle ones are told apart), and it is the one this thread holds on to.
    fn layer(&self, dir: &Path) -> Arc<Layer> {
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        let layer = self.find(dir, tick);

        layer.used.store(tick, Ordering::Relaxed);
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
    /// idle ones first when the map is full.
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
            let gone = forget_down_to(&mut kept.slots, KEPT - KEPT / 8);

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

    /// A share of the budget for a file or a layer. When there is none, the rules that nothing uses are forgotten, the ones
    /// used longest ago first and no more than it takes (and `extra` bytes more when there is that much to forget), but
    /// only when they hold enough to make room: `None` otherwise, and the file is left out. The rules of the folders in use
    /// (those above the file's own among them) are never given up.
    fn reserve(&self, lines: usize, bytes: usize, extra: usize) -> Option<Charge> {
        loop {
            let (more_lines, more_bytes) = match self.budget.take(lines, bytes) {
                Ok(charge) => return Some(charge),
                Err(more) => more,
            };
            let gone = {
                let mut kept = self.kept.write().ok()?;

                forget_to_free(&mut kept.slots, more_lines, more_bytes, extra)
            };

            if gone.is_empty() {
                return None;
            }

            drop(gone);
        }
    }

    /// A share of the budget for the layer of the folder `dir`. When nothing could be forgotten to make room, the layers
    /// asked for in the next [`PATIENCE`] lookups are refused without looking again: the memory is in use and looking at all
    /// of it for every folder of a deep tree would keep the threads waiting for one another.
    fn reserve_layer(&self, dir: &Path) -> Option<Charge> {
        if self.clock.load(Ordering::Relaxed) < self.starved_until.load(Ordering::Relaxed) {
            #[cfg(test)]
            self.refused.fetch_add(1, Ordering::Relaxed);

            return None;
        }

        let charge = self.reserve(0, LAYER_BYTES + 2 * dir.as_os_str().len(), ROOM);
        if charge.is_none() {
            #[cfg(test)]
            self.refused.fetch_add(1, Ordering::Relaxed);

            self.starved_until
                .store(self.clock.load(Ordering::Relaxed) + PATIENCE, Ordering::Relaxed);
        }

        charge
    }
}

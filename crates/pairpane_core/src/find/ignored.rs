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

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::BuildHasher;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder, Glob};

use crate::ops::open_listed;

/// The biggest ignore file that is read, in bytes.
const MAX_BYTES: u64 = 256 * 1024;

/// The most lines of an ignore file.
pub(super) const MAX_LINES: usize = 2_000;

/// The longest line of an ignore file, in bytes.
pub(super) const MAX_LINE: usize = 4096;

/// The most lines of ignore files that are in use at once, in all the folders of a search: each line is compiled into
/// rules that take far more memory than the text does, so what a hostile tree can make the search hold is bounded here.
pub(super) const HELD_LINES: usize = 50_000;

/// The most bytes of ignore files that are in use at once.
pub(super) const HELD_BYTES: usize = 8 * 1024 * 1024;

/// The most folders whose rules are remembered at once; past it the older half is dropped and read again when needed.
const KEPT: usize = 4096;

/// The rules of one ignore file, with the share of the budget they hold until they are dropped.
struct Rules {
    gitignore: Gitignore,
    _charge: Charge,
}

/// The rules that apply in one folder: its own files and, through `parent`, those of the folders above.
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
}

/// The lines and bytes of ignore files held by the rules that are alive: those remembered and those a lookup still uses.
#[derive(Default)]
struct Budget {
    used: Mutex<Used>,
}

#[derive(Default)]
struct Used {
    lines: usize,
    bytes: usize,
    /// The most that was held at once.
    #[cfg(test)]
    peak: (usize, usize),
}

/// A share of the [`Budget`], given back when the rules it was taken for are dropped.
struct Charge {
    budget: Arc<Budget>,
    lines: usize,
    bytes: usize,
}

impl Budget {
    /// A share for a file of `lines` and `bytes`; `None` when the budget has no room for it.
    fn take(self: &Arc<Self>, lines: usize, bytes: usize) -> Option<Charge> {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);

        if used.lines + lines > HELD_LINES || used.bytes + bytes > HELD_BYTES {
            return None;
        }

        used.lines += lines;
        used.bytes += bytes;
        #[cfg(test)]
        {
            used.peak = (used.peak.0.max(used.lines), used.peak.1.max(used.bytes));
        }

        Some(Charge {
            budget: Arc::clone(self),
            lines,
            bytes,
        })
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().unwrap_or_else(PoisonError::into_inner);

        used.lines -= self.lines;
        used.bytes -= self.bytes;
    }
}

/// The place of one folder's rules in the memory of the search: read once, by whichever thread comes first.
struct Slot {
    layer: OnceLock<Arc<Layer>>,
    /// When the folder was last asked for.
    used: AtomicU64,
}

impl Slot {
    fn new(used: u64) -> Self {
        Self {
            layer: OnceLock::new(),
            used: AtomicU64::new(used),
        }
    }
}

/// What `.ignore` and git ignore files say about the paths of one search.
pub(crate) struct Ignores {
    root: PathBuf,
    top: Arc<Layer>,
    kept: RwLock<HashMap<PathBuf, Arc<Slot>>>,
    clock: AtomicU64,
    budget: Arc<Budget>,
    /// The files already counted as left out (by a hash of their path), so that reading a folder again counts nothing.
    counted: Mutex<HashSet<u64>>,
    hasher: RandomState,
    skipped: Arc<AtomicU64>,
}

impl Ignores {
    /// The rules for a search of `root`, with those of the folders above it. Each file that is left out is added to
    /// `skipped`, once.
    pub(crate) fn new(root: &Path, skipped: Arc<AtomicU64>) -> Self {
        let mut ignores = Self {
            root: root.to_owned(),
            top: Arc::new(Layer::empty(root)),
            kept: RwLock::new(HashMap::new()),
            clock: AtomicU64::new(0),
            budget: Arc::new(Budget::default()),
            counted: Mutex::new(HashSet::new()),
            hasher: RandomState::new(),
            skipped,
        };

        let mut above: Vec<&Path> = root.ancestors().skip(1).collect();
        above.reverse();

        let mut layer: Option<Arc<Layer>> = None;
        for dir in above.into_iter().chain([root]) {
            layer = Some(Arc::new(ignores.read(layer, dir)));
        }
        if let Some(layer) = layer {
            ignores.top = layer;
        }

        ignores
    }

    /// The most lines and bytes of ignore files that were held at once.
    #[cfg(test)]
    pub(super) fn peak(&self) -> (usize, usize) {
        self.budget.used.lock().unwrap_or_else(PoisonError::into_inner).peak
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

    /// The rules of the folder `dir`, read when they are not remembered yet. A folder is read once at a time: a thread
    /// that finds it being read waits for the result.
    fn layer(&self, dir: &Path) -> Arc<Layer> {
        if dir == self.root || !dir.starts_with(&self.root) {
            return Arc::clone(&self.top);
        }

        let slot = self.slot(dir);
        if let Some(layer) = slot.layer.get() {
            return Arc::clone(layer);
        }

        let parent = dir
            .parent()
            .map_or_else(|| Arc::clone(&self.top), |above| self.layer(above));

        Arc::clone(slot.layer.get_or_init(|| Arc::new(self.read(Some(parent), dir))))
    }

    /// The place of the folder `dir`, made when there is none. Looking one up only reads the map.
    fn slot(&self, dir: &Path) -> Arc<Slot> {
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);

        if let Ok(kept) = self.kept.read()
            && let Some(slot) = kept.get(dir)
        {
            slot.used.store(tick, Ordering::Relaxed);

            return Arc::clone(slot);
        }

        let Ok(mut kept) = self.kept.write() else {
            return Arc::new(Slot::new(tick));
        };

        let slot = Arc::clone(kept.entry(dir.to_owned()).or_insert_with(|| Arc::new(Slot::new(tick))));
        let gone = if kept.len() > KEPT {
            forget_older_half(&mut kept)
        } else {
            Vec::new()
        };

        // The rules are freed outside the lock.
        drop(kept);
        drop(gone);

        slot
    }

    /// A share of the budget for a file; when there is none, the folders remembered longest ago are forgotten (what no
    /// lookup uses is freed) until there is, or until none is left. `None` then: the file is left out.
    fn reserve(&self, lines: usize, bytes: usize) -> Option<Charge> {
        loop {
            if let Some(charge) = self.budget.take(lines, bytes) {
                return Some(charge);
            }

            let gone = {
                let mut kept = self.kept.write().ok()?;

                if kept.is_empty() {
                    return None;
                }

                forget_older_half(&mut kept)
            };

            drop(gone);
        }
    }

    /// The rules of the folder `dir`, below the folders of `parent`.
    fn read(&self, parent: Option<Arc<Layer>>, dir: &Path) -> Layer {
        let git_entry = fs::symlink_metadata(dir.join(".git")).ok();
        let repository = git_entry.is_some();
        let in_repository = repository || parent.as_ref().is_some_and(|parent| parent.in_repository);

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

        Layer {
            parent,
            dir: dir.to_owned(),
            ignore,
            git,
            exclude,
            repository,
            in_repository,
        }
    }

    /// The rules in the file `name` (names joined with `/`) of the folder `dir`: `None` when there is none, when it holds
    /// no rule, or when it cannot be used (which is counted).
    fn file(&self, dir: &Path, name: &str) -> Option<Rules> {
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

        match parse(dir, bytes) {
            Some(gitignore) if gitignore.is_empty() => None,
            Some(gitignore) => Some(Rules {
                gitignore,
                _charge: charge,
            }),
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

/// Removes the older half of the remembered folders and returns them, to be dropped once the lock is released.
fn forget_older_half(kept: &mut HashMap<PathBuf, Arc<Slot>>) -> Vec<Arc<Slot>> {
    let mut ages: Vec<(u64, PathBuf)> = kept
        .iter()
        .map(|(dir, slot)| (slot.used.load(Ordering::Relaxed), dir.clone()))
        .collect();

    ages.sort_unstable();

    ages.into_iter()
        .take(kept.len().div_ceil(2))
        .filter_map(|(_, dir)| kept.remove(&dir))
        .collect()
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
        }
    }
}

fn matched<'a>(rules: &'a Option<Rules>, relative: &Path, is_dir: bool) -> Match<&'a Glob> {
    match rules {
        Some(rules) => rules.gitignore.matched(relative, is_dir),
        None => Match::None,
    }
}

/// The path of `name` in the folder `below` the one searched, names joined with `/`; `None` when a name is not valid Unicode.
fn listed(below: &Path, name: &str) -> Option<String> {
    let mut listed = String::new();

    for part in below.components() {
        listed.push_str(part.as_os_str().to_str()?);
        listed.push('/');
    }
    listed.push_str(name);

    Some(listed)
}

/// What `file` holds up to [`MAX_BYTES`]; `None` when it holds more, or cannot be read.
fn read_limited(file: fs::File) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut limited = file.take(MAX_BYTES + 1);

    limited.read_to_end(&mut bytes).ok()?;

    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

/// The lines of `bytes`: a final newline ends the last line, it does not start another.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let body = bytes.strip_suffix(b"\n").unwrap_or(bytes);

    body.split(|byte| *byte == b'\n').filter(|_| !bytes.is_empty())
}

/// How many lines `bytes` has; `None` when there are too many or a line is too long.
fn count_lines(bytes: &[u8]) -> Option<usize> {
    let mut count = 0;

    for line in lines(bytes) {
        count += 1;
        if count > MAX_LINES || line.len() > MAX_LINE {
            return None;
        }
    }

    Some(count)
}

/// The rules the lines of `bytes` (already within the limits) make, as if the file was in `dir`. `None` when the rules
/// cannot be built. A line that is not a valid pattern is left out, as git does.
fn parse(dir: &Path, bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(dir);

    for line in lines(bytes) {
        builder.add_line(None, &String::from_utf8_lossy(line)).ok();
    }

    builder.build().ok()
}

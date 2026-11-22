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

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder, Glob};

use crate::ops::open_listed;

/// The biggest ignore file that is read, in bytes.
const MAX_BYTES: u64 = 1024 * 1024;

/// The most lines of an ignore file.
pub(super) const MAX_LINES: usize = 10_000;

/// The longest line of an ignore file, in bytes.
pub(super) const MAX_LINE: usize = 4096;

/// The most folders whose rules are kept at once; past it the lot is dropped and read again when needed.
const KEPT: usize = 4096;

/// The rules that apply in one folder: its own files and, through `parent`, those of the folders above.
struct Layer {
    parent: Option<Arc<Layer>>,
    dir: PathBuf,
    ignore: Option<Gitignore>,
    git: Option<Gitignore>,
    exclude: Option<Gitignore>,
    /// The folder holds a `.git`.
    repository: bool,
    /// The folder is in a git repository: it holds a `.git` or a folder above does.
    in_repository: bool,
}

/// What `.ignore` and git ignore files say about the paths of one search.
pub(crate) struct Ignores {
    root: PathBuf,
    top: Arc<Layer>,
    kept: Mutex<HashMap<PathBuf, Arc<Layer>>>,
    skipped: Arc<AtomicU64>,
}

impl Ignores {
    /// The rules for a search of `root`, with those of the folders above it. Each file that is left out is added to
    /// `skipped`.
    pub(crate) fn new(root: &Path, skipped: Arc<AtomicU64>) -> Self {
        let mut ignores = Self {
            root: root.to_owned(),
            top: Arc::new(Layer::empty(root)),
            kept: Mutex::new(HashMap::new()),
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

    /// The rules of the folder `dir`, read when they are not kept yet.
    fn layer(&self, dir: &Path) -> Arc<Layer> {
        let mut missing: Vec<&Path> = Vec::new();
        let mut nearest = Arc::clone(&self.top);
        let mut at = Some(dir);

        while let Some(current) = at {
            if current == self.root {
                break;
            }

            let kept = self.kept.lock().ok().and_then(|kept| kept.get(current).cloned());
            if let Some(kept) = kept {
                nearest = kept;
                break;
            }

            missing.push(current);
            at = current.parent();
        }

        for current in missing.into_iter().rev() {
            let layer = Arc::new(self.read(Some(Arc::clone(&nearest)), current));
            self.keep(current, &layer);
            nearest = layer;
        }

        nearest
    }

    fn keep(&self, dir: &Path, layer: &Arc<Layer>) {
        let Ok(mut kept) = self.kept.lock() else {
            return;
        };

        if kept.len() >= KEPT {
            kept.clear();
        }
        kept.insert(dir.to_owned(), Arc::clone(layer));
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
    fn file(&self, dir: &Path, name: &str) -> Option<Gitignore> {
        let opened = match dir.strip_prefix(&self.root) {
            Ok(below) => match listed(below, name) {
                Some(listed) => open_listed(&self.root, &listed),
                None => return self.left_out(),
            },
            Err(_) => open_listed(dir, name),
        };

        let (file, metadata) = match opened {
            Ok(opened) => opened,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => return self.left_out(),
        };

        if metadata.len() > MAX_BYTES {
            return self.left_out();
        }

        let Some(text) = read_limited(file) else {
            return self.left_out();
        };

        match parse(dir, &text) {
            Some(rules) => (!rules.is_empty()).then_some(rules),
            None => self.left_out(),
        }
    }

    fn left_out(&self) -> Option<Gitignore> {
        self.skipped.fetch_add(1, Ordering::Relaxed);

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
        }
    }
}

fn matched<'a>(rules: &'a Option<Gitignore>, relative: &Path, is_dir: bool) -> Match<&'a Glob> {
    match rules {
        Some(rules) => rules.matched(relative, is_dir),
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

/// The rules the lines of `bytes` make, as if the file was in `dir`. `None` when there are too many lines, a line is too
/// long or the rules cannot be built. A line that is not a valid pattern is left out, as git does.
fn parse(dir: &Path, bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(dir);
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);

    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if index >= MAX_LINES || line.len() > MAX_LINE {
            return None;
        }

        builder.add_line(None, &String::from_utf8_lossy(line)).ok();
    }

    builder.build().ok()
}

//! Walking the folder tree on the threads of `ignore`'s parallel walker, one search's own.
//!
//! The walker never follows a link (a link is a result, not a folder to enter), prunes what the options leave out
//! before it enters it, and stops once the search does. It reads no file by itself: the ignore files are read by
//! [`Ignores`], which opens them without following a link or waiting on a pipe, within limits. Each thread tests the entries it meets and sends the ones that
//! fit to the calling thread over a channel; the calling thread batches them (see [`Out`]). The depth is the only bound
//! on a tree that loops back on itself (a bind mount): [`MAX_DEPTH`] levels, as deep as a copy goes.

use std::fs::Metadata;
use std::path::{Component, Path};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;

use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::listing::platform;
use crate::model::{EntryKind, FindHit};
use crate::ops::open_listed;

use super::content::{Content, Scan};
use super::filters::Facts;
use super::ignored::Ignores;
use super::inside;
use super::out::{Out, Run, pump};
use super::{Compiled, MAX_DEPTH, WALK_THREADS};

/// A file that may have a twin: what the duplicate check needs of it.
pub(crate) struct Candidate {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) modified: Option<i64>,
    pub(crate) identity: Option<(u64, u64)>,
}

/// What a walker thread sends.
enum Item {
    Hit(FindHit),
    Candidate(Candidate),
}

/// Walks `root` (the place to open, see [`platform::local_path`]) and reports what fits: as results, or, when looking for
/// duplicates, as the candidates that are returned. Returns when the walk is over or stopped.
pub(crate) fn search(
    root: &Path,
    compiled: &Compiled,
    run: &Run<'_>,
    out: &mut Out<'_>,
    limit: usize,
) -> Vec<Candidate> {
    let (sender, receiver) = mpsc::channel();
    let left_out = Arc::new(AtomicU64::new(0));
    let walker = builder(root, compiled, &left_out);
    let mut candidates = Vec::new();

    thread::scope(|scope| {
        scope.spawn(move || {
            walker.build_parallel().run(|| {
                let mut worker = Worker::new(root, compiled, run, sender.clone());

                Box::new(move |entry| worker.visit(entry))
            });
            drop(sender);
        });

        pump(&receiver, run, out, |item, out| match item {
            Item::Hit(hit) => out.push(hit, run),
            // Only the walk stops: what was collected is still compared.
            Item::Candidate(candidate) if candidates.len() >= limit => {
                drop(candidate);
                out.truncate_walk(run);
            }
            Item::Candidate(candidate) => candidates.push(candidate),
        });
    });

    run.skipped
        .fetch_add(left_out.load(Ordering::Relaxed), Ordering::Relaxed);

    candidates
}

/// The walker for `compiled`: hidden entries and ignore files as asked, links never followed. What it leaves out for a
/// name or an ignore file that cannot be used is added to `left_out`.
fn builder(root: &Path, compiled: &Compiled, left_out: &Arc<AtomicU64>) -> WalkBuilder {
    let threads = thread::available_parallelism().map_or(1, usize::from).min(WALK_THREADS);
    let filter = Filter {
        skip_hidden: compiled.skip_hidden,
        ignores: compiled.skip_ignored.then(|| Ignores::new(root, Arc::clone(left_out))),
        left_out: Arc::clone(left_out),
    };
    let mut builder = WalkBuilder::new(root);

    // Nothing is left out unless the options say so: every filter of the walker is off, and so is its reading of ignore
    // files (see the module's note); each is done by `filter`. Hidden entries are filtered by the platform's rule (on
    // Windows the attribute, not the dot).
    builder
        .standard_filters(false)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .parents(false)
        .follow_links(false)
        .max_depth(Some(MAX_DEPTH))
        .threads(threads)
        .same_file_system(compiled.one_filesystem)
        .filter_entry(move |entry| filter.enters(entry));

    builder
}

/// What the walk leaves out before it enters or reports an entry.
struct Filter {
    skip_hidden: bool,
    ignores: Option<Ignores>,
    left_out: Arc<AtomicU64>,
}

impl Filter {
    /// Whether the walk goes on with `entry` (and below it): a name that is valid Unicode (the paths of the results are
    /// text, and a name that is not could stand for another file), not one the system reads as something else, not a
    /// hidden one when hidden entries are skipped, and not one the ignore files leave out when they are asked for.
    fn enters(&self, entry: &DirEntry) -> bool {
        if entry.depth() == 0 {
            return true;
        }

        let Some(name) = entry.file_name().to_str() else {
            self.left_out.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if platform::is_unsafe_name(name) {
            return false;
        }

        if self.skip_hidden {
            let metadata = platform::HIDDEN_NEEDS_METADATA.then(|| entry.metadata().ok()).flatten();

            if platform::is_hidden(name, metadata.as_ref()) {
                return false;
            }
        }

        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());

        !self
            .ignores
            .as_ref()
            .is_some_and(|ignores| ignores.leaves_out(entry.path(), is_dir))
    }
}

/// One walker thread: tests each entry it is given.
struct Worker<'a> {
    root: &'a Path,
    compiled: &'a Compiled,
    run: &'a Run<'a>,
    sender: Sender<Item>,
    searcher: Option<grep_searcher::Searcher>,
}

impl<'a> Worker<'a> {
    fn new(root: &'a Path, compiled: &'a Compiled, run: &'a Run<'a>, sender: Sender<Item>) -> Self {
        Self {
            root,
            compiled,
            run,
            sender,
            searcher: compiled.content.as_ref().map(Content::searcher),
        }
    }

    fn visit(&mut self, entry: Result<DirEntry, ignore::Error>) -> WalkState {
        if self.run.raised() || self.run.walk_over() {
            return WalkState::Quit;
        }

        let Ok(entry) = entry else {
            self.run.skip();
            return WalkState::Continue;
        };
        if entry.depth() == 0 {
            return WalkState::Continue;
        }

        let Ok(metadata) = entry.metadata() else {
            self.run.skip();
            return WalkState::Continue;
        };

        self.count(&metadata);

        let Some(item) = self.test(&entry, &metadata) else {
            return WalkState::Continue;
        };

        if self.sender.send(item).is_err() {
            WalkState::Quit
        } else {
            WalkState::Continue
        }
    }

    fn count(&self, metadata: &Metadata) {
        if metadata.is_dir() {
            self.run.folders.fetch_add(1, Ordering::Relaxed);
        } else {
            self.run.files.fetch_add(1, Ordering::Relaxed);
            self.run.bytes.fetch_add(metadata.len(), Ordering::Relaxed);
        }
    }

    /// What `entry` is as a result, when it fits every test; archives are searched on the way.
    fn test(&mut self, entry: &DirEntry, metadata: &Metadata) -> Option<Item> {
        let compiled = self.compiled;
        let name = entry.file_name().to_str()?;
        let path = relative(self.root, entry.path())?;

        if compiled.archives
            && metadata.is_file()
            && let Some(format) = inside::format(name)
        {
            let skipped = inside::scan(self.root, &path, format, compiled, self.run, &mut |hit| {
                self.sender.send(Item::Hit(hit)).ok();
            });
            self.run.skipped.fetch_add(skipped, Ordering::Relaxed);
        }

        let facts = Facts::of(name, metadata);
        if !compiled.names.fits(name, &path) || !compiled.filters.fits(&facts) {
            return None;
        }

        let found = match &compiled.content {
            Some(content) => Some(self.read(content, &path, facts.size)?),
            None => None,
        };

        if compiled.duplicates {
            return (facts.size > 0).then(|| {
                Item::Candidate(Candidate {
                    path,
                    size: facts.size,
                    modified: facts.modified,
                    identity: platform::identity(metadata),
                })
            });
        }

        let link = metadata.file_type().is_symlink();
        let kind = kind_of(metadata);

        Some(Item::Hit(FindHit {
            path,
            inner: None,
            kind,
            link,
            size: facts.size,
            modified: facts.modified,
            matches: found.as_ref().map(|found| found.matches),
            line: found.as_ref().map(|found| found.line.clone()),
            line_number: found.as_ref().map(|found| found.number),
        }))
    }

    /// Searches the file at `path` for the text: what was found, or `None` when it holds none (or cannot be searched, which
    /// is counted).
    fn read(&mut self, content: &Content, path: &str, size: u64) -> Option<super::content::Found> {
        if content.too_big(size) {
            self.run.skip();
            return None;
        }

        let Ok((file, _)) = open_listed(self.root, path) else {
            self.run.skip();
            return None;
        };
        let searcher = self.searcher.as_mut()?;

        match content.search(searcher, file, self.run) {
            Scan::Found(found) => Some(found),
            Scan::Nothing => None,
            Scan::Unread => {
                self.run.skip();
                None
            }
        }
    }
}

/// The path of `path` below `root`, names joined with `/`. `None` when a name is not valid Unicode: the text of the path
/// would stand for another file too.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let below = path.strip_prefix(root).ok()?;
    let mut names = Vec::new();

    for part in below.components() {
        if let Component::Normal(name) = part {
            names.push(name.to_str()?);
        }
    }

    Some(names.join("/"))
}

/// What a hit is: a link is `other` (it is not followed).
fn kind_of(metadata: &Metadata) -> EntryKind {
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        EntryKind::Other
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

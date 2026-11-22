//! Find files: a search of a folder tree by name, size, date, attributes and content, and for duplicates.
//!
//! # API
//!
//! 1. [`check`] tests the criteria by their syntax alone (nothing is compiled or touched on disk), so a saved search that
//!    can never run is told from one that can. [`compile`] makes the [`Compiled`] search of a [`FindQuery`] and refuses
//!    what [`check`] refuses, a pattern that is not valid and one too big to run safely, all with a fixed text
//!    ([`crate::view::is_refusal`]).
//! 2. [`run`] walks the folder on threads of its own and calls `emit` on the calling thread with `found` batches (every
//!    100 ms or 256 results), `progress`, `group` (duplicates) and, last, exactly one `done` (`found`, `skipped`,
//!    `truncated`, `cancelled`). It returns when the search is over; an `Err` (before any event) means the folder cannot
//!    be searched. It stops, within one read or one entry, once `cancel` is set.
//! 3. [`entry`] reads a result of the filesystem as a pane lists it, `None` when it is gone.
//!
//! # Rules
//!
//! - A link is never followed: the walker does not enter one, and a file's content is opened with
//!   [`crate::ops::open_listed`], which refuses a link on the way. A link is a result by its name, with kind `other`.
//! - Entries whose names the system reads as something else (Windows devices, streams) are skipped. Hidden files are
//!   included unless `skip_hidden`; `.gitignore`, `.ignore` and `.git/info/exclude` are read only for `skip_ignored`
//!   (and, as in ripgrep, `.gitignore` files count inside a git repository only). This crate reads them itself and not
//!   the walker (see `ignored.rs`): no link followed, no pipe waited on, at most 1 MiB, 10 000 lines and 4 KiB a line
//!   each; one over a limit or unreadable is left out and counted as skipped. A `.git` file (a worktree) is not followed.
//! - Everything is bounded: a pattern of 1024 bytes, 64 masks, a regular expression of 1 MiB compiled (and 1 MiB of lazy
//!   automaton), at most 100 000 results (the walk stops there and `done` says `truncated`), 16 MiB of buffer for each
//!   of at most 8 walking threads, 1 GiB read to list one archive, 500 000 candidates of a duplicate search (the walk
//!   stops there, `done` says `truncated` and the candidates collected are still compared). The depth of the tree is
//!   bounded by 1024 levels, which is the only bound on a tree that loops back on itself.
//! - Text search reads UTF-8 (and UTF-8 and UTF-16 byte order marks) or the one encoding the search names; a file with a
//!   NUL byte in it is binary and left out whole. A file over 64 MiB is skipped (and counted) unless the search says any
//!   size. An entry with a name that is not valid Unicode (in its path below the folder) is skipped and counted, and a
//!   folder with such a name is not entered: the text of a path could stand for another file.
//! - With a text, a size limit or duplicates only files are results (a folder or a link has no content to test).

mod content;
mod dupes;
mod filters;
mod ignored;
mod inside;
mod mask;
mod out;
mod walk;

#[cfg(test)]
mod content_tests;
#[cfg(test)]
mod dupes_tests;
#[cfg(test)]
mod filters_tests;
#[cfg(test)]
mod ignored_budget_tests;
#[cfg(test)]
mod ignored_tests;
#[cfg(test)]
mod inside_tests;
#[cfg(test)]
mod mask_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod walk_tests;

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::listing::{describe_present, platform};
use crate::model::{Entry, FindCriteria, FindEvent, FindHit, FindQuery};
use crate::view::{is_known_encoding, refusal};

use content::Content;
use filters::{Filters, Names};
use out::{Out, Run};

/// The longest name pattern or text, in bytes.
pub const MAX_PATTERN_BYTES: usize = 1024;

/// The most masks in a name.
pub const MAX_MASKS: usize = 64;

/// The most results of a search; the walk stops there.
pub const MAX_HITS: u64 = 100_000;

/// The biggest a compiled regular expression (and the lazy automaton that runs it) may be, in bytes.
const REGEX_LIMIT: usize = 1024 * 1024;

/// How deeply a regular expression may nest.
const NEST_LIMIT: u32 = 100;

/// The most a text search buffers for one thread: the longest line it can read.
const HEAP_LIMIT: usize = 16 * 1024 * 1024;

/// The characters kept of the first matching line.
const SNIPPET_CHARS: usize = 256;

/// The biggest file a text search reads, unless it is told any size.
pub const MAX_TEXT_FILE: u64 = 64 * 1024 * 1024;

/// The most levels below the folder that are searched: the depth a copy goes to too.
const MAX_DEPTH: usize = 1_024;

/// The most threads that walk the tree.
const WALK_THREADS: usize = 8;

/// The most bytes of one archive that are read to list it.
const ARCHIVE_SCAN: u64 = 1024 * 1024 * 1024;

/// The most entries read from one archive.
const MAX_ARCHIVE_RECORDS: usize = 200_000;

/// The most files a duplicate search keeps to compare; past it the search stops walking and says `truncated`.
const MAX_CANDIDATES: usize = 500_000;

/// The bytes at the start of a file that are hashed first.
const HASH_HEAD: u64 = 64 * 1024;

/// The threads that hash files.
const HASH_THREADS: usize = 4;

/// A search ready to run: its patterns compiled, its limits set.
pub struct Compiled {
    names: Names,
    filters: Filters,
    content: Option<Content>,
    skip_hidden: bool,
    skip_ignored: bool,
    one_filesystem: bool,
    archives: bool,
    duplicates: bool,
}

// A search is compiled once and run on many threads, and a caller keeps it in a shared handle.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Compiled>();
};

fn invalid(text: &'static str) -> io::Error {
    refusal(io::ErrorKind::InvalidInput, text)
}

/// Whether `criteria` can ever run, judged by their syntax and consistency alone (nothing is compiled or read). Fails
/// with a refusal (`InvalidInput`) of a fixed text.
pub fn check(criteria: &FindCriteria) -> io::Result<()> {
    let names = criteria.names.trim();

    if names.len() > MAX_PATTERN_BYTES {
        return Err(invalid("the name is too long"));
    }
    if !criteria.name_regex && mask::parts(names).count() > MAX_MASKS {
        return Err(invalid("the name has too many masks"));
    }

    if let Some(text) = &criteria.text {
        if text.pattern.is_empty() {
            return Err(invalid("the text is empty"));
        }
        if text.pattern.len() > MAX_PATTERN_BYTES {
            return Err(invalid("the text is too long"));
        }
        if text.pattern.contains(['\n', '\r']) {
            return Err(invalid("the text must be on one line"));
        }
        if text.encoding.as_deref().is_some_and(|label| !is_known_encoding(label)) {
            return Err(invalid("the encoding is not known"));
        }
    }

    if criteria
        .min_size
        .zip(criteria.max_size)
        .is_some_and(|(min, max)| min > max)
    {
        return Err(invalid("the smallest size is above the largest"));
    }
    if criteria
        .modified_from
        .zip(criteria.modified_to)
        .is_some_and(|(from, to)| from > to)
    {
        return Err(invalid("the first date is after the last"));
    }
    if cfg!(windows) && criteria.executable != crate::model::Tri::Any {
        return Err(invalid("executable files cannot be told on Windows"));
    }
    if criteria.skip_hidden && criteria.hidden == crate::model::Tri::Yes {
        return Err(invalid("hidden files are skipped and asked for at once"));
    }
    if criteria.archives && (criteria.duplicates || criteria.text.is_some()) {
        return Err(invalid("archives are searched by name only"));
    }

    Ok(())
}

/// The search of `query`: [`check`] passed, the folder's syntax valid, every pattern compiled within its limits.
pub fn compile(query: &FindQuery) -> io::Result<Compiled> {
    let criteria = &query.criteria;

    platform::check_folder(Path::new(&query.root)).map_err(|text| refusal(io::ErrorKind::InvalidInput, text))?;
    check(criteria)?;

    Ok(Compiled {
        names: Names::new(criteria)?,
        filters: Filters::new(criteria),
        content: criteria.text.as_ref().map(Content::new).transpose()?,
        skip_hidden: criteria.skip_hidden,
        skip_ignored: criteria.skip_ignored,
        one_filesystem: criteria.one_filesystem,
        archives: criteria.archives,
        duplicates: criteria.duplicates,
    })
}

/// Runs `compiled` in the folder `root` (absolute; checked and resolved the way a pane opens a folder), sending its
/// events to `emit` on this thread, and returns when the search has ended with `done`. See the module's rules.
///
/// Fails, before any event, when `root` cannot be searched: not a plain absolute local path, not a folder, not readable.
pub fn run(root: &Path, compiled: &Compiled, cancel: &AtomicBool, emit: &mut dyn FnMut(FindEvent)) -> io::Result<()> {
    run_limited(root, compiled, cancel, Limits::DEFAULT, emit)
}

/// The limits that are set by a constant and that a test lowers.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) hits: u64,
    pub(crate) candidates: usize,
}

impl Limits {
    pub(crate) const DEFAULT: Self = Self {
        hits: MAX_HITS,
        candidates: MAX_CANDIDATES,
    };
}

/// [`run`] within `limits`.
pub(crate) fn run_limited(
    root: &Path,
    compiled: &Compiled,
    cancel: &AtomicBool,
    limits: Limits,
    emit: &mut dyn FnMut(FindEvent),
) -> io::Result<()> {
    platform::check_folder(root).map_err(|text| refusal(io::ErrorKind::InvalidInput, text))?;

    let actual = platform::local_path(root)?;
    if !fs::metadata(&actual)?.is_dir() {
        return Err(refusal(io::ErrorKind::NotADirectory, "this is not a folder"));
    }
    fs::read_dir(&actual)?;

    let run = Run::new(cancel);
    let mut out = Out::new(emit, limits.hits);
    let candidates = walk::search(&actual, compiled, &run, &mut out, limits.candidates);

    if compiled.duplicates && !run.raised() {
        dupes::find(&actual, candidates, &run, &mut out);
    }

    out.finish(&run);

    Ok(())
}

/// The result `hit` of a search in `root` as a pane lists it (its name is the path below `root`), read now. `None` when
/// the file is gone, or the hit is inside an archive, or its path is not made of plain names, or a folder on the way to it
/// is a link or a file now (the folders are walked one name at a time from `root`, none followed; a folder swapped for a
/// link after the search drops its hits). What is left between that check and the read of the entry is the window every
/// pane has.
pub fn entry(root: &Path, hit: &FindHit) -> Option<Entry> {
    if hit.inner.is_some() {
        return None;
    }

    let mut names: Vec<&str> = hit.path.split('/').collect();
    let leaf = names.pop()?;
    if !names.iter().chain([&leaf]).all(|name| platform::is_plain_name(name)) {
        return None;
    }

    crate::ops::check_folder_below(root, &names.join("/")).ok()?;

    let folder = names.iter().fold(root.to_owned(), |folder, name| folder.join(name));
    let mut entry = describe_present(&folder, leaf)?;
    entry.name.clone_from(&hit.path);

    Some(entry)
}

//! Fast folder listings for the panes.
//!
//! [`list`] works in two passes so the first screen arrives long before a huge folder is fully read:
//!
//! 1. Read every name and kind (one `readdir` pass; no per-entry `stat` on Unix except for symbolic links) and
//!    drop hidden entries unless asked for. The total is known now, so the UI can size its scrollbar from the first
//!    event. Only the first screen is put in order now (`order_first`: folders first, then by name, [`platform`]
//!    rules); the rest is sorted once the first batch is sent, as sorting a huge folder takes longer than picking out
//!    its first screen.
//! 2. Read the metadata of the sorted entries in parallel, in batches, in order: a small first batch (the
//!    first screen), then larger ones. A cancelled listing stops between batches.
//!
//! [`watch`] reports changes to a listed folder, debounced.

pub mod platform;
pub mod size;
mod watch;

use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

pub use watch::{FolderWatch, reason as watch_error_reason, watch};

use crate::model::{Crumb, Entry, EntryKind, ListEvent, PaneCapabilities, PaneLocation};

/// Entries in the first batch: about one screen, so it is ready within milliseconds.
pub const FIRST_BATCH: usize = 200;

/// Entries in every later batch.
pub const BATCH: usize = 2_000;

/// How a listing ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every entry was sent, then `done`.
    Done,
    /// `cancel` was set; the events stopped early (no `done`).
    Cancelled,
}

/// Listings of up to this many entries read their metadata on the listing thread, with no pool.
const SEQUENTIAL_MAX: usize = 256;

/// Threads of a listing's own pool for the second pass.
const POOL_THREADS: usize = 8;

/// An entry known after the first pass.
struct Name {
    name: String,
    os_name: OsString,
    key: String,
    dir: bool,
}

/// Lists the folder at `path` (absolute), sending `start`, the `batch`es and `done` to `emit`, and stops early
/// once `cancel` is set. Hidden entries are left out unless `show_hidden`. Fails only when the folder itself
/// cannot be read; an entry that vanishes or cannot be read meanwhile is skipped or listed without details.
pub fn list(path: &Path, show_hidden: bool, cancel: &AtomicBool, emit: impl FnMut(ListEvent)) -> io::Result<Outcome> {
    list_at(path, path, show_hidden, cancel, emit)
}

/// [`list`] for a folder shown as `shown` but read at `actual`: the same folder, spelled by the path that
/// [`platform::local_path`] resolved, which holds no link. The system then never resolves a link of its own while
/// reading it (on Windows that could reach a network host), and the pane keeps the path the user chose.
pub fn list_at(
    shown: &Path,
    actual: &Path,
    show_hidden: bool,
    cancel: &AtomicBool,
    mut emit: impl FnMut(ListEvent),
) -> io::Result<Outcome> {
    if !shown.is_absolute() || !actual.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the path must be absolute"));
    }

    let Some(mut names) = read_names(actual, show_hidden, cancel)? else {
        return Ok(Outcome::Cancelled);
    };

    emit(ListEvent::Start {
        path: shown.to_string_lossy().into_owned(),
        parent: shown.parent().map(|parent| parent.to_string_lossy().into_owned()),
        crumbs: crumbs(shown),
        total: count(names.len()),
        location: PaneLocation::Folder {
            path: shown.to_string_lossy().into_owned(),
        },
        capabilities: PaneCapabilities::WRITABLE,
    });

    let (mut dirs, mut files, mut bytes) = (0_u32, 0_u32, 0_u64);
    let mut offset = 0;
    let pool = (names.len() > SEQUENTIAL_MAX).then(describe_pool).flatten();
    order_first(&mut names);

    while offset < names.len() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled);
        }

        // The first batch is out: now the rest is put in order, behind it.
        if offset == FIRST_BATCH {
            names[FIRST_BATCH..].par_sort_unstable_by(order);
        }

        let size = if offset == 0 { FIRST_BATCH } else { BATCH };
        let chunk = &names[offset..names.len().min(offset + size)];
        let Some(entries) = describe_batch(pool.as_ref(), chunk, actual, cancel) else {
            return Ok(Outcome::Cancelled);
        };

        for entry in &entries {
            match entry.kind {
                EntryKind::Dir => dirs = dirs.saturating_add(1),
                _ => {
                    files = files.saturating_add(1);
                    bytes = bytes.saturating_add(entry.size);
                }
            }
        }

        emit(ListEvent::Batch {
            offset: count(offset),
            entries,
        });
        offset += chunk.len();
    }

    emit(ListEvent::Done {
        dirs,
        files,
        bytes,
        free: platform::free_space(actual),
    });

    Ok(Outcome::Done)
}

/// The first pass: names and kinds, filtered, in the order the system gave them. `None` when cancelled.
fn read_names(path: &Path, show_hidden: bool, cancel: &AtomicBool) -> io::Result<Option<Vec<Name>>> {
    let mut names = Vec::new();

    for entry in fs::read_dir(path)? {
        // One atomic load per entry: a slow folder stops promptly once the pane moves on.
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }

        // An entry that cannot be read (removed meanwhile, a broken mount) is skipped, not fatal.
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else { continue };

        let os_name = entry.file_name();
        let name = os_name.to_string_lossy().into_owned();

        if !show_hidden {
            let metadata = platform::HIDDEN_NEEDS_METADATA.then(|| entry.metadata().ok()).flatten();
            if platform::is_hidden(&name, metadata.as_ref()) {
                continue;
            }
        }

        // A link to a folder sorts and opens like a folder (links that lead to network places are not followed).
        // An unsafe name is never followed or counted as a folder (see `platform::is_unsafe_name`).
        let dir = if platform::is_unsafe_name(&name) {
            false
        } else if file_type.is_symlink() {
            platform::follow(&entry.path()).is_some_and(|metadata| metadata.is_dir())
        } else {
            file_type.is_dir()
        };

        names.push(Name {
            key: platform::sort_key(&name),
            name,
            os_name,
            dir,
        });
    }

    Ok(Some(names))
}

/// The order of a listing: folders first, then by name ([`platform::compare`]).
fn order(a: &Name, b: &Name) -> std::cmp::Ordering {
    b.dir
        .cmp(&a.dir)
        .then_with(|| platform::compare(&a.key, &a.name, &b.key, &b.name))
        // Two names that are not UTF-8 can read alike once made readable (`a\xff` and `a\xfe` are both `a\u{FFFD}`): the
        // system's own bytes order them, so they keep one order however the listing is sorted.
        .then_with(|| a.os_name.cmp(&b.os_name))
}

/// Puts the first [`FIRST_BATCH`] names in their final order and every later one after them, unsorted (see the
/// module docs). Picking them out is linear, where sorting the whole of a huge folder is not; a folder that fits in
/// the first batch is sorted whole.
fn order_first(names: &mut [Name]) {
    if names.len() > FIRST_BATCH {
        names.select_nth_unstable_by(FIRST_BATCH, order);
        names[..FIRST_BATCH].sort_unstable_by(order);
    } else {
        names.sort_unstable_by(order);
    }
}

/// A pool for one listing's second pass: its own threads, so folders that do not respond (a stale mount inside the
/// listed folder blocks the thread reading its metadata for good) can never stall another listing or pane, the way
/// they could on rayon's shared pool. `None` when the threads cannot be started; the listing then runs without it.
fn describe_pool() -> Option<rayon::ThreadPool> {
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(POOL_THREADS);

    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("pairpane-list-{index}"))
        .build()
        .ok()
}

/// The second pass for one batch, in order: `None` once `cancel` is set (entries are skipped from then on, so a
/// cancelled listing does not go on reading a slow disk).
fn describe_batch(
    pool: Option<&rayon::ThreadPool>,
    chunk: &[Name],
    folder: &Path,
    cancel: &AtomicBool,
) -> Option<Vec<Entry>> {
    let one = |name: &Name| (!cancel.load(Ordering::Relaxed)).then(|| describe(folder, name));

    match pool {
        Some(pool) => pool.install(|| chunk.par_iter().map(one).collect()),
        None => chunk.iter().map(one).collect(),
    }
}

/// The second pass for one entry: its metadata, following a symbolic link for kind, size and date.
fn describe(folder: &Path, name: &Name) -> Entry {
    if platform::is_unsafe_name(&name.name) {
        return unread(name);
    }

    let own = fs::symlink_metadata(folder.join(&name.os_name)).ok();

    describe_known(folder, name, own)
}

/// The entry `name` of `folder` as a listing shows it, read now; `None` when it is gone or its name must not be opened
/// (see `platform::is_unsafe_name`). For a caller that lists entries it found itself (a search's results).
pub fn describe_present(folder: &Path, name: &str) -> Option<Entry> {
    if platform::is_unsafe_name(name) {
        return None;
    }

    let name = Name {
        name: name.to_owned(),
        os_name: OsString::from(name),
        key: String::new(),
        dir: false,
    };
    let own = fs::symlink_metadata(folder.join(&name.os_name)).ok()?;

    Some(describe_known(folder, &name, Some(own)))
}

/// [`describe`] for an entry whose own metadata (not following a link) `own` is already read; `None` when it vanished.
fn describe_known(folder: &Path, name: &Name, own: Option<Metadata>) -> Entry {
    let path = folder.join(&name.os_name);

    let link = own
        .as_ref()
        .filter(|metadata| metadata.file_type().is_symlink())
        .and_then(|_| fs::read_link(&path).ok());

    // Links that lead to network places are not followed (see `platform::local_path`).
    let followed = match &link {
        Some(_) => platform::follow(&path),
        None => own.clone(),
    };
    let link_target = link.map(|target| target.to_string_lossy().into_owned());
    let shown = followed.as_ref().or(own.as_ref());

    let kind = match &followed {
        Some(metadata) if metadata.is_dir() => EntryKind::Dir,
        Some(metadata) if metadata.is_file() => EntryKind::File,
        // A vanished entry keeps the kind the first pass saw, so it stays among its kind.
        None if own.is_none() && name.dir => EntryKind::Dir,
        None if own.is_none() => EntryKind::File,
        _ => EntryKind::Other,
    };

    Entry {
        name: name.name.clone(),
        kind,
        size: match kind {
            EntryKind::File => shown.map_or(0, Metadata::len),
            _ => 0,
        },
        modified: shown.and_then(modified_millis),
        hidden: platform::is_hidden(&name.name, own.as_ref()),
        readonly: shown.is_some_and(|metadata| metadata.permissions().readonly()),
        mode: shown.and_then(platform::mode),
        link_target,
    }
}

/// An entry whose name must not be opened (see `platform::is_unsafe_name`): shown, but never read.
fn unread(name: &Name) -> Entry {
    Entry {
        name: name.name.clone(),
        kind: EntryKind::Other,
        size: 0,
        modified: None,
        hidden: false,
        readonly: false,
        mode: None,
        link_target: None,
    }
}

/// Last modification in milliseconds since the Unix epoch (negative before it).
pub(crate) fn modified_millis(metadata: &Metadata) -> Option<i64> {
    system_millis(metadata.modified().ok()?)
}

/// `time` in milliseconds since the Unix epoch (negative before it).
pub(crate) fn system_millis(time: std::time::SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
    }
}

/// The path bar for `path`: one crumb per folder from the root down. The root is named as written (`/`, `C:\`).
pub fn crumbs(path: &Path) -> Vec<Crumb> {
    let mut crumbs: Vec<Crumb> = path
        .ancestors()
        .map(|ancestor| {
            let name = match ancestor.components().next_back() {
                Some(Component::Normal(name)) => name.to_string_lossy().into_owned(),
                _ => ancestor.to_string_lossy().into_owned(),
            };

            Crumb {
                name,
                path: ancestor.to_string_lossy().into_owned(),
                inside: None,
            }
        })
        .collect();
    crumbs.reverse();

    crumbs
}

/// Entry counts are capped at `u32::MAX` on the wire (no folder comes close). Public for the listings the app builds
/// itself (search results).
pub fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;

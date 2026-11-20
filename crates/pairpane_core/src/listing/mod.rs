//! Fast folder listings for the panes.
//!
//! [`list`] works in two passes so the first screen arrives long before a huge folder is fully read:
//!
//! 1. Read every name and kind (one `readdir` pass; no per-entry `stat` on Unix except for symbolic links),
//!    drop hidden entries unless asked for, and sort: folders first, then by name ([`platform`] rules).
//!    The total is known now, so the UI can size its scrollbar from the first event.
//! 2. Read the metadata of the sorted entries in parallel, in batches, in order: a small first batch (the
//!    first screen), then larger ones. A cancelled listing stops between batches.
//!
//! [`watch`] reports changes to a listed folder, debounced.

pub mod platform;
mod watch;

use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

pub use watch::{FolderWatch, reason as watch_error_reason, watch};

use crate::model::{Crumb, Entry, EntryKind, ListEvent};

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
pub fn list(
    path: &Path,
    show_hidden: bool,
    cancel: &AtomicBool,
    mut emit: impl FnMut(ListEvent),
) -> io::Result<Outcome> {
    if !path.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the path must be absolute"));
    }

    let Some(names) = read_names(path, show_hidden, cancel)? else {
        return Ok(Outcome::Cancelled);
    };

    emit(ListEvent::Start {
        path: path.to_string_lossy().into_owned(),
        parent: path.parent().map(|parent| parent.to_string_lossy().into_owned()),
        crumbs: crumbs(path),
        total: count(names.len()),
    });

    let (mut dirs, mut files, mut bytes) = (0_u32, 0_u32, 0_u64);
    let mut offset = 0;

    while offset < names.len() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled);
        }

        let size = if offset == 0 { FIRST_BATCH } else { BATCH };
        let chunk = &names[offset..names.len().min(offset + size)];
        let entries: Vec<Entry> = chunk.par_iter().map(|name| describe(path, name)).collect();

        for entry in &entries {
            match entry.kind {
                EntryKind::Dir => dirs += 1,
                _ => {
                    files += 1;
                    bytes += entry.size;
                }
            }
        }

        emit(ListEvent::Batch {
            offset: count(offset),
            entries,
        });
        offset += chunk.len();
    }

    emit(ListEvent::Done { dirs, files, bytes });

    Ok(Outcome::Done)
}

/// The first pass: names and kinds, filtered and sorted. `None` when cancelled.
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

        // A link to a folder sorts and opens like a folder (links to network places are not followed).
        let dir = if file_type.is_symlink() {
            let path = entry.path();
            fs::read_link(&path).is_ok_and(|target| platform::may_follow(&target))
                && fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir())
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

    names.par_sort_unstable_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| platform::compare(&a.key, &a.name, &b.key, &b.name))
    });

    Ok(Some(names))
}

/// The second pass for one entry: its metadata, following a symbolic link for kind, size and date.
fn describe(folder: &Path, name: &Name) -> Entry {
    let path = folder.join(&name.os_name);
    let own = fs::symlink_metadata(&path).ok();

    let link = own
        .as_ref()
        .filter(|metadata| metadata.file_type().is_symlink())
        .and_then(|_| fs::read_link(&path).ok());

    // Links to network places are not followed (see `platform::may_follow`).
    let followed = match &link {
        Some(target) if platform::may_follow(target) => fs::metadata(&path).ok(),
        Some(_) => None,
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

/// Last modification in milliseconds since the Unix epoch (negative before it).
fn modified_millis(metadata: &Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?;

    match modified.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
    }
}

/// The path bar for `path`: one crumb per folder from the root down. The root is named as written (`/`, `C:\`).
fn crumbs(path: &Path) -> Vec<Crumb> {
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
            }
        })
        .collect();
    crumbs.reverse();

    crumbs
}

/// Entry counts are capped at `u32::MAX` on the wire (no folder comes close).
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;

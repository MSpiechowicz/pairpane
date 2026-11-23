//! The Trash where the system does the work: the shell on Windows (which can list the Recycle Bin and restore from it),
//! Finder on macOS (which cannot), through the `trash` crate.
//!
//! The items of a move are found again by name, folder and the time of the move ([`pick`]).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ::trash::TrashItem;

use super::{NOT_MOVED, Trashed, UntrashFailure};
use crate::listing::platform;
use crate::ops::items::ItemFailure;
use crate::ops::report::refusal;

// `list` and `restore` read the Trash and put items back.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as system;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use unsupported as system;

/// Moves the items `names` of `folder` to the Trash one by one, stopping at the first that fails (see
/// [`crate::ops::trash`]).
pub(super) fn trash(folder: &Path, names: &[String]) -> (Trashed, Result<(), ItemFailure>) {
    let from = now_seconds().saturating_sub(1);
    let mut done = Vec::with_capacity(names.len());
    let mut result = Ok(());

    for name in names {
        match trash_one(folder, name) {
            Ok(()) => done.push(name.clone()),
            Err(error) => {
                result = Err(ItemFailure {
                    name: name.clone(),
                    error,
                });
                break;
            }
        }
    }

    let record = Trashed {
        folder: folder.to_path_buf(),
        names: done,
        from,
        to: now_seconds().saturating_add(1),
    };

    (record, result)
}

/// Moves one item to the Trash. A link goes as the link (only the folder holding it is resolved), never its target.
/// Moves the item `name` of `folder` to the Trash, by path: the system takes a path, not a handle (see `sync_remove`).
pub(super) fn trash_named(folder: &Path, name: &str) -> io::Result<()> {
    trash_one(folder, name)
}

fn trash_one(folder: &Path, name: &str) -> io::Result<()> {
    if !platform::is_plain_name(name) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the name is not a plain file name",
        ));
    }

    let path = folder.join(name);

    ::trash::delete(&path).map_err(|error| {
        tracing::warn!(?error, "cannot move an item to the Trash");

        item_error(&error, &path)
    })
}

/// The error to show for `error`, without the paths that the crate's own errors carry.
fn item_error(error: &::trash::Error, path: &Path) -> io::Error {
    use ::trash::Error;

    match error {
        Error::TargetedRoot => refusal(
            io::ErrorKind::InvalidInput,
            "the root of a disk cannot be moved to the Trash",
        ),
        // The crate does not say which: the item is gone, or it cannot be read.
        Error::CouldNotAccess { .. } | Error::CanonicalizePath { .. } => match fs::symlink_metadata(path) {
            Err(error) => io::Error::from(error.kind()),
            Ok(_) => io::Error::from(io::ErrorKind::PermissionDenied),
        },
        // The shell on Windows fails an item that is gone with an error code of its own.
        _ if fs::symlink_metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound) => {
            io::Error::from(io::ErrorKind::NotFound)
        }
        _ => refusal(io::ErrorKind::Other, NOT_MOVED),
    }
}

/// Puts the items of `record` back where they were (see [`crate::ops::untrash`]).
pub(super) fn untrash(record: &Trashed) -> Result<usize, UntrashFailure> {
    let picked = pick(system::list(record)?, record);

    if picked.is_empty() {
        return Err(UntrashFailure::NothingFound);
    }

    system::restore(picked)
}

/// The failure for an item that is at the original place of an item to put back.
#[cfg(windows)]
fn in_the_way(path: &Path) -> UntrashFailure {
    UntrashFailure::InTheWay {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// The items of `all` (what the Trash holds) that `record` moved there: its names, stamped inside its time window,
/// from its folder, and only the newest of those with one original path (the system refuses to restore two at once).
/// Sorted by original path.
pub(in crate::ops) fn pick(all: Vec<TrashItem>, record: &Trashed) -> Vec<TrashItem> {
    let names: HashSet<&str> = record.names.iter().map(String::as_str).collect();
    let candidates: Vec<TrashItem> = all
        .into_iter()
        .filter(|item| (record.from..=record.to).contains(&item.time_deleted))
        .filter(|item| item.name.to_str().is_some_and(|name| names.contains(name)))
        .collect();

    // The disk is asked only about the few items that can match.
    let folder = if candidates.is_empty() {
        None
    } else {
        comparable(&record.folder)
    };
    let mut newest: HashMap<PathBuf, TrashItem> = HashMap::new();

    for item in candidates {
        if !is_in_folder(&item, &record.folder, folder.as_deref()) {
            continue;
        }

        let path = item.original_path();
        match newest.get(&path) {
            Some(kept) if kept.time_deleted >= item.time_deleted => {}
            _ => {
                newest.insert(path, item);
            }
        }
    }

    let mut picked: Vec<TrashItem> = newest.into_values().collect();
    picked.sort_by_key(TrashItem::original_path);

    picked
}

/// Whether `item` was in `folder`. The system may report the folder by another spelling (a link on the way
/// resolved, on Windows a `\\?\` prefix), so when the paths differ both are resolved and compared; `resolved` is
/// `folder` resolved already.
fn is_in_folder(item: &TrashItem, folder: &Path, resolved: Option<&Path>) -> bool {
    if item.original_parent == folder {
        return true;
    }

    match (resolved, comparable(&item.original_parent)) {
        (Some(wanted), Some(found)) => wanted == found,
        _ => false,
    }
}

/// `path` resolved so that two spellings of one folder are equal, or `None` when it does not resolve.
///
/// Windows goes through `local_path`, which refuses a network or device path (the system must not connect to a
/// host because of a name in the Trash); elsewhere resolving a path never connects anywhere.
pub(in crate::ops) fn comparable(path: &Path) -> Option<PathBuf> {
    if cfg!(windows) {
        let resolved = platform::local_path(path).ok()?;

        return Some(PathBuf::from(windows_comparable(resolved.to_str()?)));
    }

    fs::canonicalize(path).ok()
}

/// A Windows path as text that equals the same place spelled another way: no `\\?\` before a drive path, and
/// lowercase (the drives are usually case-insensitive).
pub(in crate::ops) fn windows_comparable(path: &str) -> String {
    let plain = match path.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest,
        _ => path,
    };

    plain.to_lowercase()
}

/// The seconds since the Unix epoch, or 0 when the clock is before it.
fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

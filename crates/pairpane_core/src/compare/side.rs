//! One side of a comparison, and how a folder of it is listed: a folder on disk (read with no link followed) or a folder
//! inside an archive (read from the index the caller made).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::archive::{ArchiveIndex, Child as ArchiveChild, EntryType};
use crate::listing::{modified_millis, platform};
use crate::model::{CompareFacts, EntryKind};
use crate::ops::{Child, Kind, read_children};
use crate::view::refusal;

/// One side of a comparison.
#[derive(Debug, Clone)]
pub enum Side {
    /// A folder on disk, absolute.
    Folder(PathBuf),
    /// The folder `dir` (`""` is the root, `a/b` a folder) of an archive whose index the caller made.
    Archive { index: Arc<ArchiveIndex>, dir: String },
}

/// Where the walk is on one side: a folder of the disk, or a folder of an archive.
#[derive(Clone)]
pub(super) enum Cursor<'a> {
    Folder(PathBuf),
    Archive { index: &'a ArchiveIndex, dir: String },
}

/// A name in a folder, and what it is.
pub(super) struct Item {
    pub(super) name: String,
    pub(super) facts: CompareFacts,
}

/// What a folder holds: the entries (by name), and how many could not be used (a name that is not text, an entry that
/// cannot be read, a name the system reads as something else).
pub(super) struct Listing {
    pub(super) items: Vec<Item>,
    pub(super) skipped: u64,
}

impl<'a> Cursor<'a> {
    pub(super) fn of(side: &'a Side) -> Self {
        match side {
            Side::Folder(path) => Self::Folder(path.clone()),
            Side::Archive { index, dir } => Self::Archive {
                index,
                dir: dir.clone(),
            },
        }
    }

    /// The folder `name` of this one.
    pub(super) fn child(&self, name: &str) -> Self {
        match self {
            Self::Folder(path) => Self::Folder(path.join(name)),
            Self::Archive { index, dir } => Self::Archive {
                index,
                dir: if dir.is_empty() {
                    name.to_owned()
                } else {
                    format!("{dir}/{name}")
                },
            },
        }
    }

    /// The folder on disk, when this is one.
    pub(super) fn folder(&self) -> Option<&Path> {
        match self {
            Self::Folder(path) => Some(path),
            Self::Archive { .. } => None,
        }
    }

    /// Where the link `name` of this folder leads, as text; `None` when it cannot be read or is not text.
    pub(super) fn link_target(&self, name: &str) -> Option<String> {
        match self {
            Self::Folder(path) => fs::read_link(path.join(name)).ok()?.to_str().map(str::to_owned),
            Self::Archive { index, dir } => index.find(dir, name)?.entry?.link.clone(),
        }
    }

    /// The entries of this folder, sorted by name; the hidden ones only when `hidden` says so.
    pub(super) fn list(&self, hidden: bool) -> io::Result<Listing> {
        let mut listing = match self {
            Self::Folder(path) => list_folder(path, hidden)?,
            Self::Archive { index, dir } => list_archive(index, dir, hidden)?,
        };
        listing.items.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(listing)
    }
}

fn list_folder(path: &Path, hidden: bool) -> io::Result<Listing> {
    let found = read_children(path)?;
    let mut listing = Listing {
        items: Vec::with_capacity(found.children.len()),
        skipped: u64::from(found.skipped).saturating_add(found.failed.len() as u64),
    };

    for child in &found.children {
        let Some(name) = child.name.to_str() else {
            listing.skipped += 1;
            continue;
        };
        if !hidden && platform::is_hidden(name, Some(&child.meta)) {
            continue;
        }

        listing.items.push(Item {
            name: name.to_owned(),
            facts: local_facts(child),
        });
    }

    Ok(listing)
}

/// What the entry `child` of a folder on disk is: a link is `other` and flagged, never followed.
fn local_facts(child: &Child) -> CompareFacts {
    let (kind, link) = match child.kind {
        Kind::Dir => (EntryKind::Dir, false),
        Kind::File => (EntryKind::File, false),
        Kind::Link => (EntryKind::Other, true),
        Kind::Other => (EntryKind::Other, false),
    };

    CompareFacts {
        kind,
        link,
        size: if kind == EntryKind::File { child.meta.len() } else { 0 },
        modified: modified_millis(&child.meta),
        files: 0,
        bytes: 0,
    }
}

fn list_archive(index: &ArchiveIndex, dir: &str, hidden: bool) -> io::Result<Listing> {
    let children = index
        .list(dir)
        .ok_or_else(|| refusal(io::ErrorKind::NotFound, "this folder is not in the archive"))?;

    Ok(Listing {
        items: children
            .iter()
            // The rule of a hidden name inside an archive is the Unix one, whatever the system.
            .filter(|child| hidden || !child.name.starts_with('.'))
            .map(|child| Item {
                name: child.name.to_owned(),
                facts: archive_facts(child),
            })
            .collect(),
        skipped: 0,
    })
}

/// What the entry `child` of an archive is: a link is `other` and flagged, and so is a folder that only exists because of
/// what is below it, with no time.
fn archive_facts(child: &ArchiveChild<'_>) -> CompareFacts {
    let (kind, link, size) = match child.entry {
        _ if child.dir => (EntryKind::Dir, false, 0),
        Some(entry) if entry.kind == EntryType::File => (EntryKind::File, false, entry.size),
        Some(entry) if matches!(entry.kind, EntryType::Symlink | EntryType::Hardlink) => (EntryKind::Other, true, 0),
        _ => (EntryKind::Other, false, 0),
    };

    CompareFacts {
        kind,
        link,
        size,
        modified: child.entry.and_then(|entry| entry.modified),
        files: 0,
        bytes: 0,
    }
}

/// How many entries of the index have a name that is refused and lie below `dir`: they are never listed.
pub(super) fn refused_below(index: &ArchiveIndex, dir: &str) -> u64 {
    let count = index
        .entries()
        .iter()
        .filter(|entry| entry.refused.is_some())
        .filter(|entry| dir.is_empty() || entry.path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/')))
        .count();

    count as u64
}

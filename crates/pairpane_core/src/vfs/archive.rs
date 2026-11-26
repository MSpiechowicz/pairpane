//! A folder inside an archive, listed from the archive's index.
//!
//! The index is built once and kept by the caller, so moving between the folders of one archive reads nothing. Nothing
//! is extracted and nothing is watched: an archive is read-only and its file is not followed for changes.
//!
//! The `start` event carries the folder that holds the archive as `path` (what a tab saves and `..` from the archive's
//! root returns to), the real folders as crumbs, then the archive's own crumb and one crumb per folder inside it (each
//! with `inside` set), and the place shown as the `location`.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::{VfsFolder, send_listing, sorted};
use crate::archive::{ArchiveIndex, Child, EntryType};
use crate::listing::{Outcome, count};
use crate::model::{Crumb, Entry, EntryKind, ListEvent, PaneCapabilities, PaneLocation};

/// The folder `dir` (`""` is the root, `a/b` a folder) of an archive whose `index` is built.
pub struct ArchiveFolder {
    index: Arc<ArchiveIndex>,
    /// The folder that holds the archive file.
    folder: PathBuf,
    /// The archive file's name.
    name: String,
    /// `folder` and `name` joined.
    archive: PathBuf,
    dir: String,
}

impl ArchiveFolder {
    pub fn new(index: Arc<ArchiveIndex>, folder: PathBuf, name: String, dir: String) -> Self {
        let archive = folder.join(&name);

        Self {
            index,
            folder,
            name,
            archive,
            dir,
        }
    }

    /// The entries of the folder, folders first and then by name (the rules of a folder listing); hidden ones (a name
    /// with a leading dot) only when `show_hidden`. `None` when the folder is not in the archive.
    fn describe(&self, show_hidden: bool) -> Option<Vec<Entry>> {
        let found = self
            .index
            .list(&self.dir)?
            .into_iter()
            .filter(|child| show_hidden || !child.name.starts_with('.'))
            .map(|child| entry_of(&child))
            .collect();

        Some(sorted(found))
    }

    /// The path bar: the real folders from the root down, the archive (`inside` is `""`) and each folder inside it.
    fn crumbs(&self) -> Vec<Crumb> {
        let archive = self.archive.to_string_lossy().into_owned();
        let mut crumbs = folder_crumbs(&self.folder);

        crumbs.push(Crumb {
            name: self.name.clone(),
            path: archive.clone(),
            inside: Some(String::new()),
        });

        let mut inside = String::new();
        for name in self.dir.split('/').filter(|name| !name.is_empty()) {
            if !inside.is_empty() {
                inside.push('/');
            }
            inside.push_str(name);

            crumbs.push(Crumb {
                name: name.to_owned(),
                path: archive.clone(),
                inside: Some(inside.clone()),
            });
        }

        crumbs
    }
}

impl VfsFolder for ArchiveFolder {
    fn location(&self) -> PaneLocation {
        PaneLocation::Archive {
            archive: self.archive.to_string_lossy().into_owned(),
            dir: self.dir.clone(),
        }
    }

    fn capabilities(&self) -> PaneCapabilities {
        PaneCapabilities::READ_ONLY
    }

    /// A folder that is not in the archive is a `NotFound` error, before `start`.
    fn list(&self, show_hidden: bool, cancel: &AtomicBool, emit: &mut dyn FnMut(ListEvent)) -> io::Result<Outcome> {
        let entries = self
            .describe(show_hidden)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "This folder is not in the archive."))?;

        let start = ListEvent::Start {
            path: self.folder.to_string_lossy().into_owned(),
            parent: self.folder.parent().map(|parent| parent.to_string_lossy().into_owned()),
            crumbs: self.crumbs(),
            total: count(entries.len()),
            location: self.location(),
            capabilities: self.capabilities(),
        };

        Ok(send_listing(start, &entries, cancel, emit))
    }
}

/// One entry as a pane shows it. Everything in an archive is read-only; a link or a special entry is shown as "other".
fn entry_of(child: &Child<'_>) -> Entry {
    let record = child.entry;
    let kind = if child.dir {
        EntryKind::Dir
    } else if record.is_some_and(|entry| entry.kind == EntryType::File) {
        EntryKind::File
    } else {
        EntryKind::Other
    };

    Entry {
        name: child.name.to_owned(),
        kind,
        size: match kind {
            EntryKind::File => record.map_or(0, |entry| entry.size),
            _ => 0,
        },
        modified: record.and_then(|entry| entry.modified),
        hidden: child.name.starts_with('.'),
        readonly: true,
        mode: record.and_then(|entry| entry.mode).map(|mode| mode & 0o7777),
        link_target: record
            .filter(|entry| entry.kind == EntryType::Symlink)
            .and_then(|entry| entry.link.clone()),
    }
}

/// One crumb per real folder from the root down; the root is named as written (`/`, `C:\`).
fn folder_crumbs(folder: &Path) -> Vec<Crumb> {
    let mut path = PathBuf::new();
    let mut crumbs = Vec::new();

    for component in folder.components() {
        path.push(component);
        // A drive's prefix and the root after it make one crumb (`C:\`).
        if matches!(component, Component::Prefix(_)) {
            continue;
        }

        let shown = path.to_string_lossy().into_owned();
        let name = match component {
            Component::Normal(name) => name.to_string_lossy().into_owned(),
            _ => shown.clone(),
        };
        crumbs.push(Crumb {
            name,
            path: shown,
            inside: None,
        });
    }

    crumbs
}

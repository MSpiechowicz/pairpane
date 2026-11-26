//! A folder on a server, listed through a [`RemoteFs`].
//!
//! The server is read-only here and not watched: [`RemoteFolder::capabilities`] is `READ_ONLY` and `watch` stays the
//! default `None`, so a pane reloads on demand only.
//!
//! The `start` event carries the folder as the server's path in `path`, `None` as `parent` at `/`, and the place as the
//! `location` (`PaneLocation::Remote`). The crumbs are the server's own: the first is named as the server (its label)
//! and stands for `/`, then one per folder, each with `path` set to that folder's path *on the server* and `inside`
//! absent. A crumb is told from a local one by the pane's location (`remote`), not by the crumb: the page opens a
//! remote crumb through the pane's server, never as a folder on disk.
//!
//! What an entry is comes from the listing, except for a symbolic link, whose kind is its target's (`stat`). That is a
//! round trip per link, so only the first [`MAX_LINK_STATS`] links of a listing are looked at; the others, links that
//! point nowhere and links whose target cannot be asked about show as "other". `link_target` is always `None`: reading
//! where a link points would be another round trip for every link, and the pane only needs the kind to open it.
//!
//! Known limit: a listing is not streamed. [`RemoteFs::read_dir`] returns the whole folder, so the pane shows it once it
//! is complete (a big folder on a slow server shows nothing until then), and a cancel is noticed between the pages the
//! server sends, not in the middle of one. Showing entries as the pages arrive is left for later.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{
    RemoteEntry, RemoteFs, RemoteKind, RemoteMeta, RemotePath, VfsFolder, is_remote_name, send_listing, sorted,
};
use crate::listing::{Outcome, count};
use crate::model::{Crumb, Entry, EntryKind, ListEvent, PaneCapabilities, PaneLocation};

/// The most symbolic links of one listing whose target is looked at.
pub const MAX_LINK_STATS: usize = 256;

/// The folder `dir` of the server `fs` speaks to.
pub struct RemoteFolder {
    fs: Arc<dyn RemoteFs>,
    dir: RemotePath,
}

impl RemoteFolder {
    pub fn new(fs: Arc<dyn RemoteFs>, dir: RemotePath) -> Self {
        Self { fs, dir }
    }

    /// The path bar: the server (`/`), then each folder down to this one.
    fn crumbs(&self) -> Vec<Crumb> {
        let mut crumbs = vec![Crumb {
            name: self.fs.label().to_owned(),
            path: RemotePath::root().to_string(),
            inside: None,
        }];

        let mut path = String::new();
        for name in self.dir.components() {
            path.push('/');
            path.push_str(name);

            crumbs.push(Crumb {
                name: name.to_owned(),
                path: path.clone(),
                inside: None,
            });
        }

        crumbs
    }

    /// The entries to show, folders first and then by name. `None` when `cancel` was raised on the way.
    fn describe(&self, show_hidden: bool, cancel: &AtomicBool) -> io::Result<Option<Vec<Entry>>> {
        let listed = match self.fs.read_dir(&self.dir, cancel) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted && cancel.load(Ordering::Relaxed) => {
                return Ok(None);
            }
            other => other?,
        };

        let mut links_left = MAX_LINK_STATS;
        let mut entries = Vec::with_capacity(listed.len());

        for item in listed {
            let hidden = item.name.starts_with('.');
            if !is_remote_name(&item.name) || (hidden && !show_hidden) {
                continue;
            }

            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }

            let meta = if item.meta.kind == RemoteKind::Link {
                self.resolved(&item, &mut links_left)?
            } else {
                item.meta
            };
            entries.push(entry_of(item.name, meta, hidden));
        }

        Ok(Some(sorted(entries)))
    }

    /// What a link shows as: its target's attributes, or its own as "other" when the target is not asked about, is
    /// missing or cannot be told. A lost connection or a time-out ends the listing: the next links would only wait.
    fn resolved(&self, link: &RemoteEntry, links_left: &mut usize) -> io::Result<RemoteMeta> {
        let unresolved = RemoteMeta {
            kind: RemoteKind::Other,
            size: 0,
            ..link.meta
        };

        if *links_left == 0 {
            return Ok(unresolved);
        }
        *links_left -= 1;

        let Ok(path) = self.dir.join(&link.name) else {
            return Ok(unresolved);
        };

        match self.fs.stat(&path) {
            Ok(Some(target)) if matches!(target.kind, RemoteKind::Dir | RemoteKind::File) => Ok(target),
            Ok(_) => Ok(unresolved),
            Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::ConnectionAborted) => {
                Err(error)
            }
            Err(_) => Ok(unresolved),
        }
    }
}

impl VfsFolder for RemoteFolder {
    fn location(&self) -> PaneLocation {
        PaneLocation::Remote {
            site: self.fs.site(),
            label: self.fs.label().to_owned(),
            dir: self.dir.to_string(),
        }
    }

    fn capabilities(&self) -> PaneCapabilities {
        PaneCapabilities::READ_ONLY
    }

    /// A folder the server cannot list is its error (`NotFound`, `PermissionDenied`, `TimedOut`...), before `start`.
    fn list(&self, show_hidden: bool, cancel: &AtomicBool, emit: &mut dyn FnMut(ListEvent)) -> io::Result<Outcome> {
        let Some(entries) = self.describe(show_hidden, cancel)? else {
            return Ok(Outcome::Cancelled);
        };

        let start = ListEvent::Start {
            path: self.dir.to_string(),
            parent: self.dir.parent().map(|parent| parent.to_string()),
            crumbs: self.crumbs(),
            total: count(entries.len()),
            location: self.location(),
            capabilities: self.capabilities(),
        };

        Ok(send_listing(start, &entries, cancel, emit))
    }
}

/// One entry as a pane shows it. A folder has no size; a read-only entry is one the owner may not write.
fn entry_of(name: String, meta: RemoteMeta, hidden: bool) -> Entry {
    let kind = match meta.kind {
        RemoteKind::Dir => EntryKind::Dir,
        RemoteKind::File => EntryKind::File,
        RemoteKind::Link | RemoteKind::Other => EntryKind::Other,
    };

    Entry {
        name,
        kind,
        size: if kind == EntryKind::File { meta.size } else { 0 },
        modified: meta.modified_ms,
        hidden,
        readonly: meta.mode.is_some_and(|mode| mode & 0o200 == 0),
        mode: meta.mode.map(|mode| mode & 0o7777),
        link_target: None,
    }
}

#[cfg(test)]
mod tests;

//! `archive_open`: a folder inside an archive listed into a pane, like `pane_open` lists a folder on disk.
//!
//! The listing is made from the archive's index (built once and kept, see `commands/archives.rs`), so moving between the
//! folders of one archive reads nothing. Nothing is extracted and nothing is watched: an archive is read-only and its
//! file is not followed for changes (the pane's open of another place, or of this one again, replaces the listing). The
//! open takes a pane slot like `pane_open` does, so a later open of the pane supersedes this one: its flag is raised and
//! the index build or the batches stop. Its slot is keyed by the archive's path, never by the folder that holds the
//! archive, so the watch of that folder (kept for the same folder) is dropped and the size jobs of the pane stop.
//!
//! The `start` event carries the folder that holds the archive as `path` (what a tab saves and `..` from the archive's
//! root returns to), the real folders as crumbs, then the archive's own crumb and one crumb per folder inside it (each
//! with `inside` set), and `archive` with the place shown.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::archive::{ArchiveIndex, Child, EntryType};
use pairpane_core::listing::{BATCH, FIRST_BATCH, platform};
use pairpane_core::model::{ArchiveSpot, Crumb, Entry, EntryKind, IpcError, IpcErrorCode, ListEvent, PaneId};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use super::{Claim, Panes, Slots, claim_sizing};
use crate::commands::archives::{Archives, Target, archive_error, check_inner};
use crate::commands::limit::Permit;
use crate::error::CommandResult;

#[cfg(test)]
mod tests;

/// One `archive_open`, ready to run on a blocking thread.
struct Open {
    slots: Slots,
    archives: Archives,
    pane: PaneId,
    generation: u64,
    target: Target,
    dir: String,
    show_hidden: bool,
    cancel: Arc<AtomicBool>,
    on_event: Channel<ListEvent>,
}

/// Lists the folder `dir` (`""` is the root, `a/b` a folder) of the archive file `archive` (absolute) into `pane`. A
/// folder that is not in the archive, a password the archive needs, a missing `unrar` and a file that is not an archive
/// arrive on `on_event` as an error, after the arguments are checked.
#[tauri::command]
pub async fn archive_open(
    panes: State<'_, Panes>,
    archives: State<'_, Archives>,
    pane: PaneId,
    archive: String,
    dir: String,
    show_hidden: bool,
    on_event: Channel<ListEvent>,
) -> CommandResult<()> {
    start(&panes, &archives, pane, archive, dir, show_hidden, on_event)
}

/// Checks the arguments, takes the pane's slot and starts the listing on a blocking thread.
fn start(
    panes: &Panes,
    archives: &Archives,
    pane: PaneId,
    archive: String,
    dir: String,
    show_hidden: bool,
    on_event: Channel<ListEvent>,
) -> CommandResult<()> {
    let target = Target::parse(archive)?;
    check_inner(&dir)?;
    // The slot is keyed by the archive's path, never by the folder that holds it: that folder's watch is not kept.
    let Claim {
        permit,
        generation,
        cancel,
        ..
    } = panes.claim(pane, &target.path, &on_event)?;

    let open = Open {
        slots: Arc::clone(&panes.slots),
        archives: archives.clone(),
        pane,
        generation,
        target,
        dir,
        show_hidden,
        cancel,
        on_event,
    };
    // The permit is held until the listing ends, however it ends.
    async_runtime::spawn_blocking(move || open.run(permit));

    Ok(())
}

impl Open {
    fn run(self, permit: Permit) {
        let _permit = permit;

        let result = self.list();
        match result {
            Ok(()) => {}
            // A listing that was superseded stops quietly; the pane is showing the newer one.
            Err(_) if self.cancel.load(Ordering::Relaxed) => {}
            Err(error) => {
                self.on_event.send(ListEvent::Error { error }).ok();
            }
        }
    }

    fn list(&self) -> Result<(), IpcError> {
        let opened = self
            .archives
            .open(&self.target)
            .map_err(|error| archive_error(&error))?;
        let index = self
            .archives
            .index(&self.target, &opened, &self.cancel, None)
            .map_err(|error| archive_error(&error))?;

        let started = || claim_sizing(&self.slots, self.pane, self.generation, &self.target.path);
        // A closed channel only means the page went away; the next open starts over.
        let send = |event: ListEvent| {
            self.on_event.send(event).ok();
        };

        list_archive(
            &index,
            &self.target,
            &self.dir,
            self.show_hidden,
            &self.cancel,
            started,
            send,
        )
    }
}

/// Sends the listing of the folder `dir` of `index` as `start`, the `batch`es and `done`, calling `on_start` just before
/// `start`; stops quietly when `cancel` is raised. A folder that is not in the archive is a `NotFound` error.
fn list_archive(
    index: &ArchiveIndex,
    target: &Target,
    dir: &str,
    show_hidden: bool,
    cancel: &AtomicBool,
    on_start: impl FnOnce(),
    mut emit: impl FnMut(ListEvent),
) -> Result<(), IpcError> {
    let entries = describe(index, dir, show_hidden)
        .ok_or_else(|| IpcError::new(IpcErrorCode::NotFound, "This folder is not in the archive."))?;

    on_start();
    emit(ListEvent::Start {
        path: target.folder.to_string_lossy().into_owned(),
        parent: target
            .folder
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        crumbs: crumbs(target, dir),
        total: count(entries.len()),
        archive: Some(ArchiveSpot {
            archive: target.path.to_string_lossy().into_owned(),
            dir: dir.to_owned(),
        }),
    });

    let mut offset = 0;

    while offset < entries.len() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let size = if offset == 0 { FIRST_BATCH } else { BATCH };
        let chunk = &entries[offset..entries.len().min(offset + size)];
        emit(ListEvent::Batch {
            offset: count(offset),
            entries: chunk.to_vec(),
        });
        offset += chunk.len();
    }

    let dirs = entries.iter().filter(|entry| entry.kind == EntryKind::Dir).count();
    let bytes = entries
        .iter()
        .filter(|entry| entry.kind != EntryKind::Dir)
        .fold(0_u64, |sum, entry| sum.saturating_add(entry.size));
    emit(ListEvent::Done {
        dirs: count(dirs),
        files: count(entries.len() - dirs),
        bytes,
        free: None,
    });

    Ok(())
}

/// The entries of the folder `dir`, folders first and then by name (the rules of a folder listing); hidden ones (a name
/// with a leading dot) only when `show_hidden`. `None` when `dir` is not a folder of the archive.
fn describe(index: &ArchiveIndex, dir: &str, show_hidden: bool) -> Option<Vec<Entry>> {
    let mut found: Vec<(bool, String, Entry)> = index
        .list(dir)?
        .into_iter()
        .filter(|child| show_hidden || !child.name.starts_with('.'))
        .map(|child| (child.dir, platform::sort_key(child.name), entry_of(&child)))
        .collect();

    found.sort_by(|(a_dir, a_key, a), (b_dir, b_key, b)| {
        b_dir
            .cmp(a_dir)
            .then_with(|| platform::compare(a_key, &a.name, b_key, &b.name))
    });

    Some(found.into_iter().map(|(_, _, entry)| entry).collect())
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

/// The path bar: the real folders from the root down, the archive (`inside` is `""`) and each folder inside it.
fn crumbs(target: &Target, dir: &str) -> Vec<Crumb> {
    let archive = target.path.to_string_lossy().into_owned();
    let mut crumbs = folder_crumbs(&target.folder);

    crumbs.push(Crumb {
        name: target.name.clone(),
        path: archive.clone(),
        inside: Some(String::new()),
    });

    let mut inside = String::new();
    for name in dir.split('/').filter(|name| !name.is_empty()) {
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

/// Entry counts are capped at `u32::MAX` on the wire.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

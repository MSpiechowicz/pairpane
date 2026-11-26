//! What a pane lists: one folder of some kind of place, behind one interface.
//!
//! A pane's listing is a sequence of [`ListEvent`]s (see there); [`VfsFolder`] is whatever produces one: a folder on disk
//! ([`LocalFolder`]) or a folder inside an archive ([`ArchiveFolder`]). The shell opens a folder through the trait
//! alone, so what a place can do (see [`PaneCapabilities`]) is the place's own answer, not something the shell
//! guesses from the kind of path it was given.

mod archive;
mod local;
mod remote;
mod remote_folder;

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub use archive::ArchiveFolder;
pub use local::LocalFolder;
pub use remote::{
    MAX_REMOTE_PATH, RemoteEntry, RemoteFs, RemoteKind, RemoteMeta, RemotePath, RemoteReader, is_remote_name,
};
pub use remote_folder::{MAX_LINK_STATS, RemoteFolder};

use crate::listing::{BATCH, FIRST_BATCH, FolderWatch, Outcome, count, platform};
use crate::model::{Entry, EntryKind, ListEvent, PaneCapabilities, PaneLocation};

/// One folder of a place a pane can show, ready to list.
pub trait VfsFolder: Send {
    /// Where this folder is, as the `start` event of its listing says.
    fn location(&self) -> PaneLocation;

    /// What a pane may do with its entries.
    fn capabilities(&self) -> PaneCapabilities;

    /// Sends the listing as `start`, the `batch`es and `done` to `emit`, and stops early (no `done`) once `cancel` is
    /// set; hidden entries are left out unless `show_hidden`. Fails, before `start`, only when the folder itself
    /// cannot be listed.
    fn list(&self, show_hidden: bool, cancel: &AtomicBool, emit: &mut dyn FnMut(ListEvent)) -> io::Result<Outcome>;

    /// Calls `on_change` (on the watch's own thread) when the folder changes, for as long as the watch lives. `None`
    /// for a place that is not watched.
    fn watch(&self, _on_change: Box<dyn Fn() + Send + 'static>) -> Option<notify::Result<FolderWatch>> {
        None
    }
}

/// Sends `start` (whose `total` is `entries.len()`), `entries` in `batch`es and `done` (in the folder listing's sizes
/// of batch); stops quietly, without `done`, when `cancel` is raised. Shared by every listing that is made from a list.
pub fn send_listing(
    start: ListEvent,
    entries: &[Entry],
    cancel: &AtomicBool,
    mut emit: impl FnMut(ListEvent),
) -> Outcome {
    emit(start);

    let mut offset = 0;

    while offset < entries.len() {
        if cancel.load(Ordering::Relaxed) {
            return Outcome::Cancelled;
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

    Outcome::Done
}

/// `entries` in the order of a folder listing: folders first, then by name (the platform's rules).
pub fn sorted(entries: Vec<Entry>) -> Vec<Entry> {
    let mut keyed: Vec<(String, Entry)> = entries
        .into_iter()
        .map(|entry| (platform::sort_key(&entry.name), entry))
        .collect();

    keyed.sort_by(|(a_key, a), (b_key, b)| {
        (b.kind == EntryKind::Dir)
            .cmp(&(a.kind == EntryKind::Dir))
            .then_with(|| platform::compare(a_key, &a.name, b_key, &b.name))
    });

    keyed.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests;

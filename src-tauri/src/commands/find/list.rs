//! `find_open`: the results of a search listed into a pane, like `pane_open` lists a folder on disk.
//!
//! The listing is made from the hits the finder kept: each one on the filesystem is read again now (a file that is gone
//! is left out; a hit inside an archive has no place in a folder and is left out too), named by its path below the
//! folder searched, and sorted as a folder's entries are. The open takes a pane slot like `archive_open` does, so a later
//! open of the pane supersedes this one, and its slot is keyed by a path no folder has: nothing is watched and the pane's
//! size jobs stop.
//!
//! The `start` event carries the folder that was searched as `path` and as `parent` (so that going up from the results
//! goes to the folder searched, not to the folder above it), its crumbs, and the search's id as `results`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::find as core;
use pairpane_core::listing::crumbs;
use pairpane_core::model::{Entry, FindHit, FindId, ListEvent, PaneId};
use tauri::State;
use tauri::ipc::Channel;

use super::Finder;
use crate::commands::items::blocking;
use crate::commands::panes::archive::{count, send_listing, sorted};
use crate::commands::panes::{Claim, Panes};
use crate::error::CommandResult;

#[cfg(test)]
mod tests;

/// Lists the results the search `id` kept so far into `pane` and answers when the listing is sent. The pane then shows
/// the folder searched with every entry named by its path below it. A search that is not kept is `notFound`.
#[tauri::command]
pub async fn find_open(
    panes: State<'_, Panes>,
    finder: State<'_, Finder>,
    pane: PaneId,
    id: FindId,
    on_event: Channel<ListEvent>,
) -> CommandResult<()> {
    open(&panes, &finder, pane, id, on_event).await
}

/// Takes the pane's slot, and reads the hits and sends the listing on a blocking thread, which it waits for.
async fn open(
    panes: &Panes,
    finder: &Finder,
    pane: PaneId,
    id: FindId,
    on_event: Channel<ListEvent>,
) -> CommandResult<()> {
    let search = finder.get(id)?;
    let Claim {
        permit,
        generation,
        cancel,
        ..
    } = panes.claim_results(pane, id, &on_event)?;

    let hits = search.snapshot();
    let started = panes.results_listed(pane, generation, id);
    let root = search.root.clone();

    // The permit is held until the listing ends, however it ends.
    blocking(permit, move || {
        // A closed channel only means the page went away; the next open starts over.
        let send = |event: ListEvent| {
            on_event.send(event).ok();
        };

        list_results(&root, &hits, id, &cancel, started, send);
    })
    .await
}

/// Sends the listing of `hits` (found in `root`) as `start`, the `batch`es and `done`, calling `on_start` just before
/// `start`; stops quietly when `cancel` is raised.
fn list_results(
    root: &Path,
    hits: &[FindHit],
    id: FindId,
    cancel: &AtomicBool,
    on_start: impl FnOnce(),
    emit: impl FnMut(ListEvent),
) {
    let entries = present(root, hits, cancel);
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    on_start();
    let start = ListEvent::Start {
        path: root.to_string_lossy().into_owned(),
        parent: Some(root.to_string_lossy().into_owned()),
        crumbs: crumbs(root),
        total: count(entries.len()),
        archive: None,
        results: Some(id),
    };

    send_listing(start, &entries, cancel, emit);
}

/// The entries of `hits` that are still there, in the order of a folder listing; stops reading when `cancel` is raised.
fn present(root: &Path, hits: &[FindHit], cancel: &AtomicBool) -> Vec<Entry> {
    let found = hits
        .iter()
        .take_while(|_| !cancel.load(Ordering::Relaxed))
        .filter_map(|hit| core::entry(root, hit))
        .collect();

    sorted(found)
}

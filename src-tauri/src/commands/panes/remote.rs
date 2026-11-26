//! `remote_open`: a folder of a connected server listed into a pane, like `pane_open` lists a folder on disk.
//!
//! The arguments are checked first (`dir` must be an absolute path of the server's, `child` one plain name), then the pane's
//! slot is taken like `pane_open` does, so a later open of the pane supersedes this one: its flag is raised and the listing
//! stops between the pages the server sends. The command answers at once; the site's file system is looked up and the
//! listing is made on a blocking thread that holds a place of the pane's listing cap, so a server that does not answer
//! (each request ends within its time limit) never blocks the window nor takes more than the cap's threads. Everything
//! after the arguments are checked, a site that is gone and a folder that cannot be read included, arrives on `on_event`.
//!
//! Nothing is watched and no size is measured on a server: the slot's key has no folder to measure, so listing a
//! server's folder stops the pane's size jobs. The folder itself is `pairpane_core::vfs::RemoteFolder`, which makes the
//! `start` event (see there).

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::model::{IpcErrorCode, ListEvent, PaneId, SiteId};
use pairpane_core::vfs::{RemoteFolder, RemoteFs, RemotePath};
use tauri::State;
use tauri::async_runtime::{self, JoinHandle};
use tauri::ipc::Channel;

use super::run::list;
use super::{Claim, Panes, SlotKey, Slots, claim_sizing, list_error};
use crate::commands::remote::Remotes;
use crate::error::{CommandError, CommandResult};

#[cfg(test)]
mod tests;

/// Looks up the file system of the site on the listing's thread: it takes the pool's lock, which a command's thread does
/// not wait on.
type Lookup = Box<dyn FnOnce() -> io::Result<Arc<dyn RemoteFs>> + Send>;

/// One `remote_open`, ready to run on a blocking thread.
struct Open {
    slots: Slots,
    pane: PaneId,
    generation: u64,
    site: SiteId,
    dir: RemotePath,
    show_hidden: bool,
    cancel: Arc<AtomicBool>,
    on_event: Channel<ListEvent>,
}

/// Lists the folder `dir` (an absolute path on the server) of the site `site` into `pane`, or with `child` the folder of
/// that name inside it. It answers once the arguments are checked; the listing, and what fails after that, arrives on
/// `on_event`.
#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "the arguments of the command, as the page names them"
)]
pub async fn remote_open(
    panes: State<'_, Panes>,
    remotes: State<'_, Remotes>,
    pane: PaneId,
    site: SiteId,
    dir: String,
    child: Option<String>,
    show_hidden: bool,
    on_event: Channel<ListEvent>,
) -> CommandResult<()> {
    let dir = target(&dir, child.as_deref())?;
    let pool = remotes.pool()?;

    start(
        &panes,
        pane,
        site,
        dir,
        show_hidden,
        on_event,
        Box::new(move || pool.fs(site)),
    )
    .map(drop)
}

/// The folder to list: `dir` on the server, plus `child` when given.
fn target(dir: &str, child: Option<&str>) -> CommandResult<RemotePath> {
    let invalid = |message: &'static str| CommandError::new(IpcErrorCode::InvalidInput, message);

    let dir = RemotePath::new(dir).map_err(|_| invalid("a folder of a server must be an absolute path"))?;

    match child {
        None => Ok(dir),
        Some(child) => dir
            .join(child)
            .map_err(|_| invalid("a folder name must be a single plain name")),
    }
}

/// Takes the pane's slot and lists `dir` on a blocking thread, which the handle is for. Refused as busy while the pane
/// has all its listing threads outstanding.
fn start(
    panes: &Panes,
    pane: PaneId,
    site: SiteId,
    dir: RemotePath,
    show_hidden: bool,
    on_event: Channel<ListEvent>,
    lookup: Lookup,
) -> CommandResult<JoinHandle<()>> {
    let Claim {
        permit,
        generation,
        cancel,
        ..
    } = panes.claim(pane, SlotKey::Remote { site, dir: dir.clone() }, &on_event)?;

    let open = Open {
        slots: Arc::clone(&panes.slots),
        pane,
        generation,
        site,
        dir,
        show_hidden,
        cancel,
        on_event,
    };

    Ok(async_runtime::spawn_blocking(move || {
        // The permit is held until the listing ends, however it ends.
        let _permit = permit;

        open.run(lookup);
    }))
}

impl Open {
    fn run(self, lookup: Lookup) {
        let fs = match lookup() {
            Ok(fs) => fs,
            // A listing that was superseded is not told about a site that went away: the pane shows the newer one.
            Err(_) if self.cancel.load(Ordering::Relaxed) => return,
            Err(error) => {
                self.on_event
                    .send(ListEvent::Error {
                        error: list_error(&error),
                    })
                    .ok();

                return;
            }
        };

        let key = SlotKey::Remote {
            site: self.site,
            dir: self.dir.clone(),
        };
        let folder = RemoteFolder::new(fs, self.dir);
        let started = || claim_sizing(&self.slots, self.pane, self.generation, &key);

        list(&folder, self.show_hidden, &self.cancel, &self.on_event, started);
    }
}

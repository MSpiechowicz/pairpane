//! `archive_open`: a folder inside an archive listed into a pane, like `pane_open` lists a folder on disk.
//!
//! The listing is made from the archive's index (built once and kept, see `commands/archives.rs`), so moving between the
//! folders of one archive reads nothing. Nothing is extracted and nothing is watched: an archive is read-only and its
//! file is not followed for changes (the pane's open of another place, or of this one again, replaces the listing). The
//! open takes a pane slot like `pane_open` does, so a later open of the pane supersedes this one: its flag is raised and
//! the index build or the batches stop. Its slot is keyed by the archive's path, never by the folder that holds the
//! archive, so the watch of that folder (kept for the same folder) is dropped and the size jobs of the pane stop.
//!
//! The folder itself is `pairpane_core::vfs::ArchiveFolder`, which makes the `start` event (see there).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::model::{IpcError, IpcErrorCode, ListEvent, PaneId};
use pairpane_core::vfs::{ArchiveFolder, VfsFolder};
use tauri::State;
use tauri::ipc::Channel;

use super::run::claiming_sizes;
use super::{Claim, Panes, SlotKey, Slots};
use crate::commands::archives::{Archives, Target, archive_error, check_inner};
use crate::commands::items::blocking;
use crate::commands::ops::failed;
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

/// Lists the folder `dir` (`""` is the root, `a/b` a folder) of the archive file `archive` (absolute) into `pane`, and
/// answers when the listing is sent. A folder that is not in the archive, a password the archive needs (`passwordRequired`),
/// a missing `unrar` and a file that is not an archive reject the command, once the arguments are checked.
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
    start(&panes, &archives, pane, archive, dir, show_hidden, on_event).await
}

/// Checks the arguments, takes the pane's slot, and builds the index and sends the listing on a blocking thread, which
/// it waits for.
async fn start(
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
    } = panes.claim(pane, SlotKey::Archive(target.path.clone()), &on_event)?;

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
    blocking(permit, move || open.run()).await?.map_err(failed)
}

impl Open {
    fn run(self) -> Result<(), IpcError> {
        match self.list() {
            // A listing that was superseded stops quietly; the pane is showing the newer one.
            Err(_) if self.cancel.load(Ordering::Relaxed) => Ok(()),
            result => result,
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
            .map_err(|error| self.archives.report(&self.target, &error))?;
        let folder = ArchiveFolder::new(
            index,
            self.target.folder.clone(),
            self.target.name.clone(),
            self.dir.clone(),
        );

        let mut send = claiming_sizes(
            &self.slots,
            self.pane,
            self.generation,
            SlotKey::Archive(self.target.path.clone()),
            &self.on_event,
        );

        // The folder is not in the archive: nothing was sent, and the command rejects.
        folder
            .list(self.show_hidden, &self.cancel, &mut send)
            .map(drop)
            .map_err(|error| IpcError::new(IpcErrorCode::NotFound, error.to_string()))
    }
}

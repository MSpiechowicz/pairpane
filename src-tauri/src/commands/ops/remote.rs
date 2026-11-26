//! `ops_remote`: a copy or move between a folder of a connected server and a folder on this computer, as a job of the
//! transfer engine. Only the download (server to computer) is supported for now.
//!
//! Like `ops_start` it checks the arguments for syntax and touches nothing on disk: the local folder passes
//! `check_folder`, the server's folder is an absolute path and the names are plain names a server may hold. The site's
//! file system is looked up on a blocking thread (it takes the pool's lock); a site that is gone is a fixed error, never
//! a job that fails later. The engine does the rest, on its own worker.

use std::sync::Arc;

use pairpane_core::model::{IpcErrorCode, JobId, OpKind, SiteId, TransferPlace};
use pairpane_core::ops::Request;
use pairpane_core::vfs::{RemoteFs, RemotePath, is_remote_name};
use tauri::State;
use tauri::async_runtime::spawn_blocking;

use super::{Ops, failed, unique_names, unnamed};
use crate::commands::panes::resolve;
use crate::commands::remote::Remotes;
use crate::error::{CommandError, CommandResult};

/// Queues a copy or move (`kind`) of the `names` between the folders `from` and `to`, one a folder on a server and the
/// other a folder on this computer, and returns the job's id at once. Only a download is possible for now: `from` on a
/// server, `to` on this computer. Anything else is refused: both on this computer is invalid input, and a transfer to a
/// server, or between two servers, is unsupported. A site that is not connected is refused with a fixed error.
#[tauri::command]
pub async fn ops_remote(
    ops: State<'_, Ops>,
    remotes: State<'_, Remotes>,
    kind: OpKind,
    from: TransferPlace,
    to: TransferPlace,
    names: Vec<String>,
    verify: bool,
) -> CommandResult<JobId> {
    ops.remote(&remotes, kind, from, to, names, verify).await
}

impl Ops {
    /// Checks the request, looks up the site's file system and queues the download.
    pub(super) async fn remote(
        &self,
        remotes: &Remotes,
        kind: OpKind,
        from: TransferPlace,
        to: TransferPlace,
        names: Vec<String>,
        verify: bool,
    ) -> CommandResult<JobId> {
        if !matches!(kind, OpKind::Copy | OpKind::Move) {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "only a copy or a move goes between a server and this computer",
            ));
        }
        let (site, dir, to) = download(from, to)?;
        let dir = RemotePath::new(&dir).map_err(|_| {
            CommandError::new(
                IpcErrorCode::InvalidInput,
                "a folder of a server must be an absolute path",
            )
        })?;
        let to = resolve(Some(to), None)?;
        let names = unique_names(names, |name| if is_remote_name(name) { Ok(()) } else { Err(unnamed()) })?;
        let fs = fs_of(remotes, site).await?;

        self.engine
            .submit(Request::download(kind, fs, dir, names, to, verify))
            .map_err(failed)
    }
}

/// The server's folder (`site`, `dir`) and the folder on this computer of a download; the other pairs are refused.
fn download(from: TransferPlace, to: TransferPlace) -> CommandResult<(SiteId, String, String)> {
    match (from, to) {
        (TransferPlace::Remote { site, dir }, TransferPlace::Folder { path }) => Ok((site, dir, path)),
        (TransferPlace::Folder { .. }, TransferPlace::Remote { .. }) => Err(CommandError::new(
            IpcErrorCode::Unsupported,
            "copying to a server comes in a later version",
        )),
        (TransferPlace::Remote { .. }, TransferPlace::Remote { .. }) => Err(CommandError::new(
            IpcErrorCode::Unsupported,
            "copying from one server to another is not supported",
        )),
        (TransferPlace::Folder { .. }, TransferPlace::Folder { .. }) => Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "one of the folders must be on a server",
        )),
    }
}

/// The file system of `site`, looked up on a blocking thread. Its error is the pool's fixed text.
async fn fs_of(remotes: &Remotes, site: SiteId) -> CommandResult<Arc<dyn RemoteFs>> {
    let pool = remotes.pool()?;
    let found = spawn_blocking(move || pool.fs(site))
        .await
        .map_err(|_| CommandError::new(IpcErrorCode::Internal, "the server could not be looked up"))?;

    found.map_err(|error| CommandError::new(IpcErrorCode::Io, format!("Cannot reach this server: {error}")))
}

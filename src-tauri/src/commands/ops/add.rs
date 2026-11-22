//! `ops_add_to_archive`: files added to an archive as a job of the transfer engine, which writes a new archive and replaces
//! the old one when it is complete.
//!
//! The command is `ops_extract`'s counterpart and shares its preparation ([`load`]): the index kept for the archive (built
//! now when missing, for every format, as the checks below need it), the state of the file it was made from, and the
//! password the user gave earlier, which the new entries are encrypted with. What it refuses before the job starts: an
//! archive that is not written (a rar, a single compressed file), one whose index is not whole (too large, or with names
//! that are refused: the rewrite could not keep them), a folder that is not in the archive, and, for an archive with
//! encrypted entries, a missing password (`passwordRequired`, so the page asks first). The job goes through
//! [`Extractions`](super::extractions::Extractions), which forgets a wrong password and, when the job ends, the index kept
//! for the archive (the file may be a new one, and the page reads it again).

use pairpane_core::archive::{self, PasswordFault};
use pairpane_core::model::{ArchiveFormat, IpcError, IpcErrorCode, JobId};
use pairpane_core::ops::{ArchiveJob, Request, Task};
use tauri::State;

use super::extract::{check_folders, load};
use super::{Ops, checked_names, failed};
use crate::commands::archives::{Archives, Target, archive_error, check_inner};
use crate::commands::items::blocking;
use crate::commands::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// Adds the `names` of the folder `from` to the folder `dir` of the archive file `archive` (absolute; `dir` is `""` for
/// its root, otherwise `a/b`) and returns the job's id at once. Fails with `passwordRequired` when the archive has
/// encrypted entries and the app has no password for it.
#[tauri::command]
pub async fn ops_add_to_archive(
    ops: State<'_, Ops>,
    archives: State<'_, Archives>,
    archive: String,
    dir: String,
    from: String,
    names: Vec<String>,
) -> CommandResult<JobId> {
    ops.add_to_archive(&archives, archive, dir, from, names).await
}

impl Ops {
    /// Checks the request, prepares what the job brings of the archive (on a blocking thread) and queues it.
    pub(super) async fn add_to_archive(
        &self,
        archives: &Archives,
        archive: String,
        dir: String,
        from: String,
        names: Vec<String>,
    ) -> CommandResult<JobId> {
        let target = Target::parse(archive)?;
        check_inner(&dir)?;
        check_folders(&dir)?;
        check_writable(&target)?;
        let from = resolve(Some(from), None)?;
        let names = checked_names(&from, names)?;

        let permit = archives.begin()?;
        let shared = archives.clone();
        let (folder, name, path) = (target.folder.clone(), target.name.clone(), target.path.clone());
        let inner = dir.clone();
        let source = blocking(permit, move || prepare(&shared, &target, &inner))
            .await?
            .map_err(failed)?;

        let password = source.password.clone();
        let task = Task::AddToArchive {
            name,
            inner: dir,
            source,
        };

        self.extractions.submit_rewrite(archives, path, password, || {
            self.engine
                .submit(Request::task(from, names, folder, task))
                .map_err(failed)
        })
    }
}

/// Refuses an archive of a format that is not written, by its extension (the check needs no disk).
fn check_writable(target: &Target) -> CommandResult<()> {
    match archive::by_extension(&target.name) {
        None | Some(ArchiveFormat::Rar | ArchiveFormat::Gz) => Err(CommandError::new(
            IpcErrorCode::Unsupported,
            "This archive can't be changed",
        )),
        Some(_) => Ok(()),
    }
}

/// What the job brings of the archive, after the checks that need its index (see the module documentation). Opens the
/// archive, so it runs on a blocking thread.
fn prepare(archives: &Archives, target: &Target, dir: &str) -> Result<ArchiveJob, IpcError> {
    let source = load(archives, target, true)?;
    let Some(index) = &source.index else {
        return Err(IpcError::new(IpcErrorCode::Internal, "the archive could not be listed"));
    };

    if index.truncated() {
        return Err(IpcError::new(
            IpcErrorCode::Unsupported,
            "This archive is too large to be changed",
        ));
    }
    if index.refused() > 0 {
        return Err(IpcError::new(
            IpcErrorCode::Unsupported,
            "This archive has names that can't be kept, so it can't be changed",
        ));
    }
    if !index.has_dir(dir) {
        return Err(IpcError::new(
            IpcErrorCode::NotFound,
            "the folder is not in the archive",
        ));
    }

    let encrypted = index.select("", &[]).is_some_and(|all| all.any_encrypted());
    if encrypted && source.password.is_none() {
        return Err(archive_error(&archive::password_error(PasswordFault::Needed)));
    }

    Ok(source)
}

#[cfg(test)]
mod tests;

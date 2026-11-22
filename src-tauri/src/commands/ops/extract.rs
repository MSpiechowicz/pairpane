//! `ops_extract`: the entries of an archive extracted into a folder as a job of the transfer engine.
//!
//! The archive and the folders are checked for syntax here, like `ops_start` does, and the engine does the rest. What
//! the command adds is the archive's own state: the index kept for it (so the job does not read the archive again, and
//! the engine checks the file is still the one it was made from), the password the user gave (it travels inside the job
//! and is never sent again by the page), and a check before the job starts for the formats whose directory is cheap to
//! read (zip and 7z): when the chosen entries are encrypted and no password was given, the command fails with
//! `passwordRequired`, so the page asks first and the job never starts only to stop.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use pairpane_core::archive::{self, PasswordFault};
use pairpane_core::listing::platform;
use pairpane_core::model::{ArchiveFormat, IpcError, IpcErrorCode, JobId};
use pairpane_core::ops::{ArchiveJob, MAX_NAMES, MAX_NAMES_BYTES, Request, Task, name_fits};
use tauri::State;

use super::{Ops, failed, names_too_large};
use crate::commands::archives::{Archives, Target, archive_error, check_inner};
use crate::commands::items::blocking;
use crate::commands::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// Extracts the `names` of the folder `dir` of the archive file `archive` (absolute; `dir` is `""` for its root, otherwise
/// `a/b`; no names means everything in `dir`), with all that is below the folders among them, into the folder `to`, and
/// returns the job's id at once. Fails with `passwordRequired` when the entries need a password the app does not have.
#[tauri::command]
pub async fn ops_extract(
    ops: State<'_, Ops>,
    archives: State<'_, Archives>,
    archive: String,
    dir: String,
    names: Vec<String>,
    to: String,
) -> CommandResult<JobId> {
    ops.extract(&archives, archive, dir, names, to).await
}

impl Ops {
    /// Checks the request, prepares what the job brings of the archive (on a blocking thread) and queues it.
    pub(super) async fn extract(
        &self,
        archives: &Archives,
        archive: String,
        dir: String,
        names: Vec<String>,
        to: String,
    ) -> CommandResult<JobId> {
        let target = Target::parse(archive)?;
        check_inner(&dir)?;
        check_folders(&dir)?;
        let to = resolve(Some(to), None)?;
        let names = checked_entries(names)?;

        let permit = archives.begin()?;
        let shared = archives.clone();
        let (folder, name) = (target.folder.clone(), target.name.clone());
        let (inner, chosen) = (dir.clone(), names.clone());
        let source = blocking(permit, move || prepare(&shared, &target, &inner, &chosen))
            .await?
            .map_err(failed)?;

        let task = Task::Extract {
            inner: dir,
            entries: names,
            source,
        };

        self.engine
            .submit(Request::task(folder, vec![name], to, task))
            .map_err(failed)
    }
}

/// What the job brings of the archive: the index (kept, or built now for a zip or 7z, which are read from their
/// directory), the state of the file it was made from and the password. Opens the archive, so it runs on a blocking
/// thread. Fails with `passwordRequired` when the chosen entries are encrypted and there is no password, or when the
/// archive's directory turns down the one there is (which is then forgotten, so the page asks again). A password that
/// only fails later, while the job decrypts, ends the job with `passwordRequired` and is not forgotten here.
fn prepare(archives: &Archives, target: &Target, dir: &str, names: &[String]) -> Result<ArchiveJob, IpcError> {
    let opened = archives.open(target).map_err(|error| archive_error(&error))?;
    let password = archives.password(&target.path);

    let index = match archives.cached(&opened) {
        Some(index) => Some(index),
        None => {
            let format = archives
                .format(target, &opened)
                .map_err(|error| archive_error(&error))?;

            // A compressed tar is decoded by the job itself, which can be cancelled and shows its progress.
            if matches!(format, ArchiveFormat::Zip | ArchiveFormat::SevenZ) {
                let never = AtomicBool::new(false);

                Some(
                    archives
                        .index(target, &opened, &never, None)
                        .map_err(|error| archives.report(target, &error))?,
                )
            } else {
                None
            }
        }
    };

    if let Some(index) = &index {
        let selection = index
            .select(dir, names)
            .ok_or_else(|| IpcError::new(IpcErrorCode::NotFound, "the folder or item is not in the archive"))?;

        if password.is_none() && selection.any_encrypted() {
            return Err(archive_error(&archive::password_error(PasswordFault::Needed)));
        }
    }

    Ok(ArchiveJob {
        index,
        fingerprint: Some(opened.fingerprint),
        password,
    })
}

/// A folder inside an archive: every part is a plain name that fits.
fn check_folders(dir: &str) -> CommandResult<()> {
    if dir.is_empty() {
        return Ok(());
    }

    for part in dir.split('/') {
        if !name_fits(part) {
            return Err(names_too_large());
        }
        if !platform::is_plain_name(part) {
            return Err(invalid_entry());
        }
    }

    Ok(())
}

/// The names of the entries to extract, inside the folder chosen: plain names that fit (the names together take at most
/// [`MAX_NAMES_BYTES`]), duplicates dropped (the first kept), at most [`MAX_NAMES`]. None is allowed: it means everything.
fn checked_entries(names: Vec<String>) -> CommandResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    let mut bytes = 0_usize;

    for name in names {
        bytes = bytes.saturating_add(name.len());
        if !name_fits(&name) || bytes > MAX_NAMES_BYTES {
            return Err(names_too_large());
        }
        if !platform::is_plain_name(&name) {
            return Err(invalid_entry());
        }

        if seen.contains(&name) {
            continue;
        }
        if unique.len() == MAX_NAMES {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                format!("choose at most {MAX_NAMES} different items"),
            ));
        }
        seen.insert(name.clone());
        unique.push(name);
    }

    Ok(unique)
}

fn invalid_entry() -> CommandError {
    CommandError::new(IpcErrorCode::InvalidInput, "an item name must be a single plain name")
}

#[cfg(test)]
mod tests;

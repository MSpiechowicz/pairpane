//! `ops_pack`: the items of a folder packed into a new archive as a job of the transfer engine.
//!
//! Like `ops_start`, the command checks syntax only and touches no disk: the archive's path passes [`Target::parse`], its
//! extension must be the one of the format chosen (the engine writes the format asked for, and a name that says another
//! would be opened as that), and the folder and the names pass the checks every job has. The password travels inside the
//! job, wrapped in a [`Password`] as soon as it arrives, and is never kept by the app: a pack job is the only one that
//! writes with it, and the page asks again for the next archive.

use std::sync::Arc;

use pairpane_core::archive;
use pairpane_core::model::{ArchiveFormat, IpcErrorCode, JobId, PackFormat, PackLevel};
use pairpane_core::ops::{PackJob, Request, Task};
use tauri::State;

use super::{Ops, checked_names, failed};
use crate::commands::archives::{Target, checked_password};
use crate::commands::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// Packs the `names` of the folder `from` into the new archive `archive` (an absolute path, with the extension of the
/// `format`) and returns the job's id at once. An empty `password` means none; a tar.zst cannot have one. `encrypt_names`
/// only counts for a 7z with a password. Nothing is read from the disk here: a taken name is asked about by the job.
#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "the arguments are the page's, one for each choice of the dialog"
)]
pub async fn ops_pack(
    ops: State<'_, Ops>,
    from: String,
    names: Vec<String>,
    archive: String,
    format: PackFormat,
    level: PackLevel,
    password: Option<String>,
    encrypt_names: bool,
) -> CommandResult<JobId> {
    let options = pack_job(format, level, password, encrypt_names)?;
    let request = pack_request(from, names, archive, options)?;

    ops.engine.submit(request).map_err(failed)
}

/// How the archive is written: a password of an empty text is none; a tar.zst has no encryption; names are encrypted only
/// in a 7z with a password (asking for it elsewhere means nothing, so it is dropped, not refused).
fn pack_job(
    format: PackFormat,
    level: PackLevel,
    password: Option<String>,
    encrypt_names: bool,
) -> CommandResult<PackJob> {
    let password = password
        .filter(|text| !text.is_empty())
        .map(checked_password)
        .transpose()?;

    if password.is_some() && format == PackFormat::TarZst {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "a tar.zst archive cannot have a password",
        ));
    }
    let encrypt_names = encrypt_names && format == PackFormat::SevenZ && password.is_some();

    Ok(PackJob {
        format,
        level,
        password: password.map(Arc::new),
        encrypt_names,
    })
}

/// Checks the request from the webview and builds the engine's: the archive goes into its folder, the items come from
/// `from`.
fn pack_request(from: String, names: Vec<String>, archive: String, options: PackJob) -> CommandResult<Request> {
    let target = Target::parse(archive)?;

    if archive::by_extension(&target.name) != Some(archive_format(options.format)) {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "the archive's extension does not match its format",
        ));
    }
    let from = resolve(Some(from), None)?;
    let names = checked_names(&from, names)?;

    let task = Task::Pack {
        name: target.name,
        options,
    };

    Ok(Request::task(from, names, target.folder, task))
}

/// The archive format a pack format writes.
fn archive_format(format: PackFormat) -> ArchiveFormat {
    match format {
        PackFormat::Zip => ArchiveFormat::Zip,
        PackFormat::TarZst => ArchiveFormat::TarZst,
        PackFormat::SevenZ => ArchiveFormat::SevenZ,
    }
}

#[cfg(test)]
mod tests;

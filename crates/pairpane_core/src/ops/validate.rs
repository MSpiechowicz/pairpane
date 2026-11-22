//! What `submit` can tell about a request without the disk: the number of names, their spelling and their size.

use std::collections::HashSet;

use crate::archive;
use crate::listing::platform;
use crate::model::{ArchiveFormat, AttributeChange, ModeChange, OpKind, PackFormat};

use super::{MAX_NAME_UNITS, MAX_NAMES, MAX_NAMES_BYTES, MIN_PART_SIZE, PackJob, Refused, Request, Task};

/// The Unix permission bits a mask of a mode change may use (with setuid, setgid and sticky).
const MODE_MASK: u32 = 0o7777;

/// How much of the limit [`MAX_NAME_UNITS`] one char of a file name takes: its bytes, and on Windows its UTF-16
/// units (which is what its filesystems count).
pub(super) fn char_units(char: char) -> usize {
    if cfg!(windows) {
        char.len_utf16()
    } else {
        char.len_utf8()
    }
}

/// How many units `name` takes against [`MAX_NAME_UNITS`].
pub(super) fn name_units(name: &str) -> usize {
    name.chars().map(char_units).sum()
}

/// Whether `name` is short enough for a file name here: at most [`MAX_NAME_UNITS`] units (see [`char_units`]).
pub fn name_fits(name: &str) -> bool {
    name_units(name) <= MAX_NAME_UNITS
}

/// Refuses what `submit` can tell without the disk: no names, too many, a name twice, one that is not a plain file
/// name, one that is too long, names that together take more than [`MAX_NAMES_BYTES`], or a task that does not
/// fit the request (see [`validate_task`]).
pub(super) fn validate(request: &Request) -> Result<(), Refused> {
    if request.names.is_empty() {
        return Err(Refused::InvalidNames);
    }

    check_names(&request.names)?;

    validate_task(request)
}

/// Refuses more than [`MAX_NAMES`] names, a name twice, one that is not a plain file name, one that is too long, and
/// names that together take more than [`MAX_NAMES_BYTES`].
fn check_names(names: &[String]) -> Result<(), Refused> {
    if names.len() > MAX_NAMES {
        return Err(Refused::InvalidNames);
    }

    let mut seen = HashSet::with_capacity(names.len());
    let mut bytes = 0_usize;

    for name in names {
        if !platform::is_plain_name(name) || !seen.insert(name.as_str()) {
            return Err(Refused::InvalidNames);
        }

        bytes = bytes.saturating_add(name.len());
        if !name_fits(name) || bytes > MAX_NAMES_BYTES {
            return Err(Refused::NamesTooLarge);
        }
    }

    Ok(())
}

/// Refuses an extraction whose archive folder is not a normalized archive path (see [`archive::normalize`]: no `..`,
/// no drive, no backslash, no control character) or whose entries are not names of the kind [`check_names`] accepts.
fn check_extraction(inner: &str, entries: &[String]) -> Result<(), Refused> {
    match archive::normalize(inner) {
        Ok(Some(path)) if path == inner => {}
        Ok(None) if inner.is_empty() => {}
        _ => return Err(Refused::InvalidTask),
    }

    check_names(entries)
}

/// Whether `name` is a plain file name that fits, and an archive of the `format` by its extension.
fn is_archive_name(name: &str, format: Option<ArchiveFormat>) -> bool {
    platform::is_plain_name(name) && name_fits(name) && archive::by_extension(name) == format
}

/// The format of an archive a pack job writes into.
fn archive_format(format: PackFormat) -> ArchiveFormat {
    match format {
        PackFormat::Zip => ArchiveFormat::Zip,
        PackFormat::TarZst => ArchiveFormat::TarZst,
        PackFormat::SevenZ => ArchiveFormat::SevenZ,
    }
}

/// Whether a pack job of `options` can be written: a tar.zst has no encryption, and names are encrypted only with a
/// password in a 7z.
fn pack_acceptable(options: &PackJob) -> bool {
    let encrypted_names =
        !options.encrypt_names || (options.format == PackFormat::SevenZ && options.password.is_some());

    (options.password.is_none() || options.format != PackFormat::TarZst) && encrypted_names
}

/// Whether the archive of an add job is one that is written: not a rar, nor a single compressed file.
fn is_writable(format: Option<ArchiveFormat>) -> bool {
    !matches!(format, None | Some(ArchiveFormat::Rar | ArchiveFormat::Gz))
}

/// Refuses a kind that is not the task's, a verify, split, combine or extraction that has other than one name, an output
/// that is not a plain name, a pack whose archive name is not a plain name with the extension of its format (or that
/// has a password for a tar.zst, or encrypted names without one in a 7z), an add whose archive is not a plain name of a
/// format that is written or whose folder is not a normalized path, a part shorter than [`MIN_PART_SIZE`], and an attribute change that changes nothing, uses
/// bits beyond `0o7777`, sets permissions on Windows or the Windows attributes elsewhere.
fn validate_task(request: &Request) -> Result<(), Refused> {
    let fits = match request.task.kind() {
        None => matches!(request.kind, OpKind::Copy | OpKind::Move),
        Some(kind) => kind == request.kind,
    };
    if !fits {
        return Err(Refused::InvalidTask);
    }

    let accepted = match &request.task {
        Task::Transfer => true,
        Task::Checksum { output, .. } => platform::is_plain_name(output) && name_fits(output),
        Task::VerifyChecksum | Task::Combine => request.names.len() == 1,
        Task::Extract { inner, entries, .. } => {
            check_extraction(inner, entries)?;

            request.names.len() == 1
        }
        Task::Pack { name, options } => {
            is_archive_name(name, Some(archive_format(options.format))) && pack_acceptable(options)
        }
        Task::AddToArchive { name, inner, .. } => {
            check_extraction(inner, &[])?;

            platform::is_plain_name(name) && name_fits(name) && is_writable(archive::by_extension(name))
        }
        Task::Split { part_size } => request.names.len() == 1 && *part_size >= MIN_PART_SIZE,
        Task::Attributes(change) => attributes_acceptable(change),
    };

    if accepted { Ok(()) } else { Err(Refused::InvalidTask) }
}

fn attributes_acceptable(change: &AttributeChange) -> bool {
    let in_range = match change.mode {
        None => true,
        Some(ModeChange::Set { mode }) => mode <= MODE_MASK,
        Some(ModeChange::Adjust { add, remove }) => add <= MODE_MASK && remove <= MODE_MASK,
    };

    // A system has the one kind of permissions or the other.
    let here = if cfg!(windows) {
        change.mode.is_none()
    } else {
        !change.sets_windows_attributes()
    };

    !change.changes_nothing() && in_range && here
}

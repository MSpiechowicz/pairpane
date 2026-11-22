//! What `submit` can tell about a request without the disk: the number of names, their spelling and their size.

use std::collections::HashSet;

use crate::archive;
use crate::listing::platform;
use crate::model::{ArchiveFormat, AttributeChange, ModeChange, OpKind, PackFormat};

use super::transfer::MAX_DEPTH;
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

/// Whether `name` is an item of a pack job: a plain file name, or a path below the folder of the items made of plain
/// names (`a/b.txt`: [`archive::normalize`] leaves it as it is, and it is not nested deeper than the walk goes). A name
/// of one part is judged as every other job judges its names, whatever it holds.
pub fn is_item_path(name: &str) -> bool {
    if !name.contains('/') {
        return platform::is_plain_name(name);
    }

    let normal = matches!(archive::normalize(name), Ok(Some(ref plain)) if plain == name);

    normal && name.split('/').count() <= MAX_DEPTH
}

/// Whether one of `names` is a folder that another is in (`a` and `a/b`): packing both would put the same file in the
/// archive twice. Where the file system tells `A` from `a` no more than a person does (Windows, macOS) the names are
/// compared that way too: `a` and `A/x` are the same folder.
#[must_use]
pub fn has_nested_names(names: &[String]) -> bool {
    let folded: Vec<String> = names.iter().map(|name| fold_case(name)).collect();
    let all: HashSet<&str> = folded.iter().map(String::as_str).collect();

    folded
        .iter()
        .any(|name| name.match_indices('/').any(|(at, _)| all.contains(&name[..at])))
}

/// `name` as the file system compares it.
pub(crate) fn fold_case(name: &str) -> String {
    if platform::CASE_INSENSITIVE {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

/// Refuses what `submit` can tell without the disk: no names, too many, a name twice, one that is not a plain file
/// name (for a pack: a plain name or a relative path, none inside another), one that is too long, names that together
/// take more than [`MAX_NAMES_BYTES`], or a task that does not fit the request (see [`validate_task`]).
pub(super) fn validate(request: &Request) -> Result<(), Refused> {
    if request.names.is_empty() {
        return Err(Refused::InvalidNames);
    }

    if matches!(request.task, Task::Pack { .. }) {
        check_names_with(&request.names, is_item_path)?;

        if has_nested_names(&request.names) {
            return Err(Refused::InvalidNames);
        }
    } else {
        check_names(&request.names)?;
    }

    validate_task(request)
}

/// Refuses more than [`MAX_NAMES`] names, a name twice, one that is not a plain file name, one that is too long, and
/// names that together take more than [`MAX_NAMES_BYTES`].
fn check_names(names: &[String]) -> Result<(), Refused> {
    check_names_with(names, platform::is_plain_name)
}

/// [`check_names`] for names that are spelled right when `spelled` says so; each part of a path has to fit.
fn check_names_with(names: &[String], spelled: impl Fn(&str) -> bool) -> Result<(), Refused> {
    if names.len() > MAX_NAMES {
        return Err(Refused::InvalidNames);
    }

    let mut seen = HashSet::with_capacity(names.len());
    let mut bytes = 0_usize;

    for name in names {
        if !spelled(name) || !seen.insert(name.as_str()) {
            return Err(Refused::InvalidNames);
        }

        bytes = bytes.saturating_add(name.len());
        if !name.split('/').all(name_fits) || bytes > MAX_NAMES_BYTES {
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
/// format that is written or whose folder is not a normalized path, a part shorter than [`MIN_PART_SIZE`], an attribute change that changes nothing, uses
/// bits beyond `0o7777`, sets permissions on Windows or the Windows attributes elsewhere, and new names that are not
/// right (see [`check_targets`]).
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
        Task::Rename { targets, .. } => {
            check_targets(&request.names, targets)?;

            true
        }
    };

    if accepted { Ok(()) } else { Err(Refused::InvalidTask) }
}

/// Refuses new names that are not one for each item, that are not plain names that fit (together within
/// [`MAX_NAMES_BYTES`]), that are one that the same item has, or that are the same name twice (as the file system
/// compares them).
fn check_targets(names: &[String], targets: &[String]) -> Result<(), Refused> {
    if targets.len() != names.len() {
        return Err(Refused::InvalidTask);
    }

    let mut seen = HashSet::with_capacity(targets.len());
    let mut bytes = 0_usize;

    for (name, target) in names.iter().zip(targets) {
        if !platform::is_plain_name(target) || target == name {
            return Err(Refused::InvalidTask);
        }

        bytes = bytes.saturating_add(target.len());
        if !name_fits(target) || bytes > MAX_NAMES_BYTES {
            return Err(Refused::NamesTooLarge);
        }

        if !seen.insert(fold_case(target)) {
            return Err(Refused::InvalidTask);
        }
    }

    Ok(())
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

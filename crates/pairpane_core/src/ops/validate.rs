//! What `submit` can tell about a request without the disk: the number of names, their spelling and their size.

use std::collections::HashSet;

use crate::listing::platform;
use crate::model::{AttributeChange, ModeChange, OpKind};

use super::{MAX_NAME_UNITS, MAX_NAMES, MAX_NAMES_BYTES, MIN_PART_SIZE, Refused, Request, Task};

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
    if request.names.is_empty() || request.names.len() > MAX_NAMES {
        return Err(Refused::InvalidNames);
    }

    let mut seen = HashSet::with_capacity(request.names.len());
    let mut bytes = 0_usize;

    for name in &request.names {
        if !platform::is_plain_name(name) || !seen.insert(name.as_str()) {
            return Err(Refused::InvalidNames);
        }

        bytes = bytes.saturating_add(name.len());
        if !name_fits(name) || bytes > MAX_NAMES_BYTES {
            return Err(Refused::NamesTooLarge);
        }
    }

    validate_task(request)
}

/// Refuses a kind that is not the task's, a verify, split or combine that has other than one name, an output that
/// is not a plain name, a part shorter than [`MIN_PART_SIZE`], and an attribute change that changes nothing, uses
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

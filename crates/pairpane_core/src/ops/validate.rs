//! What `submit` can tell about a request without the disk: the number of names, their spelling and their size.

use std::collections::HashSet;

use crate::listing::platform;

use super::{MAX_NAME_UNITS, MAX_NAMES, MAX_NAMES_BYTES, Refused, Request};

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
/// name, one that is too long, or names that together take more than [`MAX_NAMES_BYTES`].
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

    Ok(())
}

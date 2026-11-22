//! The names of entries, checked before anything is joined to a path. A name that could reach outside the folder it is
//! extracted into, or that the system would read as another file, is refused for that entry; the rest of the archive is
//! not touched by it.

use crate::listing::platform;
use crate::ops::{MAX_FOLDER_DEPTH, name_fits};

use super::MAX_NAME_BYTES;

/// Why a name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsafe {
    /// A path from the root (`/etc/x`).
    Absolute,
    /// A `..` part, anywhere (`../x`, `a/../../x`, `a/../b`).
    Parent,
    /// A drive (`C:x`, `C:\x`), whatever the system.
    Drive,
    /// A backslash, whatever the system: Windows reads it as a separator, so `a\..\..\x` would climb, and `\\host\x`
    /// is a network path.
    Backslash,
    /// A control character (NUL among them) or a bidirectional control, which could make the name read as another.
    Control,
    /// A part that this system reads as something other than a file: on Windows a device (`NUL`, `COM1`), a stream, a
    /// name that ends in a dot or a space (see [`platform::is_special_windows_name`]).
    Reserved,
    /// A part longer than [`MAX_NAME_UNITS`](crate::ops::MAX_NAME_UNITS), or a whole name longer than
    /// [`MAX_NAME_BYTES`].
    TooLong,
    /// More parts than [`MAX_FOLDER_DEPTH`].
    TooDeep,
}

/// Whether `character` must never be in a name that is shown or written: a C0 or C1 control (NUL and DEL among them), or
/// a bidirectional control (it could reorder the name on the screen).
pub fn is_hostile(character: char) -> bool {
    matches!(
        character,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
    ) || character.is_control()
}

/// The path of an entry as plain names joined with `/`, without a leading or trailing separator: `a/b/c`. `.` parts
/// and empty parts (`a//b`, `./a`) are dropped. `Ok(None)` is a name with no part left (`.`, `./`, the empty name): the
/// archive's own root, with nothing to extract. Every name that could reach outside is `Err`: see [`Unsafe`].
///
/// The check is the same on every system apart from the names Windows alone reads as devices or streams, so an archive
/// is refused or accepted alike wherever it is opened.
pub fn normalize(raw: &str) -> Result<Option<String>, Unsafe> {
    if raw.len() > MAX_NAME_BYTES {
        return Err(Unsafe::TooLong);
    }

    if raw.chars().any(is_hostile) {
        return Err(Unsafe::Control);
    }

    if raw.starts_with('/') {
        return Err(Unsafe::Absolute);
    }

    if raw.contains('\\') {
        return Err(Unsafe::Backslash);
    }

    let mut parts: Vec<&str> = Vec::new();

    for part in raw.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(Unsafe::Parent),
            _ => {}
        }

        if parts.is_empty() && is_drive(part) {
            return Err(Unsafe::Drive);
        }

        if !name_fits(part) {
            return Err(Unsafe::TooLong);
        }

        if !platform::is_plain_name(part) || platform::is_unsafe_name(part) {
            return Err(Unsafe::Reserved);
        }

        parts.push(part);
    }

    if parts.len() > MAX_FOLDER_DEPTH {
        return Err(Unsafe::TooDeep);
    }

    if parts.is_empty() {
        return Ok(None);
    }

    Ok(Some(parts.join("/")))
}

/// A first part that starts like a drive: a letter and a colon.
fn is_drive(part: &str) -> bool {
    matches!(part.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic())
}

/// The last part of a normalized path.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

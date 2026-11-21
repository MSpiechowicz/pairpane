//! The `.trashinfo` record of an item in the Trash: what is written when an item is moved there, and how a record is
//! read when it is put back.

use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jiff::ZonedDateTime;

use crate::ops::report::refusal;

/// The first line of every record.
const HEADER: &str = "[Trash Info]";

/// The text of a record for an item whose original `path` is given as it goes in the record (absolute, or relative to
/// the top folder of the drive), deleted at `date`.
pub(super) fn render(path: &Path, date: &str) -> String {
    format!("{HEADER}\nPath={}\nDeletionDate={date}\n", encode(path))
}

/// The local time now as a record writes it (`2026-04-01T12:30:00`).
///
/// # Errors
///
/// A refusal when the clock is set outside the years a date can hold. No made-up date is written instead: a date of
/// 1970 would make a tool that empties the Trash by age take the item for very old.
pub(super) fn deletion_date() -> io::Result<String> {
    date_of(SystemTime::now())
}

/// `time` as a record writes it, local; the refusal of [`deletion_date`] when it is out of range.
pub(super) fn date_of(time: SystemTime) -> io::Result<String> {
    ZonedDateTime::try_from(time)
        .map(|zoned| format_date(&zoned))
        .map_err(|_| refusal(io::ErrorKind::Other, "the system clock is out of range"))
}

/// `time` as a record writes it: local time with no zone, to the second.
pub(super) fn format_date(time: &ZonedDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

/// The path with `%XX` written for every byte that is not plain (a letter, a digit, or one of `-_.~/`), as the
/// specification asks of a path in a record. The bytes are encoded, not text, so a name that is not text survives.
fn encode(path: &Path) -> String {
    let mut text = String::with_capacity(path.as_os_str().len());

    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => text.push(char::from(*byte)),
            other => text.push_str(&format!("%{other:02X}")),
        }
    }

    text
}

/// The path of a record: `None` when the text is not a record or holds no path. As elsewhere, the first `Path` counts
/// and a line is split at the first `=`.
pub(super) fn parse_path(text: &str) -> Option<PathBuf> {
    let mut lines = text.lines();

    if lines.next()?.trim() != HEADER {
        return None;
    }

    lines.find_map(|line| {
        let (key, value) = line.split_once('=')?;

        (key.trim() == "Path").then(|| decode(value.trim()))
    })
}

/// The path of a record, with `%XX` read as bytes. A `%` that is not followed by two hex digits is kept.
fn decode(text: &str) -> PathBuf {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| char::from(*byte).to_digit(16));

        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                decoded.push(u8::try_from(high * 16 + low).unwrap_or(b'%'));
                index += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    PathBuf::from(OsString::from_vec(decoded))
}

#[cfg(test)]
mod tests;

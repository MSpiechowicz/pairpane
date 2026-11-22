//! The origin marker of an archive that came from elsewhere, and its copy onto what is extracted from it, so a file taken
//! out of a downloaded archive is still known as downloaded (and the system warns before it is run).
//!
//! - Windows: the alternate data stream `Zone.Identifier` of the archive (the "mark of the web"), copied byte for byte to
//!   a stream of the same name on each file.
//! - macOS: the extended attribute `com.apple.quarantine`, copied to each file.
//! - Elsewhere there is no such marker and nothing is done.
//!
//! The marker is read once per job ([`Origin::of`]) and put on each file while it is still the temporary file, before its
//! mode and time are set and before it is renamed into place ([`Origin::apply`]): there is no window in which the file is
//! at its final name without the marker, and the write never goes through a link swapped in at the final name. Putting
//! it on is best effort: a file that does not take one (a volume that has no streams or attributes) leaves the job as it
//! is, and nothing is reported.
//!
//! A marker that is there is never dropped. One that is too long, malformed or cannot be read is replaced by a minimal
//! one of the same kind (see `windows_marker` and `quarantine_marker`); only an archive that definitely has none, or a
//! volume that cannot hold one, gives files without a marker.
//!
//! On macOS the marker is read from the open archive, which was opened without following a link: the quarantine of a
//! symlinked archive is not followed, by design. On Windows a stream cannot be read through the handle without the native
//! API, so it is read by the archive's path a moment after the archive was opened; a file swapped in at that path in
//! between would lend its marker (or none), which is the residual of that platform.

use std::fs::File;
use std::path::Path;

/// The most bytes of a marker that are copied; a real one is a few hundred.
#[cfg(any(windows, target_os = "macos"))]
const MAX_MARKER: usize = 4096;

/// What reading the marker of an archive found.
#[cfg(any(windows, target_os = "macos", test))]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Found {
    /// There is definitely no marker (not there, or the volume cannot hold one).
    Absent,
    /// The whole marker.
    Marker(Vec<u8>),
    /// A marker longer than [`MAX_MARKER`]: its first bytes.
    TooLong(Vec<u8>),
    /// There is one, or may be, but it could not be read.
    Unreadable,
}

/// The marker of an archive: what is put on each file extracted from it, or nothing.
#[derive(Debug, Clone, Default)]
pub(super) struct Origin(Option<Vec<u8>>);

impl Origin {
    /// The marker of the archive `file`, which is at `path` and was opened without following a link; nothing when the
    /// archive has none or the platform has none.
    pub(super) fn of(file: &File, path: &Path) -> Self {
        Self(platform::marker(file, path))
    }

    /// Puts the marker on the file `file`, at `path`, which was just written and is not yet in place. Never fails.
    pub(super) fn apply(&self, file: &File, path: &Path) {
        if let Some(marker) = &self.0 {
            platform::write(file, path, marker);
        }
    }

    /// The marker, when there is one.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}

/// The marker a Windows archive gives its files: what was read when it is a `[ZoneTransfer]` section with a zone in it,
/// else a minimal one. A minimal marker is never below the internet zone, so a malformed or cut marker can only keep the
/// restricted zone (4), never lower the warning. `None` only when there is no marker.
#[cfg(any(windows, test))]
pub(super) fn windows_marker(found: Found) -> Option<Vec<u8>> {
    match found {
        Found::Absent => None,
        Found::Marker(bytes) if is_zone_transfer(&bytes) && zone_id(&bytes).is_some() => Some(bytes),
        Found::Marker(bytes) | Found::TooLong(bytes) => Some(zone_transfer(zone_id(&bytes))),
        Found::Unreadable => Some(zone_transfer(None)),
    }
}

/// The zone of an unknown source: the internet.
#[cfg(any(windows, test))]
const INTERNET_ZONE: u8 = 3;

#[cfg(any(windows, test))]
fn zone_transfer(zone: Option<u8>) -> Vec<u8> {
    let zone = zone.map_or(INTERNET_ZONE, |zone| zone.max(INTERNET_ZONE));

    format!("[ZoneTransfer]\r\nZoneId={zone}\r\n").into_bytes()
}

/// Whether `bytes` start (after a byte order mark) with the `[ZoneTransfer]` section header.
#[cfg(any(windows, test))]
fn is_zone_transfer(bytes: &[u8]) -> bool {
    const HEADER: &[u8] = b"[ZoneTransfer]";

    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);

    bytes.len() >= HEADER.len() && bytes[..HEADER.len()].eq_ignore_ascii_case(HEADER)
}

/// The number of the first `ZoneId=N` line of `bytes` that is a zone (0 to 4).
#[cfg(any(windows, test))]
fn zone_id(bytes: &[u8]) -> Option<u8> {
    bytes.split(|&byte| byte == b'\n').find_map(|line| {
        let line = std::str::from_utf8(line).ok()?.trim();
        let (name, value) = line.split_once('=')?;

        if !name.trim().eq_ignore_ascii_case("ZoneId") {
            return None;
        }

        value.trim().parse::<u8>().ok().filter(|zone| *zone <= 4)
    })
}

/// The marker a macOS archive gives its files: what was read when it has the form of a quarantine value
/// (`flags;seconds;agent;...`), else a minimal one made at `seconds` since the Unix epoch. `None` only when there is no
/// marker.
#[cfg(any(target_os = "macos", test))]
pub(super) fn quarantine_marker(found: Found, seconds: u64) -> Option<Vec<u8>> {
    match found {
        Found::Absent => None,
        Found::Marker(bytes) if is_quarantine(&bytes) => Some(bytes),
        Found::Marker(_) | Found::TooLong(_) | Found::Unreadable => {
            Some(format!("0081;{seconds:x};Pairpane;").into_bytes())
        }
    }
}

/// Whether `bytes` read as `flags;seconds;agent...`: the first two fields hexadecimal numbers.
#[cfg(any(target_os = "macos", test))]
fn is_quarantine(bytes: &[u8]) -> bool {
    let mut fields = bytes.split(|&byte| byte == b';');
    let hex =
        |field: Option<&[u8]>| field.is_some_and(|field| !field.is_empty() && field.iter().all(u8::is_ascii_hexdigit));

    let flags = hex(fields.next());
    let time = hex(fields.next());

    flags && time && fields.next().is_some()
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::{self, Read};
    use std::path::Path;

    use super::{Found, MAX_MARKER, windows_marker};

    /// The name of the stream of the file at `path`.
    fn stream(path: &Path) -> OsString {
        let mut name = path.as_os_str().to_owned();
        name.push(":Zone.Identifier");

        name
    }

    /// Reads the stream of the archive by its path (see the module documentation for the residual of that).
    fn read(path: &Path) -> Found {
        let opened = match File::open(stream(path)) {
            Ok(opened) => opened,
            // No such stream, or a volume where a name with a colon is none: there is no marker.
            Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::InvalidFilename) => {
                return Found::Absent;
            }
            Err(_) => return Found::Unreadable,
        };

        let mut bytes = Vec::new();
        if opened.take(MAX_MARKER as u64 + 1).read_to_end(&mut bytes).is_err() {
            return Found::Unreadable;
        }

        if bytes.len() > MAX_MARKER {
            bytes.truncate(MAX_MARKER);

            return Found::TooLong(bytes);
        }

        Found::Marker(bytes)
    }

    pub(super) fn marker(_archive: &File, path: &Path) -> Option<Vec<u8>> {
        windows_marker(read(path))
    }

    /// Writes the stream of the temporary file `path`, which this job made and which is renamed with its stream.
    pub(super) fn write(_file: &File, path: &Path, marker: &[u8]) {
        let _ = fs::write(stream(path), marker);
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::fs::File;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::attribute::{read_from, write_to};
    use super::{MAX_MARKER, quarantine_marker};

    const QUARANTINE: &str = "com.apple.quarantine";

    pub(super) fn marker(archive: &File, _path: &Path) -> Option<Vec<u8>> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());

        quarantine_marker(read_from(archive, QUARANTINE, MAX_MARKER), seconds)
    }

    pub(super) fn write(file: &File, _path: &Path, marker: &[u8]) {
        write_to(file, QUARANTINE, marker);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use std::fs::File;
    use std::path::Path;

    pub(super) fn marker(_archive: &File, _path: &Path) -> Option<Vec<u8>> {
        None
    }

    pub(super) fn write(_file: &File, _path: &Path, _marker: &[u8]) {}
}

/// Extended attributes of an open file (macOS; Linux in the tests, where the same calls can be tried on a `user.`
/// attribute).
#[cfg(any(target_os = "macos", all(target_os = "linux", test)))]
pub(super) mod attribute {
    use std::fs::File;

    use rustix::fs::{XattrFlags, fgetxattr, fsetxattr};
    use rustix::io::Errno;

    use super::Found;

    /// The attribute `name` of `file`, at most `limit` bytes of it.
    pub(in crate::ops) fn read_from(file: &File, name: &str, limit: usize) -> Found {
        let mut buffer = vec![0_u8; limit + 1];

        match fgetxattr(file, name, &mut buffer[..]) {
            Ok(length) if length > limit => {
                buffer.truncate(limit);

                Found::TooLong(buffer)
            }
            Ok(length) => {
                buffer.truncate(length);

                Found::Marker(buffer)
            }
            Err(Errno::RANGE) => Found::TooLong(Vec::new()),
            Err(error) if is_missing(error) => Found::Absent,
            Err(_) => Found::Unreadable,
        }
    }

    /// Whether `error` says the attribute is not there, or that the volume keeps none.
    fn is_missing(error: Errno) -> bool {
        #[cfg(target_os = "macos")]
        let absent = Errno::NOATTR;
        #[cfg(not(target_os = "macos"))]
        let absent = Errno::NODATA;

        error == absent || error == Errno::NOTSUP
    }

    /// Sets the attribute `name` of `file` to `value`; a failure is ignored.
    pub(in crate::ops) fn write_to(file: &File, name: &str, value: &[u8]) {
        let _ = fsetxattr(file, name, value, XattrFlags::empty());
    }

    /// Sets the attribute `name` of the file at `path` (not following a link) to `value`, to mark a file in the tests.
    #[cfg(all(test, target_os = "macos"))]
    pub(in crate::ops) fn write_attribute(path: &std::path::Path, name: &str, value: &[u8]) {
        let _ = rustix::fs::lsetxattr(path, name, value, XattrFlags::empty());
    }

    /// The attribute `name` of the file at `path`, to look at a file in the tests.
    #[cfg(all(test, target_os = "macos"))]
    pub(in crate::ops) fn read_attribute(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
        let mut buffer = vec![0_u8; super::MAX_MARKER];
        let length = rustix::fs::lgetxattr(path, name, &mut buffer[..]).ok()?;
        buffer.truncate(length);

        Some(buffer)
    }
}

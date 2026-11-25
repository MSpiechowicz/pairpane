//! The alternate data streams of a Windows file, carried to its copy (see `extras`).
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library cannot list a file's streams, which `FindFirstStreamW` and `FindNextStreamW` do. Each stream is then
//! read and written with the standard library, as `file:name:$DATA`. What is done with a listed name is decided by
//! [`stream_name`] and [`may_drop`], which are safe and tested on every system.
//!
//! A listed name comes from the source's file system, which may not be a trusted one (a network share, a disk image
//! someone made): it is used only when it is a plain stream name, so it cannot name another file. The streams are found
//! and opened by path, which is checked to lead to the file the copy was made from, and each stream opened is checked to
//! belong to that file (and to the copy): a folder on the way swapped for a link while this runs can make the listing
//! look elsewhere, but no data from another file reaches the copy. Listing by handle would close that window too.
#![allow(
    unsafe_code,
    reason = "the calls of FindFirstStreamW, FindNextStreamW, FindClose and GetLastError; see `names`"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

/// The name inside a listed stream `:name:$DATA`, when it is one that can be opened as itself: not empty, without a
/// separator, a colon, a control character or a character that could not be read back (a lone surrogate, shown as
/// U+FFFD), and not one Windows rewrites (`.`, `..`, a trailing dot or space). `None` for the file's main stream
/// (`::$DATA`) and for anything else.
#[cfg_attr(not(windows), allow(dead_code, reason = "used on Windows; tested everywhere"))]
pub(super) fn stream_name(listed: &str) -> Option<&str> {
    let name = listed.strip_prefix(':')?.strip_suffix(":$DATA")?;
    let plain = !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| matches!(c, '\\' | '/' | ':' | '\u{FFFD}') || c.is_control());

    plain.then_some(name)
}

/// Whether the stream `name` (see [`stream_name`]) may be left out when it cannot be copied: the zone a download came
/// from, which Explorer drops too where the target holds no streams.
#[cfg_attr(not(windows), allow(dead_code, reason = "used on Windows; tested everywhere"))]
pub(super) fn may_drop(name: &str) -> bool {
    name.eq_ignore_ascii_case("Zone.Identifier")
}

#[cfg(windows)]
pub(super) use imp::carry;

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        ERROR_HANDLE_EOF, ERROR_INVALID_PARAMETER, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
        WIN32_FIND_STREAM_DATA,
    };

    use super::super::extras::NotKept;
    use super::super::identity::FileId;
    use super::{may_drop, stream_name};

    /// The main stream of every file, which is its content.
    const MAIN: &str = "::$DATA";

    /// The most streams a file is read for; a file with more fails to move whole.
    const MOST: usize = 4096;

    /// The most bytes of all the streams of one file that are copied; past it the rest are not, so its file's move keeps
    /// the source. Streams hold small things (where a file came from, a thumbnail, a sync tool's state); this bounds a
    /// hostile file whose sparse streams would be written out in full.
    const MOST_BYTES: u64 = 1024 * 1024 * 1024;

    /// Bytes copied between two looks at whether the job was cancelled.
    const CHUNK: usize = 1024 * 1024;

    pub(in super::super) fn carry(
        (source, source_path): (&File, &Path),
        (copy, copy_path): (&File, &Path),
        (refuse, stopped): (bool, &dyn Fn() -> bool),
    ) -> Result<(), NotKept> {
        // The streams are found and opened by path: the path must still lead to the file the copy was made from, and
        // not to a link put there since (which could lead the listing to another file, or to a network host).
        if !same_file(source, source_path) {
            return Err(NotKept(
                "alternate data streams (it changed while it was copied)".to_owned(),
            ));
        }

        let listed = match names(source_path) {
            Ok(listed) => listed,
            // A file system without streams has none to carry.
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(()),
            Err(error) => return Err(NotKept(format!("alternate data streams ({error})"))),
        };

        // A copy takes every stream it can; the first that should have gone and did not is what a move reports.
        let mut first_missing = None;
        let mut budget = MOST_BYTES;
        for listed in listed.iter().filter(|name| name.as_str() != MAIN) {
            if stopped() {
                return Err(NotKept("alternate data streams (cancelled)".to_owned()));
            }
            let Some(name) = stream_name(listed) else {
                first_missing.get_or_insert(NotKept(format!("alternate data stream {listed:?}")));
                continue;
            };
            let written = if refuse {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            } else {
                copy_stream((source, source_path), (copy, copy_path), name, (&mut budget, stopped))
            };
            if written.is_err() && !may_drop(name) {
                first_missing.get_or_insert(NotKept(format!("alternate data stream {name:?}")));
            }
        }

        first_missing.map_or(Ok(()), Err)
    }

    /// Whether `path` leads to the file open as `file`.
    fn same_file(file: &File, path: &Path) -> bool {
        matches!((FileId::of_file(file), FileId::of_path(path)), (Some(open), Some(named)) if open == named)
    }

    /// Whether the stream open as `stream` belongs to the file open as `file` (a stream's handle tells its file).
    fn stream_of(stream: &File, file: &File) -> bool {
        matches!((FileId::of_file(stream), FileId::of_file(file)), (Some(stream), Some(file)) if stream == file)
    }

    /// Copies the stream `name` of `source` to the same stream of `copy`, which must not have it yet, taking its bytes
    /// from `budget` and stopping once the job is cancelled. Neither open follows a link at the file, and each stream
    /// must belong to its file.
    fn copy_stream(
        (source, source_path): (&File, &Path),
        (copy, copy_path): (&File, &Path),
        name: &str,
        (budget, stopped): (&mut u64, &dyn Fn() -> bool),
    ) -> io::Result<()> {
        let mut from = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(with_stream(source_path, name))?;
        let mut to = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(with_stream(copy_path, name))?;
        if !stream_of(&from, source) || !stream_of(&to, copy) {
            return Err(io::Error::other("the stream is not of the file being copied"));
        }

        let mut buffer = vec![0; CHUNK];
        loop {
            if stopped() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let read = from.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            let read_bytes = read as u64;
            if read_bytes > *budget {
                return Err(io::Error::other("the streams are too large"));
            }
            *budget -= read_bytes;
            to.write_all(&buffer[..read])?;
        }
    }

    /// `path` with the stream `name` (a plain one, see `stream_name`) named after it: `path:name:$DATA`.
    fn with_stream(path: &Path, name: &str) -> OsString {
        let mut joined = path.as_os_str().to_owned();
        joined.push(":");
        joined.push(name);
        joined.push(":$DATA");
        joined
    }

    /// The names of the streams of the file at `path`, as listed (`:name:$DATA`).
    fn names(path: &Path) -> io::Result<Vec<String>> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut data = WIN32_FIND_STREAM_DATA::default();

        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call, and `data` is a writable
        // `WIN32_FIND_STREAM_DATA`, the structure `FindStreamInfoStandard` fills.
        let find = unsafe { FindFirstStreamW(wide.as_ptr(), FindStreamInfoStandard, (&raw mut data).cast(), 0) };
        if find == INVALID_HANDLE_VALUE {
            // SAFETY: reads the calling thread's last error, set by the call that just failed.
            let code = unsafe { GetLastError() };
            return match code {
                // No stream at all (a folder).
                ERROR_HANDLE_EOF => Ok(Vec::new()),
                // A file system without streams (FAT, exFAT).
                ERROR_INVALID_PARAMETER => Err(io::Error::from(io::ErrorKind::Unsupported)),
                _ => Err(io::Error::from_raw_os_error(code.cast_signed())),
            };
        }

        let mut listed = Vec::new();
        let mut last = ERROR_HANDLE_EOF;
        loop {
            listed.push(name_of(&data));
            if listed.len() >= MOST {
                break;
            }
            // SAFETY: `find` is the open search handle returned above, and `data` is writable as before.
            if unsafe { FindNextStreamW(find, (&raw mut data).cast()) } == 0 {
                // Only "no more streams" ends the list; any other failure would leave streams out unseen.
                // SAFETY: reads the calling thread's last error, set by the FindNextStreamW call that just failed.
                last = unsafe { GetLastError() };
                break;
            }
        }
        // SAFETY: closes the search handle opened above exactly once.
        unsafe { FindClose(find) };

        if listed.len() >= MOST {
            return Err(io::Error::other("too many alternate data streams"));
        }
        if last != ERROR_HANDLE_EOF {
            return Err(io::Error::from_raw_os_error(last.cast_signed()));
        }

        Ok(listed)
    }

    /// The listed name in `data`; a lone surrogate reads as U+FFFD, which `stream_name` refuses.
    fn name_of(data: &WIN32_FIND_STREAM_DATA) -> String {
        let length = data
            .cStreamName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(data.cStreamName.len());

        OsString::from_wide(&data.cStreamName[..length])
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{may_drop, stream_name};

    #[test]
    fn only_a_plain_stream_name_is_used() {
        assert_eq!(stream_name(":Zone.Identifier:$DATA"), Some("Zone.Identifier"));
        assert_eq!(stream_name(":pairpane.test:$DATA"), Some("pairpane.test"));
        for listed in [
            "::$DATA",
            ":a\\..\\b:$DATA",
            ":x\\..\\..\\Users\\v\\Startup\\a.bat:$DATA",
            ":a/b:$DATA",
            ":a:b:$DATA",
            ":x:$INDEX_ALLOCATION",
            ":\u{0}:$DATA",
            ":a\u{1}:$DATA",
            ":\u{FFFD}:$DATA",
            ":.:$DATA",
            ":..:$DATA",
            ":trailing.:$DATA",
            ":trailing :$DATA",
            ":x",
            "x:$DATA",
            "",
        ] {
            assert_eq!(stream_name(listed), None, "{listed:?}");
        }
    }

    #[test]
    fn only_the_zone_of_a_download_may_be_left_out() {
        assert!(may_drop("Zone.Identifier"));
        assert!(may_drop("zone.identifier"));
        assert!(!may_drop("com.dropbox.attributes"));
        assert!(!may_drop("extra"));
    }
}

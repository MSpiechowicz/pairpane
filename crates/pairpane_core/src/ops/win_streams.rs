//! The alternate data streams of a Windows file, carried to its copy (see `extras`).
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library cannot list a file's streams, which `FindFirstStreamW` and `FindNextStreamW` do. Each stream is then
//! read and written with the standard library, as `file:name:$DATA`. Which streams may stay behind is decided by
//! [`may_drop`], which is safe and tested on every system.
#![allow(
    unsafe_code,
    reason = "the calls of FindFirstStreamW, FindNextStreamW and FindClose; see `names`"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

/// Whether the stream `name` (as listed, `:name:$DATA`) may be left out where the target holds no streams: the zone a
/// download came from, which Explorer drops too.
#[cfg_attr(not(windows), allow(dead_code, reason = "used on Windows; tested everywhere"))]
pub(super) fn may_drop(name: &str) -> bool {
    name.eq_ignore_ascii_case(":Zone.Identifier:$DATA")
}

#[cfg(windows)]
pub(super) use imp::carry;

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        ERROR_HANDLE_EOF, ERROR_INVALID_PARAMETER, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard, WIN32_FIND_STREAM_DATA,
    };

    use super::super::extras::NotKept;
    use super::may_drop;

    /// The main stream of every file, which is its content.
    const MAIN: &str = "::$DATA";

    /// The most streams a file is read for; a file with more fails to move whole.
    const MOST: usize = 4096;

    pub(in super::super) fn carry(source: &Path, copy: &Path, refuse: bool) -> Result<(), NotKept> {
        let listed = match names(source) {
            Ok(listed) => listed,
            // A file system without streams has none to carry.
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(()),
            Err(error) => return Err(NotKept(format!("its alternate data streams ({error})"))),
        };

        for name in listed.iter().filter(|name| name.as_str() != MAIN) {
            let written = if refuse {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            } else {
                copy_stream(source, copy, name)
            };
            if written.is_err() && !may_drop(name) {
                return Err(NotKept(
                    name.trim_start_matches(':').trim_end_matches(":$DATA").to_owned(),
                ));
            }
        }

        Ok(())
    }

    /// Copies the stream `name` of the file at `source` to the same stream of the file at `copy`.
    fn copy_stream(source: &Path, copy: &Path, name: &str) -> io::Result<()> {
        let mut from = File::open(with_stream(source, name))?;
        let mut to = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(with_stream(copy, name))?;
        io::copy(&mut from, &mut to)?;

        Ok(())
    }

    fn with_stream(path: &Path, name: &str) -> OsString {
        let mut joined = path.as_os_str().to_owned();
        joined.push(name);
        joined
    }

    /// The names of the streams of the file at `path`, as `:name:$DATA`.
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
        loop {
            listed.push(name_of(&data));
            if listed.len() >= MOST {
                break;
            }
            // SAFETY: `find` is the open search handle returned above, and `data` is writable as before.
            if unsafe { FindNextStreamW(find, (&raw mut data).cast()) } == 0 {
                break;
            }
        }
        // SAFETY: closes the search handle opened above exactly once.
        unsafe { FindClose(find) };

        if listed.len() >= MOST {
            return Err(io::Error::other("too many alternate data streams"));
        }

        Ok(listed)
    }

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
    use super::may_drop;

    #[test]
    fn only_the_zone_of_a_download_may_be_left_out() {
        assert!(may_drop(":Zone.Identifier:$DATA"));
        assert!(may_drop(":zone.identifier:$DATA"));
        assert!(!may_drop(":com.dropbox.attributes:$DATA"));
        assert!(!may_drop(":extra:$DATA"));
    }
}

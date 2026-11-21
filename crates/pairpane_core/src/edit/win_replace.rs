//! Replacing a file by another on Windows with `ReplaceFileW`, which, unlike a rename, keeps the replaced file's ACLs,
//! alternate data streams and attributes.
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library has no call that replaces a file and keeps what belongs to it. The one call is made on two paths
//! that are turned into NUL-terminated UTF-16 here, and a path that holds a NUL is refused first.
#![allow(unsafe_code, reason = "the one call of ReplaceFileW; see `replace_file`")]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null;

use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

/// `path` as the NUL-terminated UTF-16 the system takes; a path with a NUL in it is refused (it would end early).
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut units: Vec<u16> = OsStr::new(path).encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }

    units.push(0);

    Ok(units)
}

/// Replaces the file `replaced` by the file `replacement`, which is gone afterwards; `replaced` keeps its ACLs, its
/// alternate streams and its attributes. Both must exist. When the system cannot keep them it fails and neither is
/// changed (no flag asks it to go on regardless).
pub(super) fn replace_file(replaced: &Path, replacement: &Path) -> io::Result<()> {
    let replaced = wide(replaced)?;
    let replacement = wide(replacement)?;

    // SAFETY: `replaced` and `replacement` are NUL-terminated UTF-16 buffers with no other NUL, alive and not moved for
    // the whole call, which only reads them. The backup name is a null pointer (no backup is made), the flags are zero,
    // and the two reserved pointers must be null. The call keeps no pointer after it returns; a failure is a zero
    // result and nothing else is touched.
    let done = unsafe { ReplaceFileW(replaced.as_ptr(), replacement.as_ptr(), null(), 0, null(), null()) };

    if done == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

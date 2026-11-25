//! The ACL of a macOS file, carried to its copy (see `extras`, #80). On macOS an ACL is not an extended attribute, so the
//! attributes do not bring it.
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library has no call for it, and `fcopyfile` with `COPYFILE_ACL` copies the ACL alone from one open file to
//! another (nothing else: not the content, the mode or the attributes). A file without an ACL gives the copy none.
#![allow(unsafe_code, reason = "the one call of fcopyfile; see `carry`")]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// Gives the open file `copy` the ACL of the open file `source`. Fails where the copy's disk holds no ACL and the source
/// has one.
pub(super) fn carry(source: &File, copy: &File) -> io::Result<()> {
    // SAFETY: both descriptors are open for the whole call (borrowed from the files), and a null state is what
    // fcopyfile takes when no state is kept between calls.
    let result = unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            copy.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

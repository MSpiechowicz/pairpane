//! The ACL of a macOS file, carried to its copy (#80). On macOS an ACL is not an extended attribute, so the attributes
//! (see `extras`) do not bring it.
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library has no call for it. The ACL is read from the source with `acl_get_fd_np` and set on the copy with
//! `acl_set_fd_np` (the extended ACL, the one Finder and `chmod +a` give), both on open files; a file without one is left
//! alone, so the copy keeps what its new folder gives it. `fcopyfile` is not used: it only logs an ACL it could not set,
//! and a move would then delete a source whose ACL its copy does not have.
#![allow(
    unsafe_code,
    reason = "the calls of acl_get_fd_np, acl_set_fd_np and acl_free; see `carry`"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::{c_int, c_uint, c_void};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// `ACL_TYPE_EXTENDED` of `<sys/acl.h>`.
const ACL_TYPE_EXTENDED: c_uint = 0x0000_0100;

// SAFETY: these are the declarations of `<sys/acl.h>` (libSystem): `acl_t` is an opaque pointer, `acl_type_t` an
// unsigned int.
unsafe extern "C" {
    fn acl_get_fd_np(fd: c_int, kind: c_uint) -> *mut c_void;
    fn acl_set_fd_np(fd: c_int, acl: *mut c_void, kind: c_uint) -> c_int;
    fn acl_free(object: *mut c_void) -> c_int;
}

/// Gives the open file `copy` the ACL of the open file `source`, when it has one. Fails where the copy's disk cannot hold
/// it (FAT, exFAT, some network disks), so a move keeps its source.
pub(super) fn carry(source: &File, copy: &File) -> io::Result<()> {
    // SAFETY: the descriptor is open for the call (borrowed from the file); the result is an ACL this code owns and frees
    // below, or null with errno set.
    let acl = unsafe { acl_get_fd_np(source.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let error = io::Error::last_os_error();
        // No ACL (ENOENT), or a disk that holds none: nothing to carry.
        return match error.raw_os_error() {
            Some(code) if code == libc::ENOENT || code == libc::ENOTSUP || code == libc::EOPNOTSUPP => Ok(()),
            _ => Err(error),
        };
    }

    // SAFETY: `acl` is the valid ACL read above, and the descriptor is open for the call.
    let set = unsafe { acl_set_fd_np(copy.as_raw_fd(), acl, ACL_TYPE_EXTENDED) };
    let error = (set != 0).then(io::Error::last_os_error);
    // SAFETY: frees the ACL read above, once, after its last use.
    unsafe { acl_free(acl) };

    error.map_or(Ok(()), Err)
}

//! The confinement of the thumbnail helper on macOS, done by the helper itself before it reads a byte (the parent cannot
//! do it before the exec). The same three steps as on Linux (`heic/sandbox.rs`), with what the system offers:
//!
//! 1. [`close_inherited_fds`]: every descriptor from 3 up is closed, so nothing the app had open reaches the decoders. The
//!    parent also starts the helper with a cleared environment (`process::command`).
//! 2. [`limit`]: no core dumps, 60 seconds of CPU time and 64 open files, and the limit on the data segment (best effort:
//!    how much of what is mapped it counts depends on the system). The limit on the address space is not set: the
//!    virtual size of a process on macOS holds the system's shared libraries and is far over any limit that means
//!    something. What stops a picture that claims more than it may is the decoder's own limits (`child.rs`).
//! 3. [`deny_all`]: the Seatbelt sandbox with the profile `(version 1) (deny default)`, which is "pure computation": the
//!    process keeps the descriptors it has (its standard input and output) and its memory and may compute, and every
//!    other operation is denied for good: opening, reading, writing, creating or deleting a file, a socket of any kind,
//!    looking up a service, starting a program or a process, signalling another process, reading system information.
//!    The profile is a fixed string, never built from anything the app is given.
//!
//! Not done: the helper does not die with its parent as it does on Linux (`PR_SET_PDEATHSIG`), which macOS has not. A
//! helper whose parent has gone finds its standard input closed, ends when it tries to write its answer (`SIGPIPE`), and
//! at the latest when it has had its 60 seconds of CPU time.
//!
//! `sandbox_init` is deprecated (Apple's own tools and the browsers still use it, and there is no other API for a process to
//! confine itself). When it fails the helper exits without reading a byte, which the parent counts as a picture that
//! cannot be decoded; it never runs unconfined.
#![allow(
    unsafe_code,
    reason = "the sandbox's C interface and `close`; see each `unsafe` block"
)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, c_char, c_int};
use std::io;
use std::ptr;

use rustix::process::{Resource, getrlimit};

use crate::process::lower_limit;

/// The profile of step 3: everything is denied.
const PROFILE: &CStr = c"(version 1)(deny default)";

/// The most CPU time of the helper, as on Linux.
const CPU_SECONDS: u64 = 60;

/// The most open files: the three standard streams and some room, as on Linux.
const MAX_FILES: u64 = 64;

/// The data segment the helper asks the system to allow it, in bytes: what `child.rs` allows a picture (the input, the
/// decoded picture and what the decoder keeps beside it) with room to spare, as on Linux.
const MEMORY: u64 = 1536 * 1024 * 1024;

unsafe extern "C" {
    /// `sandbox_init(3)`: confines the calling process for good with the profile (flags 0: `profile` is the text of one).
    /// Returns 0, or -1 with a message (to be freed by `sandbox_free_error`) in `errorbuf`.
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;

    /// Frees the message `sandbox_init` has set.
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// The three steps (see the module), in order. Fails when the process cannot be limited or confined; the helper then
/// exits and the parent counts it as a picture that cannot be decoded.
pub(super) fn confine() -> io::Result<()> {
    close_inherited_fds();
    limit()?;

    deny_all()
}

/// Step 1 (see the module). Whatever cannot be closed stays open.
fn close_inherited_fds() {
    let limit = getrlimit(Resource::Nofile).current.unwrap_or(256).min(65_536);

    for fd in 3..c_int::try_from(limit).unwrap_or(65_536) {
        // SAFETY: `close` takes the descriptor by value; one that is not open only fails. The helper holds no descriptor
        // it needs but its standard streams at this point (it is the first thing it does).
        unsafe { libc::close(fd) };
    }
}

/// Step 2 (see the module).
fn limit() -> io::Result<()> {
    for (resource, value) in [
        (Resource::Core, 0),
        (Resource::Cpu, CPU_SECONDS),
        (Resource::Nofile, MAX_FILES),
    ] {
        lower_limit(resource, value)?;
    }

    // Not enforced on every system: what is not done is not an error.
    let _ = lower_limit(Resource::Data, MEMORY);

    Ok(())
}

/// Step 3 (see the module).
fn deny_all() -> io::Result<()> {
    let mut error: *mut c_char = ptr::null_mut();

    // SAFETY: `PROFILE` is a NUL-terminated string that lives through the call, the flags are 0 (the string is a profile),
    // and `error` is a place for the pointer to a message, which is set only when the call fails.
    let result = unsafe { sandbox_init(PROFILE.as_ptr(), 0, &raw mut error) };
    if result == 0 {
        return Ok(());
    }

    let message = if error.is_null() {
        "the sandbox cannot be entered".to_owned()
    } else {
        // SAFETY: on failure `error` is a NUL-terminated message that `sandbox_init` allocated and nothing else owns; it
        // is copied and freed once, here, and not used after.
        let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
        // SAFETY: `error` was set by `sandbox_init` and is freed exactly once.
        unsafe { sandbox_free_error(error) };

        text
    };

    Err(io::Error::other(message))
}

//! The errors of the archive code. Each one that holds a text of ours has a fixed lowercase English text without a path
//! (see [`crate::view::refusal`]), so a caller can show it; the marker of a missing or wrong password is its own type.

use std::error::Error;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::view::refusal;

/// The error of an archive that is broken: `InvalidData`.
pub fn unreadable() -> io::Error {
    refusal(io::ErrorKind::InvalidData, "the archive cannot be read")
}

/// The error of a feature or format this build does not handle: `Unsupported`, with `text` as its message.
pub fn unsupported(text: &'static str) -> io::Error {
    refusal(io::ErrorKind::Unsupported, text)
}

/// The error of a tar that has data after its end, which a rewrite would drop: `Unsupported`.
pub fn data_after_end() -> io::Error {
    unsupported("the archive has data after its end, so it can't be changed")
}

/// The error of work that was stopped because its cancel flag was raised: `Interrupted`.
pub fn cancelled() -> io::Error {
    refusal(io::ErrorKind::Interrupted, "the read was cancelled")
}

/// Fails with [`cancelled`] when `cancel` is raised.
pub fn check(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    Ok(())
}

/// Why an archive's data cannot be read yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordFault {
    /// There is no password and one is needed.
    Needed,
    /// The password that was given does not open it.
    Wrong,
}

/// The payload of the error that [`password_error`] makes.
#[derive(Debug)]
struct Marker(PasswordFault, &'static str);

impl fmt::Display for Marker {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.1)
    }
}

/// What a page is told for `fault` (a fixed text without a password or a path; the page tells the two apart by the word
/// "wrong").
pub const fn fault_text(fault: PasswordFault) -> &'static str {
    match fault {
        PasswordFault::Needed => "needs a password",
        PasswordFault::Wrong => "the password is wrong",
    }
}

impl Error for Marker {}

/// The error that says an archive (or an entry of it) needs a password, or that the one given is wrong. It is
/// `PermissionDenied` and its text holds no password; [`password_fault`] tells it from every other error.
pub fn password_error(fault: PasswordFault) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, Marker(fault, fault_text(fault)))
}

/// What a page is told when a password was refused by a check that cannot tell a wrong password from damage (reading a
/// whole entry, see [`super::confirm_password`]). The word "wrong" stays, so the page asks for the password again.
const UNVERIFIED_TEXT: &str = "the password is wrong or the archive is damaged";

/// [`password_error`] of [`PasswordFault::Wrong`] with [`UNVERIFIED_TEXT`], for a check that fails closed: it is a wrong
/// password to [`password_fault`], and its text says it may be damage.
pub fn unverified_password_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        Marker(PasswordFault::Wrong, UNVERIFIED_TEXT),
    )
}

/// The text a page is told for `error` when it was made by [`password_error`] or [`unverified_password_error`].
pub fn password_text(error: &io::Error) -> Option<&'static str> {
    error.get_ref()?.downcast_ref::<Marker>().map(|marker| marker.1)
}

/// What `error` says about a password, when it was made by [`password_error`].
pub fn password_fault(error: &io::Error) -> Option<PasswordFault> {
    error.get_ref()?.downcast_ref::<Marker>().map(|marker| marker.0)
}

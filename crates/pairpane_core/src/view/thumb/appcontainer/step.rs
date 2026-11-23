//! What failed and why, for the log and the tests: every failure of the container's calls keeps the call that failed and
//! the system's own error (its code and its text), because a Windows call that is refused says little by itself. The page
//! is never told any of it (these are errors of the system, which the commands turn into fixed texts).

use std::fmt;
use std::io;

/// A failure of the call `name`, with the error the system gave.
#[derive(Debug)]
pub(super) struct Step {
    name: &'static str,
    source: io::Error,
}

impl fmt::Display for Step {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}: {}", self.name, self.source)
    }
}

impl std::error::Error for Step {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The error of the call `name` that failed with `source`.
pub(super) fn at(name: &'static str, source: io::Error) -> io::Error {
    io::Error::new(source.kind(), Step { name, source })
}

/// The error of the call `name` that failed with the Win32 error `code`.
pub(super) fn at_code(name: &'static str, code: u32) -> io::Error {
    at(name, io::Error::from_raw_os_error(code as i32))
}

/// The error of the call `name` that failed with the `HRESULT` `result`: the Win32 error in it when it is one (the
/// facility of `HRESULT_FROM_WIN32`), the number itself otherwise.
pub(super) fn at_result(name: &'static str, result: i32) -> io::Error {
    let bits = result as u32;

    if bits >> 16 == 0x8007 {
        return at_code(name, bits & 0xFFFF);
    }

    at(name, io::Error::other(format!("HRESULT {bits:#010x}")))
}

/// The Win32 error code of `error`, or of the call it is the failure of.
pub(super) fn code_of(error: &io::Error) -> Option<i32> {
    error.raw_os_error().or_else(|| {
        error
            .get_ref()?
            .downcast_ref::<Step>()
            .and_then(|step| code_of(&step.source))
    })
}

/// `ERROR_ACCESS_DENIED`: what `CreateProcessW` says when the container is refused the file it is to run. (A file or a path
/// that is not found is not this: a change of the access list does not make a file exist.)
pub(super) fn is_denied(error: &io::Error) -> bool {
    code_of(error) == Some(5)
}

/// A failure that is the state of the machine at the moment (no memory, quota or system resources) and not a verdict on
/// the container: the next try may work.
pub(super) fn is_resource(error: &io::Error) -> bool {
    // ERROR_NOT_ENOUGH_MEMORY, ERROR_OUTOFMEMORY, ERROR_NO_SYSTEM_RESOURCES, ERROR_COMMITMENT_LIMIT, ERROR_NOT_ENOUGH_QUOTA,
    // ERROR_NONPAGED_SYSTEM_RESOURCES, ERROR_PAGED_SYSTEM_RESOURCES, ERROR_PAGEFILE_QUOTA, ERROR_WORKING_SET_QUOTA.
    matches!(
        code_of(error),
        Some(8 | 14 | 1450 | 1455 | 1816 | 1451 | 1452 | 1453 | 1454)
    )
}

//! Error texts of a job: what the webview is told when an item or a whole job fails.

use std::io::{self, ErrorKind};

use crate::model::{IpcError, IpcErrorCode};

/// Windows `ERROR_PRIVILEGE_NOT_HELD`: creating a symbolic link without the privilege (or Developer Mode).
const WINDOWS_NO_LINK_PRIVILEGE: i32 = 1314;

/// The error to show for `error`. The text never holds a path, so it is safe to display as it is.
///
/// An error made by this module (it carries its own text) is shown with that text; one from the system is told
/// by its kind, as the system's own wording differs between platforms.
pub(super) fn ipc_error(error: &io::Error) -> IpcError {
    if error.raw_os_error() == Some(WINDOWS_NO_LINK_PRIVILEGE) && cfg!(windows) {
        return IpcError::new(
            IpcErrorCode::Forbidden,
            "this system does not let this user create links (it needs Developer Mode or administrator rights)",
        );
    }

    if error.raw_os_error().is_none()
        && let Some(own) = error.get_ref()
    {
        return IpcError::new(code(error.kind()), own.to_string());
    }

    let message = match error.kind() {
        ErrorKind::NotFound => "it no longer exists",
        ErrorKind::PermissionDenied => "permission denied",
        ErrorKind::AlreadyExists => "something with this name exists already",
        ErrorKind::StorageFull => "the disk is full",
        ErrorKind::QuotaExceeded => "the disk quota is used up",
        ErrorKind::ReadOnlyFilesystem => "the disk is read-only",
        ErrorKind::InvalidFilename => "the system does not accept this name",
        ErrorKind::CrossesDevices => "it is on another disk",
        ErrorKind::ResourceBusy | ErrorKind::ExecutableFileBusy => "it is in use by another program",
        ErrorKind::TimedOut => "the disk did not answer in time",
        _ => "the disk reported an error",
    };

    IpcError::new(code(error.kind()), message)
}

fn code(kind: ErrorKind) -> IpcErrorCode {
    match kind {
        ErrorKind::NotFound => IpcErrorCode::NotFound,
        ErrorKind::PermissionDenied => IpcErrorCode::Forbidden,
        ErrorKind::InvalidInput | ErrorKind::InvalidFilename => IpcErrorCode::InvalidInput,
        ErrorKind::TimedOut => IpcErrorCode::Timeout,
        _ => IpcErrorCode::Io,
    }
}

/// An error of this module with a text of its own, for a rule that no system call reports.
pub(super) fn refusal(kind: ErrorKind, text: &'static str) -> io::Error {
    io::Error::new(kind, text)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_system_error_is_told_by_its_kind() {
        let error = ipc_error(&io::Error::from(ErrorKind::PermissionDenied));

        assert_eq!(error.code, IpcErrorCode::Forbidden);
        assert_eq!(error.message, "permission denied");
    }

    #[test]
    fn an_error_with_its_own_text_keeps_it() {
        let error = ipc_error(&refusal(ErrorKind::InvalidInput, "the path must be absolute"));

        assert_eq!(error.code, IpcErrorCode::InvalidInput);
        assert_eq!(error.message, "the path must be absolute");
    }

    #[test]
    fn an_unknown_error_is_an_io_error_without_details() {
        let error = ipc_error(&io::Error::from(ErrorKind::Other));

        assert_eq!(error.code, IpcErrorCode::Io);
        assert_eq!(error.message, "the disk reported an error");
    }
}

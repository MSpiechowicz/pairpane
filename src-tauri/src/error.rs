//! The error type IPC commands return.

use pairpane_core::model::{IpcError, IpcErrorCode};
use serde::Serialize;

/// Result of an IPC command.
pub type CommandResult<T> = Result<T, CommandError>;

/// A failed IPC command. Serializes exactly as [`IpcError`]: `{ "code", "message" }`.
///
/// `Forbidden` messages are replaced by a generic text: they may carry details (a path, a signature failure)
/// that belong in the log, not in the UI. The UI branches on `code` only.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommandError(IpcError);

/// Shown instead of the detailed message for every `Forbidden` error.
const FORBIDDEN_MESSAGE: &str = "refused by a security check";

impl CommandError {
    /// An error with `code`; `message` must be safe to display.
    pub fn new(code: IpcErrorCode, message: impl Into<String>) -> Self {
        let message = match code {
            IpcErrorCode::Forbidden => FORBIDDEN_MESSAGE.to_owned(),
            _ => message.into(),
        };

        Self(IpcError::new(code, message))
    }

    /// An error with `code` whose `message` is shown as it is, `Forbidden` too. Callers must pass only a fixed text that
    /// holds no path, host, user or secret (the messages of `pairpane_remote`, which the user needs to act on: a host key
    /// that changed, a sign-in that was not accepted): the message goes to the page and to the log as it is, so a text
    /// built from anything the user or a server supplied must not come here.
    pub(crate) fn fixed(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self(IpcError::new(code, message))
    }

    /// The error category (what the UI branches on).
    #[cfg(test)]
    pub fn code(&self) -> IpcErrorCode {
        self.0.code
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serializes_as_ipc_error() {
        let error = CommandError::new(IpcErrorCode::NotFound, "no update to install");

        let value = serde_json::to_value(&error).expect("serializable");

        assert_eq!(value, json!({ "code": "notFound", "message": "no update to install" }));
    }

    #[test]
    fn a_fixed_forbidden_error_keeps_its_message() {
        let error = CommandError::fixed(IpcErrorCode::Forbidden, "the server's host key was not trusted");

        let value = serde_json::to_value(&error).expect("serializable");

        assert_eq!(
            value,
            json!({ "code": "forbidden", "message": "the server's host key was not trusted" })
        );
    }

    #[test]
    fn forbidden_errors_hide_the_details() {
        let error = CommandError::new(IpcErrorCode::Forbidden, "signature mismatch for /tmp/update.AppImage");

        let value = serde_json::to_value(&error).expect("serializable");

        assert_eq!(value, json!({ "code": "forbidden", "message": FORBIDDEN_MESSAGE }));
    }
}

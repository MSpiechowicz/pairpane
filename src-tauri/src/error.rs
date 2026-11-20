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
    fn forbidden_errors_hide_the_details() {
        let error = CommandError::new(IpcErrorCode::Forbidden, "signature mismatch for /tmp/update.AppImage");

        let value = serde_json::to_value(&error).expect("serializable");

        assert_eq!(value, json!({ "code": "forbidden", "message": FORBIDDEN_MESSAGE }));
    }
}

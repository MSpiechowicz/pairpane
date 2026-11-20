use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stable error category returned over IPC; the UI branches on this, never on the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum IpcErrorCode {
    /// The request was malformed or failed validation.
    InvalidInput,
    /// A file, folder or other resource does not exist.
    NotFound,
    /// The operation is refused by a security rule (deny-list, path containment, ...).
    Forbidden,
    /// A filesystem, network or other I/O failure.
    Io,
    /// The feature is not available in this build or on this platform (e.g. updates without a key).
    Unsupported,
    /// A remote service answered with something unexpected (e.g. a malformed update manifest).
    Protocol,
    /// A network operation took too long.
    Timeout,
    /// An external program failed (e.g. the package manager installing an update).
    Process,
    /// A bug or an unexpected internal state.
    Internal,
    /// The request cannot start yet because an earlier one has not finished (a folder that does not respond); trying
    /// again shortly may work.
    Busy,
}

impl IpcError {
    /// An error with `code` and a `message` safe to display.
    pub fn new(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Error payload of every failed IPC command.
///
/// `message` is human-readable and must never contain secret values or file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IpcError {
    /// Machine-readable category.
    pub code: IpcErrorCode,
    /// Short description safe to display.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn codes_serialize_in_camel_case() {
        let codes = [
            (IpcErrorCode::InvalidInput, "invalidInput"),
            (IpcErrorCode::NotFound, "notFound"),
            (IpcErrorCode::Forbidden, "forbidden"),
            (IpcErrorCode::Io, "io"),
            (IpcErrorCode::Unsupported, "unsupported"),
            (IpcErrorCode::Protocol, "protocol"),
            (IpcErrorCode::Timeout, "timeout"),
            (IpcErrorCode::Process, "process"),
            (IpcErrorCode::Internal, "internal"),
            (IpcErrorCode::Busy, "busy"),
        ];

        for (code, wire) in codes {
            assert_eq!(serde_json::to_value(code).expect("serializable"), json!(wire));
        }
    }

    #[test]
    fn error_serializes_code_and_message() {
        let error = IpcError {
            code: IpcErrorCode::NotFound,
            message: "no such folder".to_owned(),
        };

        let value = serde_json::to_value(error).expect("serializable");

        assert_eq!(value, json!({ "code": "notFound", "message": "no such folder" }));
    }
}

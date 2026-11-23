use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies one command of the command line. Monotonic within a run and never reused; a `number` in TypeScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShellId(#[ts(type = "number")] pub u64);

/// What a command of the command line sends while it runs: `output` (its standard output and error as they come, as
/// text, with terminal escapes left out), then one `done`, or one `error` when it could not be started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ShellEvent {
    Output {
        text: String,
    },
    Done {
        /// The exit code; `None` when the command was ended by a signal or its code is not known.
        code: Option<i32>,
        /// It was stopped (by the user, or because the page went away).
        stopped: bool,
        /// More output came than is sent; the rest was read and left out.
        truncated: bool,
    },
    Error {
        error: super::IpcError,
    },
}

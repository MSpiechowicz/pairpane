use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::IpcError;

/// Which kind of link `ops_link` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LinkKind {
    Symbolic,
    Hard,
}

/// What a move to the Trash did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrashOutcome {
    /// The token that undoes this move with `ops_untrash`; `null` on macOS, or when nothing reached the Trash.
    pub undo: Option<u32>,
    /// Set when the move stopped part way; the items before it are in the Trash.
    pub failure: Option<IpcError>,
}

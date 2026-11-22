use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A place inside an archive, as a pane opens it: the archive file and a folder in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveSpot {
    /// The absolute path of the archive file.
    pub archive: String,
    /// The folder inside the archive: `""` for its root, otherwise `a/b` (names joined with `/`, no slash at either end).
    pub dir: String,
}

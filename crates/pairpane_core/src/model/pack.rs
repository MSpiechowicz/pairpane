use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The kind of archive a pack job writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PackFormat {
    Zip,
    /// A tar compressed with Zstandard; it has no encryption.
    TarZst,
    SevenZ,
}

/// How hard a pack job compresses. The codec of the format maps each to its own settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PackLevel {
    /// Stored as it is, with no compression; not offered for a tar.zst.
    Store,
    Fast,
    Normal,
    Maximum,
}

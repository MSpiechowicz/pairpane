use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An application update announced by the release manifest (`latest.json`).
///
/// Only announced: nothing is downloaded until `update_install`, and the download is installed only when its
/// signature matches the public key built into the app and was made for this version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInfo {
    /// The announced version (semver).
    pub version: String,
    /// The running version (semver).
    pub current_version: String,
    /// Publication time (RFC 3339), when the manifest has one.
    pub date: Option<String>,
    /// Release notes as plain text, at most [`MAX_UPDATE_NOTES_BYTES`]. Untrusted: shown as text only.
    pub notes: Option<String>,
}

/// Longest release notes kept in an [`UpdateInfo`]; longer notes are cut at a character boundary.
pub const MAX_UPDATE_NOTES_BYTES: usize = 16 * 1024;

/// Stage of an update install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdatePhase {
    /// The update is being downloaded.
    Downloading,
    /// The download passed signature verification and is being installed.
    Installing,
    /// Installed; the new version runs after `app_relaunch`.
    Done,
}

/// Progress of an update install, sent over the `onProgress` channel of `update_install`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateProgress {
    /// The current stage.
    pub phase: UpdatePhase,
    /// Bytes downloaded so far.
    #[ts(type = "number")]
    pub downloaded: u64,
    /// Size of the download in bytes, when the server reported a size that still holds.
    #[ts(type = "number | null")]
    pub total: Option<u64>,
}

/// A native menu command, sent to the UI over the channel it registered with `menu_subscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MenuCommand {
    /// "Check for Updates…" was chosen.
    CheckForUpdates,
    /// The window is being closed (or the app quit) while the editor has unsaved changes: the UI asks, and answers
    /// with `app_close_confirmed` to go on.
    CloseRequested,
}

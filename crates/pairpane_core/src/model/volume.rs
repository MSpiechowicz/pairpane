use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a volume is, which decides its icon and its place in the drive menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VolumeKind {
    /// The disk the system runs from.
    System,
    /// The user's home folder (always listed, even when it is on the system disk).
    Home,
    /// A mounted local drive.
    Drive,
    /// A drive meant to be plugged and unplugged (a USB stick, a card).
    Removable,
    /// A network mount. Its free space is never measured.
    Network,
}

/// One place the drive menu offers: a volume, or the home folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Volume {
    /// What the path bar and the menu call it (`System`, `Home`, `USB`, `C:`).
    pub name: String,
    /// A second, dimmed line for the menu (a Windows volume label, a network source); `null` when there is none.
    pub label: Option<String>,
    /// The folder to open: the mount point.
    pub path: String,
    pub kind: VolumeKind,
    /// Space available to the user, in bytes; `null` when unknown or not measured (see [`VolumeEvent::Free`]).
    #[ts(type = "number | null")]
    pub free: Option<u64>,
}

/// What the volume list channel delivers: `list` first, then a `free` for every volume whose free space was measured,
/// in list order. Measuring can be slow, so the list never waits for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum VolumeEvent {
    /// Every volume, in menu order.
    List { volumes: Vec<Volume> },
    /// The space available to the user on the volume at `path`, in bytes.
    Free {
        path: String,
        #[ts(type = "number")]
        bytes: u64,
    },
}

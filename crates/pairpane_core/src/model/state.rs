use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{PaneId, SavedRename, SavedSearch};

/// The column a pane sorts by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SortColumn {
    Name,
    Ext,
    Size,
    Modified,
}

/// One tab of a pane as it is saved: its folder and sort. History, cursor and selection are not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedTab {
    /// The folder, absolute.
    pub path: String,
    pub sort: SortColumn,
    /// Sorted in descending order.
    pub desc: bool,
    /// How the tab shows its folder. Absent in a file from before views: the detailed view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub layout: Option<PaneLayout>,
}

/// How a pane shows its folder: one row per item with its columns, the names alone in several columns, or a grid of
/// thumbnails. One this
/// version does not know (a newer version's, a file edited by hand) reads as the detailed view, so that it never makes
/// the rest of the state unreadable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PaneLayout {
    #[default]
    Detailed,
    Brief,
    /// A grid of small previews of the pictures (icons for the rest).
    Thumbnails,
}

impl<'de> Deserialize<'de> for PaneLayout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;

        Ok(match value.as_str() {
            Some("brief") => Self::Brief,
            Some("thumbnails") => Self::Thumbnails,
            _ => Self::Detailed,
        })
    }
}

/// The tabs of one pane. After loading, `tabs` is empty when nothing usable was saved (the pane starts with its
/// defaults), and otherwise `active` is a valid index into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedPane {
    pub tabs: Vec<SavedTab>,
    /// Index of the active tab.
    pub active: u32,
}

/// What comes back on the next start: both panes' tabs, the active pane, the hotlist, the saved searches, the saved
/// rename rules and the keys the user set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedState {
    /// The pane that has the focus.
    pub active: PaneId,
    pub left: SavedPane,
    pub right: SavedPane,
    /// The hotlist (favourite folders), in the order they were added.
    pub hotlist: Vec<String>,
    /// The saved searches, in the order they were saved. A state file from before they existed, or a state sent without them,
    /// has none: whoever saves must send the searches it has, or they are lost.
    #[serde(default)]
    #[ts(as = "Option<Vec<SavedSearch>>", optional)]
    pub searches: Vec<SavedSearch>,
    /// The saved rename rules, in the order they were saved. Like `searches`: absent in an old file or a state sent
    /// without them, and whoever saves must send the ones it has.
    #[serde(default)]
    #[ts(as = "Option<Vec<SavedRename>>", optional)]
    pub renames: Vec<SavedRename>,
    /// The keys the user set for actions, in place of the default ones. Like `searches`: absent in an old file or a state
    /// sent without them, and whoever saves must send the ones it has.
    #[serde(default)]
    #[ts(as = "Option<Vec<KeyOverride>>", optional)]
    pub keys: Vec<KeyOverride>,
    /// What the settings page sets. Absent in a file from before it (`None`), so that the page can tell "never set" from
    /// the defaults: it then keeps what it had (the theme it cached), instead of resetting it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preferences: Option<Preferences>,
}

/// The colours of the app: the system's, or always light or dark. In the preferences, one this version does not know (a
/// newer version's) reads as the system's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// What the settings page sets. A field missing from the file (one added later) takes its default; an unknown one is
/// ignored. The external editor is not here: it is in the user's own `settings.json`, which the page never writes.
///
/// It is read field by field: a field of the wrong kind (a file edited by hand, a newer version's) takes its default,
/// and preferences that are not an object at all are the defaults, so they never make the rest of the state unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preferences {
    pub theme: ThemeChoice,
    /// The columns the panes show besides the name.
    pub show_ext: bool,
    pub show_size: bool,
    pub show_modified: bool,
    /// The extra columns of the detailed view, computed for the rows in view: the size of a picture in pixels, and the
    /// SHA-256 of a file. Off until the user turns them on.
    pub show_dimensions: bool,
    pub show_checksum: bool,
    /// The git status column: the state of each entry of a folder inside a repository. Off until the user turns it on.
    pub show_git: bool,
    /// The panes list hidden files.
    pub show_hidden: bool,
    /// Moving to the Trash asks first (deleting for good always does).
    pub confirm_trash: bool,
    /// Copy and move ask for the target folder first; without, they go to the other pane's folder at once.
    pub confirm_transfer: bool,
    /// Quitting while copies or moves still run asks first.
    pub confirm_quit_with_transfers: bool,
    /// The app looks for an update when it starts.
    pub check_updates_at_start: bool,
}

impl<'de> Deserialize<'de> for Preferences {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let defaults = Self::default();
        let flag = |name: &str, default: bool| value.get(name).and_then(serde_json::Value::as_bool).unwrap_or(default);

        Ok(Self {
            theme: match value.get("theme").and_then(serde_json::Value::as_str) {
                Some("light") => ThemeChoice::Light,
                Some("dark") => ThemeChoice::Dark,
                _ => ThemeChoice::System,
            },
            show_ext: flag("showExt", defaults.show_ext),
            show_size: flag("showSize", defaults.show_size),
            show_modified: flag("showModified", defaults.show_modified),
            show_dimensions: flag("showDimensions", defaults.show_dimensions),
            show_checksum: flag("showChecksum", defaults.show_checksum),
            show_git: flag("showGit", defaults.show_git),
            show_hidden: flag("showHidden", defaults.show_hidden),
            confirm_trash: flag("confirmTrash", defaults.confirm_trash),
            confirm_transfer: flag("confirmTransfer", defaults.confirm_transfer),
            confirm_quit_with_transfers: flag("confirmQuitWithTransfers", defaults.confirm_quit_with_transfers),
            check_updates_at_start: flag("checkUpdatesAtStart", defaults.check_updates_at_start),
        })
    }
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            show_ext: true,
            show_size: true,
            show_modified: true,
            show_dimensions: false,
            show_checksum: false,
            show_git: false,
            show_hidden: false,
            confirm_trash: true,
            confirm_transfer: true,
            confirm_quit_with_transfers: true,
            check_updates_at_start: true,
        }
    }
}

/// A key and the modifiers held with it, as the keymap writes it (`key` is `KeyboardEvent.key`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyChord {
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
}

/// The keys the user set for one action of the keymap; none leaves the action with no key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyOverride {
    pub action: String,
    pub keys: Vec<KeyChord>,
}

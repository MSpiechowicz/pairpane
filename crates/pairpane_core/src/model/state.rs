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

/// What comes back on the next start: both panes' tabs, the active pane, the hotlist, the saved searches and the saved
/// rename rules.
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
}

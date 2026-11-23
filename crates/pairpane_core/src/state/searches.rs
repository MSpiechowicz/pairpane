//! The saved searches of the state file, and the form in which the webview sends the state to be saved.
//!
//! A saved search is untrusted like everything else in the file, and it is only ever run later, with a folder chosen
//! then. So [`clean_searches`] keeps the ones that could run (see `named`).

use serde::Deserialize;

use crate::find;
use crate::model::{FindQuery, KeyOverride, PaneId, Preferences, SavedPane, SavedRename, SavedSearch, SavedState};

use super::named::{MAX_NAME_CHARS, clean_named};

/// Saved searches kept.
pub const MAX_SEARCHES: usize = 50;

/// The longest name of a saved search, in characters (after trimming).
pub const MAX_SEARCH_NAME_CHARS: usize = MAX_NAME_CHARS;

/// `searches` with everything unusable removed, the first [`MAX_SEARCHES`] of the rest kept, in their order (see
/// [`clean_named`] for what is dropped). A search whose criteria cannot run is dropped too: [`find::compile`] must accept
/// them (their syntax and consistency, and every pattern compiled within its limits).
pub(super) fn clean_searches(searches: Vec<SavedSearch>) -> Vec<SavedSearch> {
    clean_named(searches, MAX_SEARCHES, |search| &mut search.name, can_run)
}

/// Whether the criteria of `search` compile. The folder is a placeholder: it is only checked for its syntax.
fn can_run(search: &SavedSearch) -> bool {
    let root = if cfg!(windows) { "C:\\" } else { "/" };
    let query = FindQuery {
        root: root.to_owned(),
        criteria: search.criteria.clone(),
    };

    find::compile(&query).is_ok()
}

/// A state as the webview sends it to be saved: a [`SavedState`] whose `searches` and `renames` may be absent.
///
/// Absent (a page from before saved searches or rename rules, or one that forgot them) is not the same as empty: absent leaves the
/// stored searches as they are, so such a page cannot wipe them, while an explicit empty list clears them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedUpdate {
    pub active: PaneId,
    pub left: SavedPane,
    pub right: SavedPane,
    pub hotlist: Vec<String>,
    /// `None` when the page sent none (or `null`): the stored searches are kept.
    #[serde(default)]
    pub searches: Option<Vec<SavedSearch>>,
    /// `None` when the page sent none (or `null`): the stored rename rules are kept.
    #[serde(default)]
    pub renames: Option<Vec<SavedRename>>,
    /// `None` when the page sent none (or `null`): the stored keys are kept.
    #[serde(default)]
    pub keys: Option<Vec<KeyOverride>>,
    /// `None` when the page sent none (or `null`): the stored preferences are kept.
    #[serde(default)]
    pub preferences: Option<Preferences>,
}

impl SavedUpdate {
    /// The state to save: the searches, rename rules and keys sent, else those `stored` (called only when the page left
    /// out one of them, and then once).
    pub(super) fn resolve(self, stored: impl FnOnce() -> SavedState) -> SavedState {
        let (searches, renames, keys, preferences) = match (self.searches, self.renames, self.keys, self.preferences) {
            (Some(searches), Some(renames), Some(keys), Some(preferences)) => {
                (searches, renames, keys, Some(preferences))
            }
            (searches, renames, keys, preferences) => {
                let stored = stored();

                (
                    searches.unwrap_or(stored.searches),
                    renames.unwrap_or(stored.renames),
                    keys.unwrap_or(stored.keys),
                    preferences.or(stored.preferences),
                )
            }
        };

        SavedState {
            active: self.active,
            left: self.left,
            right: self.right,
            hotlist: self.hotlist,
            searches,
            renames,
            keys,
            preferences,
        }
    }
}

impl From<SavedState> for SavedUpdate {
    /// An update that states its searches, even when there are none.
    fn from(state: SavedState) -> Self {
        Self {
            active: state.active,
            left: state.left,
            right: state.right,
            hotlist: state.hotlist,
            searches: Some(state.searches),
            renames: Some(state.renames),
            keys: Some(state.keys),
            preferences: state.preferences,
        }
    }
}

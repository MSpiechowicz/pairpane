//! The saved searches of the state file, and the form in which the webview sends the state to be saved.
//!
//! A saved search is untrusted like everything else in the file, and it is only ever run later, with a folder chosen
//! then. So [`clean_searches`] keeps the ones that could run: a name that is usable and not taken, criteria that
//! [`find::compile`] accepts (their syntax and consistency, and every pattern compiled within its limits).

use std::collections::HashSet;

use serde::Deserialize;

use crate::find;
use crate::model::{FindQuery, PaneId, SavedPane, SavedRename, SavedSearch, SavedState};

/// Saved searches kept.
pub const MAX_SEARCHES: usize = 50;

/// The longest name of a saved search, in characters (after trimming).
pub const MAX_SEARCH_NAME_CHARS: usize = 64;

/// `searches` with everything unusable removed, the first [`MAX_SEARCHES`] of the rest kept, in their order.
///
/// Dropped: a name that is empty or only whitespace, one over [`MAX_SEARCH_NAME_CHARS`] characters, one that an earlier
/// kept search has (names are compared without regard to case; a dropped search does not take its name), and criteria
/// that cannot run. A kept name is the trimmed one. Searches beyond the cap are not even examined.
pub(super) fn clean_searches(searches: Vec<SavedSearch>) -> Vec<SavedSearch> {
    let mut names = HashSet::new();

    searches
        .into_iter()
        .filter_map(|search| {
            let name = search.name.trim();
            if name.is_empty() || name.chars().count() > MAX_SEARCH_NAME_CHARS {
                return None;
            }
            if names.contains(&name.to_lowercase()) || !can_run(&search) {
                return None;
            }

            names.insert(name.to_lowercase());

            Some(SavedSearch {
                name: name.to_owned(),
                criteria: search.criteria,
            })
        })
        .take(MAX_SEARCHES)
        .collect()
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
}

impl SavedUpdate {
    /// The state to save: the searches and rename rules sent, else those `stored` (called only when the page left out
    /// either, and then once).
    pub(super) fn resolve(self, stored: impl FnOnce() -> (Vec<SavedSearch>, Vec<SavedRename>)) -> SavedState {
        let (searches, renames) = match (self.searches, self.renames) {
            (Some(searches), Some(renames)) => (searches, renames),
            (searches, renames) => {
                let (stored_searches, stored_renames) = stored();

                (searches.unwrap_or(stored_searches), renames.unwrap_or(stored_renames))
            }
        };

        SavedState {
            active: self.active,
            left: self.left,
            right: self.right,
            hotlist: self.hotlist,
            searches,
            renames,
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
        }
    }
}

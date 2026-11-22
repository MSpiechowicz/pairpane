//! The Trash and the delete of the items of search results: a folder swapped for a link after the search is refused
//! before anything is touched.

use std::fs;

use tauri::async_runtime::block_on;

use super::*;
use crate::commands::testing::names;
use crate::commands::within::tests::{text, within};

#[test]
fn a_delete_in_the_folder_searched_goes_through() {
    let folder = tempfile::tempdir().expect("folder");
    fs::create_dir(folder.path().join("docs")).expect("folder");
    fs::write(folder.path().join("docs/x.txt"), "x").expect("file");

    let guard = Some(within(&text(&folder), "docs"));
    block_on(Items::default().delete_checked(text(&folder) + "/docs", names(&["x.txt"]), guard)).expect("deleted");

    assert!(!folder.path().join("docs/x.txt").exists());
}

#[cfg(unix)]
mod swapped {
    use super::*;
    use crate::commands::within::tests::{assert_changed, searched_with_docs_swapped};

    #[test]
    fn a_delete_in_a_folder_swapped_for_a_link_is_refused_and_deletes_nothing() {
        let swapped = searched_with_docs_swapped();
        let guard = Some(within(&text(&swapped.searched), "docs"));

        assert_changed(block_on(Items::default().delete_checked(
            swapped.docs.clone(),
            names(&["x.txt"]),
            guard,
        )));
        assert!(
            swapped.elsewhere.path().join("x.txt").exists(),
            "the other folder's file is still there"
        );

        // The same request without the guard is the hole the guard closes.
        block_on(Items::default().delete_checked(swapped.docs, names(&["x.txt"]), None))
            .expect("deleted through the link");
        assert!(!swapped.elsewhere.path().join("x.txt").exists());
    }

    #[test]
    fn a_move_to_the_trash_from_a_folder_swapped_for_a_link_is_refused_and_moves_nothing() {
        let swapped = searched_with_docs_swapped();
        let guard = Some(within(&text(&swapped.searched), "docs"));

        assert_changed(block_on(Items::default().trash_checked(
            swapped.docs,
            names(&["x.txt"]),
            guard,
        )));
        assert!(swapped.elsewhere.path().join("x.txt").exists());
    }
}

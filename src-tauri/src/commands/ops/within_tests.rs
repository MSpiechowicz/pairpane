//! The copy of the items of search results: a folder swapped for a link after the search is refused before a job exists.

use std::fs;

use tauri::async_runtime::block_on;

#[cfg(unix)]
use super::tests::received;
use super::tests::{running, wait_for};
use super::*;
use crate::commands::testing::names;
use crate::commands::within::tests::{text, within};

#[cfg(unix)]
#[test]
fn a_copy_from_a_folder_swapped_for_a_link_is_refused_and_no_job_is_made() {
    use crate::commands::within::tests::{assert_changed, searched_with_docs_swapped};

    let swapped = searched_with_docs_swapped();
    let target = tempfile::tempdir().expect("folder");
    let (ops, _unused, events) = running();

    let guard = Some(within(&text(&swapped.searched), "docs"));
    let started = ops.start(
        OpKind::Copy,
        swapped.docs,
        names(&["x.txt"]),
        text(&target),
        false,
        guard,
    );
    assert_changed(block_on(started));
    assert!(
        received(&events)
            .iter()
            .all(|event| !matches!(event, OpsEvent::Added { .. })),
        "no job was queued"
    );
    assert!(!target.path().join("x.txt").exists());
}

#[test]
fn a_copy_from_the_folder_searched_is_queued_and_copies() {
    let searched = tempfile::tempdir().expect("folder");
    fs::create_dir(searched.path().join("docs")).expect("folder");
    fs::write(searched.path().join("docs/x.txt"), "x").expect("file");
    let target = tempfile::tempdir().expect("folder");
    let (ops, _unused, events) = running();

    let docs = text(&searched) + "/docs";
    let guard = Some(within(&text(&searched), "docs"));
    block_on(ops.start(OpKind::Copy, docs, names(&["x.txt"]), text(&target), false, guard)).expect("queued");

    wait_for(&events, |event| matches!(event, OpsEvent::Ended { .. }));
    assert_eq!(fs::read_to_string(target.path().join("x.txt")).expect("copied"), "x");
}

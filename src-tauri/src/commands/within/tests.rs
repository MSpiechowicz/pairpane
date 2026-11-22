//! Tests of the guard: a folder is the one that was searched, or is refused with the fixed text; a bad `dir` is refused
//! before a thread is taken; the check has places of its own.

use std::fs;

use pairpane_core::model::IpcErrorCode;
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::testing::{elsewhere, root};

const PLACES: usize = 2;

/// What the page sends: `within: { root, dir }`.
pub(crate) fn within(root: &str, dir: &str) -> Within {
    serde_json::from_value(serde_json::json!({ "root": root, "dir": dir })).expect("the shape the page sends")
}

pub(crate) fn text(folder: &tempfile::TempDir) -> String {
    folder.path().to_string_lossy().into_owned()
}

/// A searched folder whose `docs` was swapped, after the search, for a link to another folder that has an `x.txt` of its
/// own.
#[cfg(unix)]
pub(crate) struct Swapped {
    pub(crate) searched: tempfile::TempDir,
    pub(crate) elsewhere: tempfile::TempDir,
    /// The path of `docs`, which still resolves.
    pub(crate) docs: String,
}

#[cfg(unix)]
pub(crate) fn searched_with_docs_swapped() -> Swapped {
    let searched = tempfile::tempdir().expect("folder");
    fs::create_dir(searched.path().join("docs")).expect("folder");
    fs::write(searched.path().join("docs/x.txt"), "x").expect("file");
    let elsewhere = tempfile::tempdir().expect("folder");
    fs::write(elsewhere.path().join("x.txt"), "another file").expect("file");

    fs::remove_dir_all(searched.path().join("docs")).expect("removed");
    std::os::unix::fs::symlink(elsewhere.path(), searched.path().join("docs")).expect("link");
    let docs = text(&searched) + "/docs";

    Swapped {
        searched,
        elsewhere,
        docs,
    }
}

/// A folder holding `docs/deep/x.txt`.
fn searched() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    fs::create_dir_all(folder.path().join("docs/deep")).expect("folders");
    fs::write(folder.path().join("docs/deep/x.txt"), "x").expect("file");

    folder
}

/// What the page is told of a refusal: its code and its message.
pub(crate) fn told(error: &CommandError) -> (IpcErrorCode, String) {
    let sent = serde_json::to_value(error).expect("serializable");

    (error.code(), sent["message"].as_str().expect("a message").to_owned())
}

/// Checks that `result` is the refusal of a folder that is not the one searched.
#[cfg(unix)]
pub(crate) fn assert_changed<T: std::fmt::Debug>(result: CommandResult<T>) {
    let error = result.expect_err("refused");

    assert_eq!(told(&error), (IpcErrorCode::InvalidInput, CHANGED.to_owned()));
}

fn check(cap: &ThreadCap, within: Option<Within>, path: &str) -> CommandResult<()> {
    block_on(confirm(cap, PLACES, within, path))
}

#[test]
fn without_a_guard_there_is_nothing_to_check_and_no_folder_is_touched() {
    assert!(check(&ThreadCap::default(), None, &root()).is_ok());
}

#[test]
fn the_folder_searched_and_the_folders_below_it_are_what_they_were() {
    let folder = searched();
    let top = text(&folder);
    let cap = ThreadCap::default();

    for (dir, path) in [
        ("", folder.path().to_owned()),
        ("docs", folder.path().join("docs")),
        ("docs/deep", folder.path().join("docs/deep")),
    ] {
        let result = check(&cap, Some(within(&top, dir)), &path.to_string_lossy());

        assert!(result.is_ok(), "{dir:?}: {result:?}");
    }
}

#[test]
fn a_folder_that_is_not_the_one_named_is_refused_with_the_fixed_text() {
    let folder = searched();
    let top = text(&folder);
    let other = tempfile::tempdir().expect("folder");
    let cap = ThreadCap::default();

    for (dir, path) in [
        ("docs", text(&other)),
        ("docs", folder.path().join("docs/deep").to_string_lossy().into_owned()),
        ("missing", folder.path().join("docs").to_string_lossy().into_owned()),
        (
            "docs/deep/x.txt",
            folder.path().join("docs").to_string_lossy().into_owned(),
        ),
    ] {
        let error = check(&cap, Some(within(&top, dir)), &path).expect_err(dir);

        assert_eq!(told(&error), (IpcErrorCode::InvalidInput, CHANGED.to_owned()), "{dir}");
    }
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_after_the_search_is_refused_though_its_path_still_resolves() {
    let folder = searched();
    let top = text(&folder);
    let lookalike = tempfile::tempdir().expect("folder");
    fs::create_dir_all(lookalike.path().join("deep")).expect("folders");
    fs::write(lookalike.path().join("deep/x.txt"), "another file").expect("file");

    fs::remove_dir_all(folder.path().join("docs")).expect("removed");
    std::os::unix::fs::symlink(lookalike.path(), folder.path().join("docs")).expect("link");
    let reached = folder.path().join("docs/deep");
    assert!(reached.join("x.txt").exists(), "the path leads into the other folder");

    for dir in ["docs", "docs/deep"] {
        let path = folder.path().join(dir).to_string_lossy().into_owned();
        let error = check(&ThreadCap::default(), Some(within(&top, dir)), &path).expect_err(dir);

        assert_eq!(told(&error), (IpcErrorCode::InvalidInput, CHANGED.to_owned()), "{dir}");
    }
}

#[test]
fn a_dir_that_is_not_plain_names_is_refused_before_a_thread_is_taken() {
    let cap = ThreadCap::default();
    let _taken: Vec<_> = (0..PLACES).map(|_| cap.acquire(PLACES).expect("a place")).collect();
    let too_deep = vec!["d"; MAX_FOLDERS + 1].join("/");

    for dir in ["..", "a/../b", "/a", "a/", "a//b", "a\0b", too_deep.as_str()] {
        let error = check(&cap, Some(within(&root(), dir)), &root()).expect_err(dir);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{dir:?}");
    }

    for (searched, path) in [("relative", root()), (root().as_str(), "relative".to_owned())] {
        let error = check(&cap, Some(within(searched, "")), &path).expect_err(searched);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{searched}");
    }
}

#[test]
fn the_check_has_places_of_its_own_and_is_busy_when_they_are_taken() {
    let folder = searched();
    let top = text(&folder);
    let cap = ThreadCap::default();
    let taken: Vec<_> = (0..PLACES).map(|_| cap.acquire(PLACES).expect("a place")).collect();

    let busy = check(&cap, Some(within(&top, "")), &top).expect_err("busy");
    assert_eq!(busy.code(), IpcErrorCode::Busy);

    drop(taken);
    check(&cap, Some(within(&top, "")), &top).expect("a place is free again");
    assert!(
        cap.acquire(PLACES).is_some() && cap.acquire(PLACES).is_some(),
        "the place was given back"
    );
}

#[test]
fn the_page_sends_root_and_dir() {
    let sent: Within = serde_json::from_str(r#"{"root":"/r","dir":"a/b"}"#).expect("parsed");

    assert_eq!((sent.root.as_str(), sent.dir.as_str()), ("/r", "a/b"));
    assert!(serde_json::from_str::<Within>(r#"{"root":"/r"}"#).is_err());
    assert_ne!(elsewhere(), root());
}

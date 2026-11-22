//! Tests of `diff_next_difference`: the sessions it needs, its cap and the stop when a session ends.

use pairpane_core::model::ByteDifference;
use tauri::async_runtime::block_on;

use super::tests::{code, fill, folder_with, open};
use super::*;

fn ask(viewer: &Viewer, left: u32, right: u32, from: u64, forward: bool) -> CommandResult<ByteDifference> {
    block_on(viewer.next_difference(left, right, from, forward))
}

fn message(error: &CommandError) -> String {
    let shown = serde_json::to_value(error).expect("json");

    shown["message"].as_str().expect("message").to_owned()
}

#[test]
fn a_session_that_is_not_there_is_not_found() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello\n")]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "a.txt").session;

    assert_eq!(code(ask(&viewer, session, 99, 0, true)), IpcErrorCode::NotFound);
    assert_eq!(code(ask(&viewer, 99, session, 0, false)), IpcErrorCode::NotFound);

    let error = ask(&viewer, 98, 99, 0, true).expect_err("no sessions");
    assert_eq!(message(&error), "there is no such viewer session");
}

#[test]
fn finds_the_differences_of_two_sessions_in_both_directions() {
    let (_folder, path) = folder_with(&[("a.bin", b"abcdef"), ("b.bin", b"abXdefg")]);
    let viewer = Viewer::default();
    let (left, right) = (
        open(&viewer, &path, "a.bin").session,
        open(&viewer, &path, "b.bin").session,
    );

    assert_eq!(
        ask(&viewer, left, right, 0, true).expect("a scan"),
        ByteDifference::Found { offset: 2 }
    );
    assert_eq!(
        ask(&viewer, left, right, 3, true).expect("a scan"),
        ByteDifference::Found { offset: 6 }
    );
    assert_eq!(
        ask(&viewer, left, right, 7, true).expect("a scan"),
        ByteDifference::None
    );
    assert_eq!(
        ask(&viewer, left, right, 7, false).expect("a scan"),
        ByteDifference::Found { offset: 6 }
    );
    assert_eq!(
        ask(&viewer, left, right, 2, false).expect("a scan"),
        ByteDifference::None
    );
    assert_eq!(ask(&viewer, left, left, 0, true).expect("a scan"), ByteDifference::None);
}

#[test]
fn the_scan_takes_a_place_of_the_search_cap_and_leaves_the_read_cap_alone() {
    let (_folder, path) = folder_with(&[("a.bin", b"abc"), ("b.bin", b"abd")]);
    let viewer = Viewer::default();
    let (left, right) = (
        open(&viewer, &path, "a.bin").session,
        open(&viewer, &path, "b.bin").session,
    );

    let searches = fill(&viewer.searches, SEARCH_THREADS);
    assert_eq!(code(ask(&viewer, left, right, 0, true)), IpcErrorCode::Busy);
    drop(searches);

    let _reads = fill(&viewer.reads, READ_THREADS);
    assert_eq!(
        ask(&viewer, left, right, 0, true).expect("a scan"),
        ByteDifference::Found { offset: 2 }
    );
}

#[test]
fn a_session_that_ends_stops_the_scan_as_a_cancelled_read_without_a_path() {
    let (_folder, path) = folder_with(&[("a.bin", b"abc"), ("b.bin", b"abc")]);
    let viewer = Viewer::default();
    let (left, right) = (
        open(&viewer, &path, "a.bin").session,
        open(&viewer, &path, "b.bin").session,
    );
    let handle = viewer.handle(right).expect("open session");
    handle.cancel.store(true, Ordering::Relaxed);

    let error = ask(&viewer, left, right, 0, true).expect_err("stopped");

    assert_eq!(error.code(), IpcErrorCode::Io);
    assert_eq!(message(&error), "the read was cancelled");
}

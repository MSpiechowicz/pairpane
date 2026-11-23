//! What is remembered of the pictures that could not be made, and what is not (see `cache.rs`): a file that cannot be made
//! into a picture is not tried again until it changes, and what is the state of the moment is never kept.

use std::io;

use pairpane_core::model::IpcErrorCode;
use pairpane_core::view::refusal;

use super::tests::{PNG_HEAD, fill, get, shown, too_big_file};
use super::*;
use crate::commands::testing::folder_with;

#[test]
fn a_file_that_cannot_be_made_into_a_picture_is_remembered_and_not_tried_again() {
    let (folder, path) = folder_with(&[]);
    too_big_file(folder.path(), "huge.png");
    let thumbs = Thumbs::default();

    let first = shown(get(&thumbs, &path, "huge.png", 128));
    assert_eq!(
        first,
        (
            IpcErrorCode::InvalidInput,
            "the image is too big to show here".to_owned()
        )
    );
    // The failure, the path of the key and the entry itself: a few hundred bytes, whatever the text.
    let path_len = folder.path().join("huge.png").as_os_str().len();
    assert_eq!(
        thumbs.cache().bytes(),
        cache::FAILURE_BYTES + path_len + cache::ENTRY_BYTES,
        "a small fixed cost"
    );

    // Every place for reading and for the helpers is taken: only what is remembered can answer.
    let _readers = fill(&thumbs.readers, READER_THREADS);
    let _helpers = fill(&thumbs.helpers, HELPER_THREADS);
    for _ in 0..3 {
        assert_eq!(
            shown(get(&thumbs, &path, "huge.png", 128)),
            first,
            "the same code and text"
        );
    }

    // Another size is another try, which finds the places taken.
    assert_eq!(shown(get(&thumbs, &path, "huge.png", 400)).0, IpcErrorCode::Busy);
}

#[test]
fn a_remembered_failure_is_forgotten_when_the_file_changes() {
    let (folder, path) = folder_with(&[]);
    too_big_file(folder.path(), "huge.png");
    let thumbs = Thumbs::default();
    assert_eq!(
        shown(get(&thumbs, &path, "huge.png", 128)).0,
        IpcErrorCode::InvalidInput
    );

    // The file is now a small one: its length differs, so it is tried again (and the helper is not there).
    std::fs::write(folder.path().join("huge.png"), PNG_HEAD).expect("rewritten");

    assert_eq!(shown(get(&thumbs, &path, "huge.png", 128)).0, IpcErrorCode::Io);
    assert_eq!(thumbs.cache().bytes(), 0);
}

#[test]
fn a_failure_of_the_system_is_not_remembered() {
    let (_folder, path) = folder_with(&[("a.png", PNG_HEAD)]);
    let thumbs = Thumbs::default();

    // The helper is not there: not the file's fault, and the next request tries again.
    assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).0, IpcErrorCode::Io);
    assert_eq!(thumbs.cache().bytes(), 0);

    thumbs.shut_down();
    assert_eq!(shown(get(&thumbs, &path, "a.png", 128)).1, "the read was cancelled");
    assert_eq!(thumbs.cache().bytes(), 0, "a cancelled request is not a bad picture");
}

#[test]
fn only_what_the_file_decided_is_remembered() {
    // The file's own verdicts.
    for (error, code) in [
        (
            refusal(io::ErrorKind::InvalidData, "the image cannot be decoded"),
            IpcErrorCode::InvalidInput,
        ),
        (
            refusal(io::ErrorKind::InvalidInput, "the image is too big to show here"),
            IpcErrorCode::InvalidInput,
        ),
        (
            refusal(
                io::ErrorKind::Unsupported,
                "the file is not a picture that has a thumbnail",
            ),
            IpcErrorCode::Unsupported,
        ),
    ] {
        assert_eq!(remembered(&error).expect("remembered").code, code, "{error}");
    }

    // The state of the moment: a helper past its deadline (the page is told as for a picture that cannot be decoded, but
    // nothing is kept), threads or a helper that did not start, a cancelled request, a failed read.
    let slow = pairpane_core::view::passing_refusal(io::ErrorKind::InvalidData, "the image cannot be decoded");
    assert!(remembered(&slow).is_none(), "a missed deadline is not remembered");
    assert_eq!(
        thumb_error(&slow).code(),
        IpcErrorCode::InvalidInput,
        "but the page is told the same"
    );
    for error in [
        io::Error::other("the helper's threads did not start"),
        io::Error::other("the helper did not start"),
        refusal(io::ErrorKind::Interrupted, "the read was cancelled"),
        io::Error::from(io::ErrorKind::PermissionDenied),
        io::Error::from(io::ErrorKind::NotFound),
    ] {
        assert!(remembered(&error).is_none(), "{error}");
    }
}

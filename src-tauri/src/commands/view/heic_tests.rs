//! Tests of the HEIC commands of the viewer: what is refused before a thread is taken, the place a decode holds, and
//! what the page is told. The decoding itself, in a process of its own, is tested in `crates/pairpane_core` and by
//! `tests/heic_helper.rs`, which runs the real executable.

use std::io;
use std::path::PathBuf;

use pairpane_core::model::IpcErrorCode;
use pairpane_core::view::refusal;
use tauri::async_runtime::block_on;

use super::tests::{code, fill, folder_with, open};
use super::*;

/// A 16 by 8 HEIC picture of one colour (the fixture of the core crate's tests).
const TINY_HEIC: &[u8] = include_bytes!("../../../../crates/pairpane_core/src/view/heic_testdata/tiny.heic");

/// An AV1 image in a file of the brands `mif1` and `heic`: the viewer takes it for a HEIC image, and its items say it is not.
// Used only where the backend decodes (see the tests below).
#[cfg(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
))]
const AV1_HEIC: &[u8] = include_bytes!("../../../../crates/pairpane_core/src/view/heic_testdata/av1_mif1_heic.heic");

/// Where a helper would be; the tests that must not start one never reach it.
fn nowhere() -> PathBuf {
    PathBuf::from("/nonexistent/pairpane-helper")
}

fn message<T: std::fmt::Debug>(result: CommandResult<T>) -> String {
    let shown = serde_json::to_value(result.expect_err("refused")).expect("json");

    shown["message"].as_str().expect("message").to_owned()
}

#[test]
fn a_file_that_is_not_a_heic_image_is_refused_before_a_thread_is_taken() {
    let (_folder, path) = folder_with(&[
        ("a.txt", b"hello"),
        ("a.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
        ("tiny.heic", TINY_HEIC),
    ]);
    let viewer = Viewer::default();
    let text = open(&viewer, &path, "a.txt").session;
    let png = open(&viewer, &path, "a.png").session;
    let heic = open(&viewer, &path, "tiny.heic").session;
    let _decodes = fill(&viewer.decodes, DECODE_THREADS);

    for session in [text, png] {
        let refused = block_on(viewer.image_decode(session, nowhere()));
        assert_eq!(message(refused), "the file is not a heic image");
    }
    assert_eq!(
        code(block_on(viewer.image_decode(text, nowhere()))),
        IpcErrorCode::InvalidInput,
        "refused for what it is, not for a full cap"
    );
    // A HEIC picture is not refused for its kind: here only the full cap stops it.
    assert_eq!(code(block_on(viewer.image_decode(heic, nowhere()))), IpcErrorCode::Busy);
}

// Only where HEIC is decoded by the backend (on macOS the webview does it and the backend refuses at once).
#[cfg(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
))]
#[test]
fn a_file_of_another_codec_in_a_heic_container_is_refused_before_a_helper_is_started() {
    let (_folder, path) = folder_with(&[("av1.heic", AV1_HEIC)]);
    let viewer = Viewer::default();
    let opened = open(&viewer, &path, "av1.heic");
    assert_eq!(
        opened.kind,
        ViewKind::Image {
            mime: "image/heic".to_owned()
        }
    );

    // The helper is not there: had it been started, the failure would be a plain one and not this refusal.
    let refused = block_on(viewer.image_decode(opened.session, nowhere()));

    assert_eq!(
        refused.as_ref().expect_err("refused").code(),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(message(refused), "the file is not a heic image");
}

#[test]
fn a_decode_has_a_place_of_its_own_and_takes_no_place_of_the_reads() {
    let (_folder, path) = folder_with(&[("tiny.heic", TINY_HEIC)]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "tiny.heic").session;

    // A decode that runs (here: a full decode cap) does not stop the reads and the opens, ...
    let decodes = fill(&viewer.decodes, DECODE_THREADS);
    assert_eq!(block_on(viewer.bytes(session, 4, 4)).expect("a read"), b"ftyp");
    open(&viewer, &path, "tiny.heic");
    drop(decodes);

    // ... and the reads, all busy, do not stop the decode from being tried: a decode that finds its place taken is the
    // only one refused as busy (a helper that is not there is a plain failure of the read).
    let _reads = fill(&viewer.reads, READ_THREADS);
    let failed = block_on(viewer.image_decode(session, nowhere()));
    assert_ne!(code(failed), IpcErrorCode::Busy);
}

#[test]
fn decoding_the_picture_of_a_session_that_is_not_there_is_not_found() {
    let viewer = Viewer::default();

    assert_eq!(
        code(block_on(viewer.image_decode(7, nowhere()))),
        IpcErrorCode::NotFound
    );
}

// Only where HEIC is decoded by the backend (on macOS the webview does it and the backend refuses at once).
#[cfg(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
))]
#[test]
fn a_helper_that_cannot_be_started_is_a_plain_failure_without_its_text() {
    let (_folder, path) = folder_with(&[("tiny.heic", TINY_HEIC)]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "tiny.heic").session;

    let failed = block_on(viewer.image_decode(session, nowhere()));

    assert_eq!(failed.as_ref().expect_err("no helper").code(), IpcErrorCode::Io);
    assert_eq!(message(failed), "the file cannot be read");
}

/// A helper that exits at once without an answer (the program `true`, which ignores its arguments): the failure of
/// the decode is the page's "cannot be decoded", with the text of the viewer and not a library's.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
#[test]
fn a_helper_that_gives_no_answer_is_a_picture_that_cannot_be_decoded() {
    let (_folder, path) = folder_with(&[("tiny.heic", TINY_HEIC)]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "tiny.heic").session;

    let failed = block_on(viewer.image_decode(session, PathBuf::from("true")));

    assert_eq!(failed.as_ref().expect_err("no answer").code(), IpcErrorCode::Io);
    assert_eq!(message(failed), "the image cannot be decoded");
}

#[test]
fn only_the_viewers_own_refusals_show_their_text() {
    for (kind, text, expected) in [
        (
            io::ErrorKind::Unsupported,
            "no heic decoder on this system",
            IpcErrorCode::Unsupported,
        ),
        (
            io::ErrorKind::InvalidData,
            "the image cannot be decoded",
            IpcErrorCode::Io,
        ),
        (
            io::ErrorKind::InvalidInput,
            "the image is too big to show here",
            IpcErrorCode::InvalidInput,
        ),
    ] {
        let ours = refusal(kind, text);
        let shown = serde_json::to_value(view_error(&ours)).expect("json");
        assert_eq!(view_error(&ours).code(), expected);
        assert_eq!(shown["message"], text);

        // The same text and kind from anywhere else (a library, a system call) is not shown; the page gets the generic
        // text, and a path or a library's wording in it can never leak.
        let theirs = io::Error::new(kind, text);
        let shown = serde_json::to_value(view_error(&theirs)).expect("json");
        assert_eq!(view_error(&theirs).code(), IpcErrorCode::Io);
        assert_eq!(shown["message"], "the file cannot be read");
    }

    // The system's own "unsupported" (a syscall) has no text of ours either.
    let os = io::Error::from(io::ErrorKind::Unsupported);
    let shown = serde_json::to_value(view_error(&os)).expect("json");
    assert_eq!(view_error(&os).code(), IpcErrorCode::Io);
    assert_eq!(shown["message"], "the file cannot be read");
}

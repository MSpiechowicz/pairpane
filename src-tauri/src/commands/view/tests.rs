//! Tests of the viewer commands: what is refused before a thread is taken, the sessions and their limits, the reads and
//! the search with its cancelling. The media scheme is tested in `serve_tests.rs`.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pairpane_core::model::{IpcErrorCode, SearchNeedle, SearchQuery, TextQuery, ViewKind, ViewOpened};
use pairpane_core::view::MAX_READ;
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::limit::Permit;
pub(super) use crate::commands::testing::folder_with;
use crate::commands::testing::root;

pub(super) fn open(viewer: &Viewer, path: &str, name: &str) -> ViewOpened {
    block_on(viewer.open(path.to_owned(), name.to_owned())).expect("opened")
}

pub(super) fn code<T: std::fmt::Debug>(result: CommandResult<T>) -> IpcErrorCode {
    result.expect_err("refused").code()
}

/// Takes every place of `cap`.
pub(super) fn fill(cap: &ThreadCap, max: usize) -> Vec<Permit> {
    (0..max).map(|_| cap.acquire(max).expect("a place")).collect()
}

fn text_query(encoding: &str) -> TextQuery {
    TextQuery {
        encoding: encoding.to_owned(),
        offset: 0,
        backward: false,
        lines: 10,
        snap: false,
    }
}

fn find(text: &str, from: u64) -> SearchQuery {
    SearchQuery {
        needle: SearchNeedle::Text {
            text: text.to_owned(),
            case_sensitive: true,
        },
        encoding: "UTF-8".to_owned(),
        from,
        backward: false,
    }
}

fn hex(bytes: &[u8]) -> SearchQuery {
    SearchQuery {
        needle: SearchNeedle::Hex { bytes: bytes.to_vec() },
        ..find("x", 0)
    }
}

#[test]
fn arguments_that_are_not_valid_are_refused_before_a_thread_is_taken() {
    let viewer = Viewer::default();
    let _reads = fill(&viewer.reads, READ_THREADS);
    let _searches = fill(&viewer.searches, SEARCH_THREADS);

    for name in ["", ".", "..", "a/b", "../x", "x\0y"] {
        let refused = block_on(viewer.open(root(), name.to_owned()));
        assert_eq!(code(refused), IpcErrorCode::InvalidInput, "{name:?}");
    }
    assert_eq!(
        code(block_on(viewer.open(root(), "n".repeat(300)))),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(block_on(viewer.open("relative".to_owned(), "a".to_owned()))),
        IpcErrorCode::InvalidInput
    );
    // A good request waits for a place when the cap is full.
    assert_eq!(code(block_on(viewer.open(root(), "a".to_owned()))), IpcErrorCode::Busy);

    // These do not need a session to be refused, nor a place.
    let too_long = u32::try_from(MAX_READ).expect("fits") + 1;
    assert_eq!(code(block_on(viewer.bytes(1, 0, too_long))), IpcErrorCode::InvalidInput);
    assert_eq!(
        code(block_on(viewer.text(1, text_query("no-such-encoding")))),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(block_on(viewer.markdown(1, "no-such-encoding".to_owned()))),
        IpcErrorCode::InvalidInput
    );
    let mut odd = find("a", 0);
    odd.encoding = "no-such-encoding".to_owned();
    for query in [odd, find("", 0), find(&"a".repeat(300), 0), hex(&[]), hex(&[1; 300])] {
        let refused = block_on(viewer.search(1, query, |_| {}));
        assert_eq!(code(refused), IpcErrorCode::InvalidInput);
    }
}

#[test]
fn a_session_that_is_not_there_is_not_found_and_closing_it_is_not_an_error() {
    let viewer = Viewer::default();

    assert_eq!(code(block_on(viewer.bytes(7, 0, 1))), IpcErrorCode::NotFound);
    assert_eq!(
        code(block_on(viewer.text(7, text_query("UTF-8")))),
        IpcErrorCode::NotFound
    );
    assert_eq!(
        code(block_on(viewer.markdown(7, "UTF-8".to_owned()))),
        IpcErrorCode::NotFound
    );
    assert_eq!(code(block_on(viewer.archive(7))), IpcErrorCode::NotFound);
    assert_eq!(
        code(block_on(viewer.search(7, find("a", 0), |_| {}))),
        IpcErrorCode::NotFound
    );
    assert_eq!(code(viewer.cancel_search(7)), IpcErrorCode::NotFound);
    assert_eq!(code(viewer.mint(7, Instant::now())), IpcErrorCode::NotFound);

    viewer.close(7);
    viewer.close(7);
}

#[test]
fn closing_twice_is_safe_and_a_closed_session_is_not_found() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello\n")]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "a.txt").session;
    block_on(viewer.bytes(session, 0, 5)).expect("open session");

    viewer.close(session);
    viewer.close(session);

    assert_eq!(code(block_on(viewer.bytes(session, 0, 5))), IpcErrorCode::NotFound);
}

#[test]
fn closing_all_sessions_as_the_app_exits_raises_every_flag_and_ends_every_session() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello\n")]);
    let viewer = Viewer::default();
    let sessions: Vec<u32> = (0..3).map(|_| open(&viewer, &path, "a.txt").session).collect();
    let flags: Vec<_> = sessions
        .iter()
        .map(|session| viewer.handle(*session).expect("open session").cancel)
        .collect();
    assert!(flags.iter().all(|flag| !flag.load(Ordering::Relaxed)));

    viewer.close_all();

    assert!(
        flags.iter().all(|flag| flag.load(Ordering::Relaxed)),
        "the reads that run (a decode with its helper process) are told to stop"
    );
    for session in sessions {
        assert_eq!(code(block_on(viewer.bytes(session, 0, 5))), IpcErrorCode::NotFound);
    }
    viewer.close_all();
}

#[test]
fn opening_past_eight_sessions_closes_the_oldest() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello\n")]);
    let viewer = Viewer::default();

    let sessions: Vec<u32> = (0..=MAX_SESSIONS)
        .map(|_| open(&viewer, &path, "a.txt").session)
        .collect();

    let mut all = sessions.clone();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), MAX_SESSIONS + 1, "every session has a number of its own");
    assert_eq!(
        code(block_on(viewer.bytes(sessions[0], 0, 1))),
        IpcErrorCode::NotFound,
        "the oldest was closed"
    );
    for session in &sessions[1..] {
        assert_eq!(block_on(viewer.bytes(*session, 0, 5)).expect("alive"), b"hello");
    }
}

#[test]
fn what_cannot_be_viewed_is_refused_with_a_fixed_text_that_holds_no_path() {
    let (folder, path) = folder_with(&[("a.txt", b"hello")]);
    fs::create_dir(folder.path().join("sub")).expect("folder");
    #[cfg(unix)]
    std::os::unix::fs::symlink(folder.path().join("a.txt"), folder.path().join("link")).expect("link");
    let viewer = Viewer::default();

    let mut refused = vec![("sub", IpcErrorCode::InvalidInput), ("gone", IpcErrorCode::NotFound)];
    if cfg!(unix) {
        refused.push(("link", IpcErrorCode::InvalidInput));
    }

    for (name, expected) in refused {
        let error = block_on(viewer.open(path.clone(), name.to_owned())).expect_err(name);
        let shown = serde_json::to_value(&error).expect("json");
        let message = shown["message"].as_str().expect("message");

        assert_eq!(error.code(), expected, "{name}");
        assert!(!message.contains(&path), "{message}");
        assert_eq!(message, message.to_lowercase());
    }
}

#[test]
fn bytes_are_read_at_an_offset_and_the_cap_is_enforced() {
    let (folder, path) = folder_with(&[("a.txt", b"hello world")]);
    let big = File::create(folder.path().join("big.bin")).expect("file");
    big.set_len(MAX_READ as u64 * 2).expect("sparse");
    let viewer = Viewer::default();
    let small = open(&viewer, &path, "a.txt");
    let large = open(&viewer, &path, "big.bin");
    assert_eq!((small.size, large.size), (11, MAX_READ as u64 * 2));

    assert_eq!(block_on(viewer.bytes(small.session, 6, 5)).expect("read"), b"world");
    assert_eq!(block_on(viewer.bytes(small.session, 6, 100)).expect("short"), b"world");
    assert!(block_on(viewer.bytes(small.session, 11, 5)).expect("end").is_empty());
    assert!(
        block_on(viewer.bytes(small.session, u64::MAX, 5))
            .expect("past")
            .is_empty()
    );

    let most = u32::try_from(MAX_READ).expect("fits");
    assert_eq!(
        block_on(viewer.bytes(large.session, 0, most)).expect("cap").len(),
        MAX_READ
    );
    assert_eq!(
        code(block_on(viewer.bytes(large.session, 0, most + 1))),
        IpcErrorCode::InvalidInput
    );
}

#[test]
fn text_markdown_and_archives_are_read() {
    let empty_zip: Vec<u8> = [b"PK\x05\x06".as_slice(), &[0; 18]].concat();
    let (_folder, path) = folder_with(&[
        ("a.txt", b"one\ntwo\n"),
        ("r.md", b"# Title\n\nbody\n"),
        ("z.zip", &empty_zip),
        ("p.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
    ]);
    let viewer = Viewer::default();
    let text = open(&viewer, &path, "a.txt");
    let markdown = open(&viewer, &path, "r.md");
    let zip = open(&viewer, &path, "z.zip");
    let png = open(&viewer, &path, "p.png");
    assert_eq!(text.kind, ViewKind::Text);
    assert_eq!(text.encoding, "UTF-8");
    assert_eq!(markdown.kind, ViewKind::Markdown);

    let window = block_on(viewer.text(text.session, text_query("UTF-8"))).expect("window");
    let lines: Vec<&str> = window.lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!((lines, window.at_end), (vec!["one", "two"], true));

    let document = block_on(viewer.markdown(markdown.session, "UTF-8".to_owned())).expect("document");
    assert_eq!(document.blocks.len(), 2);
    assert!(!document.truncated);
    // Text is shown as Markdown too; an image is not.
    block_on(viewer.markdown(text.session, "UTF-8".to_owned())).expect("text as markdown");
    assert_eq!(
        code(block_on(viewer.markdown(png.session, "UTF-8".to_owned()))),
        IpcErrorCode::InvalidInput
    );

    let listing = block_on(viewer.archive(zip.session)).expect("listing");
    assert!(listing.entries.is_empty());
    assert_eq!(code(block_on(viewer.archive(text.session))), IpcErrorCode::InvalidInput);
}

#[test]
fn a_broken_archive_is_an_error_with_a_fixed_text() {
    let (_folder, path) = folder_with(&[("z.zip", b"PK\x03\x04 and then nothing useful")]);
    let viewer = Viewer::default();
    let zip = open(&viewer, &path, "z.zip");

    let error = block_on(viewer.archive(zip.session)).expect_err("broken");

    assert_eq!(error.code(), IpcErrorCode::Io);
    assert!(!serde_json::to_string(&error).expect("json").contains(&path));
}

#[test]
fn a_read_that_runs_when_its_session_ends_is_stopped() {
    let empty_zip = [b"PK\x05\x06".as_slice(), &[0; 18]].concat();
    let (_folder, path) = folder_with(&[("z.zip", &empty_zip)]);
    let viewer = Viewer::default();
    let list = |handle: Handle| {
        let format = pairpane_core::model::ArchiveFormat::Zip;

        viewer.read(handle, move |file, cancel| {
            pairpane_core::view::archive(file, format, cancel)
        })
    };

    let kept = open(&viewer, &path, "z.zip").session;
    block_on(list(viewer.handle(kept).expect("handle"))).expect("a listing while the session lives");

    // Closing the session stops a read that holds its handle already.
    let handle = viewer.handle(kept).expect("handle");
    viewer.close(kept);
    let error = block_on(list(handle)).expect_err("stopped");
    assert_eq!(error.code(), IpcErrorCode::Io);
    assert!(
        serde_json::to_string(&error)
            .expect("json")
            .contains("the read was cancelled")
    );

    // So does opening one session too many, which closes the oldest.
    let oldest = open(&viewer, &path, "z.zip").session;
    let handle = viewer.handle(oldest).expect("handle");
    for _ in 0..MAX_SESSIONS {
        open(&viewer, &path, "z.zip");
    }
    assert_eq!(code(block_on(list(handle))), IpcErrorCode::Io);
}

#[test]
fn a_text_and_a_hex_search_find_their_match_and_none_answers_null() {
    let (_folder, path) = folder_with(&[("a.txt", b"one two three two\n")]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "a.txt").session;

    let hit = block_on(viewer.search(session, find("two", 0), |_| {})).expect("search");
    assert_eq!(hit.map(|hit| (hit.offset, hit.len)), Some((4, 3)));

    let next = block_on(viewer.search(session, find("two", 5), |_| {})).expect("search");
    assert_eq!(next.map(|hit| hit.offset), Some(14));

    let by_bytes = block_on(viewer.search(session, hex(b"three"), |_| {})).expect("search");
    assert_eq!(by_bytes.map(|hit| hit.offset), Some(8));

    assert!(
        block_on(viewer.search(session, find("four", 0), |_| {}))
            .expect("none")
            .is_none()
    );
}

#[test]
fn a_search_that_is_cancelled_answers_null_and_a_newer_search_stops_the_older() {
    let folder = tempfile::tempdir().expect("folder");
    let mut big = File::create(folder.path().join("big.txt")).expect("file");
    // The needle is far past the first progress report (every 64 MiB), so only a search that was cancelled misses it.
    big.set_len(200 * 1024 * 1024).expect("sparse");
    big.seek(SeekFrom::End(-6)).expect("seek");
    big.write_all(b"needle").expect("needle");
    drop(big);
    let path = folder.path().to_string_lossy().into_owned();
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "big.txt").session;

    let control = block_on(viewer.search(session, hex(b"needle"), |_| {})).expect("search");
    assert_eq!(control.map(|hit| hit.offset), Some(200 * 1024 * 1024 - 6));

    let stopper = viewer.clone();
    let cancelled = block_on(viewer.search(session, hex(b"needle"), move |_| {
        stopper.cancel_search(session).expect("session");
    }));
    assert!(cancelled.expect("cancelled").is_none());

    let progress = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&progress);
    let finished = block_on(viewer.search(session, hex(b"needle"), move |scanned| {
        seen.lock().expect("not poisoned").push(scanned);
    }));
    assert!(finished.expect("search").is_some());
    assert_eq!(
        *progress.lock().expect("not poisoned"),
        [64 * 1024 * 1024, 128 * 1024 * 1024, 192 * 1024 * 1024]
    );
}

#[test]
fn starting_a_search_cancels_the_one_that_runs_and_closing_cancels_it_too() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello\n")]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "a.txt").session;

    let first = viewer.start_search(session).expect("first");
    assert!(!first.load(Ordering::Relaxed));

    let second = viewer.start_search(session).expect("second");
    assert!(first.load(Ordering::Relaxed), "the newer search stops the older");
    assert!(!second.load(Ordering::Relaxed));

    // The older search ending does not forget the newer one.
    viewer.end_search(session, &first);
    viewer.cancel_search(session).expect("session");
    assert!(second.load(Ordering::Relaxed));

    let third = viewer.start_search(session).expect("third");
    viewer.close(session);
    assert!(third.load(Ordering::Relaxed), "closing stops the search");
}

#[test]
fn a_search_is_refused_as_busy_when_the_search_cap_is_full_and_nothing_is_stopped() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello\n")]);
    let viewer = Viewer::default();
    let session = open(&viewer, &path, "a.txt").session;
    let running = viewer.start_search(session).expect("running");
    let _held = fill(&viewer.searches, SEARCH_THREADS);

    assert_eq!(
        code(block_on(viewer.search(session, find("hello", 0), |_| {}))),
        IpcErrorCode::Busy
    );
    assert!(!running.load(Ordering::Relaxed));
}

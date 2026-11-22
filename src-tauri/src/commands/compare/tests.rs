//! Tests of the compare commands: what a scan streams, what refuses it, how many run and are kept at once, how it is stopped,
//! and an archive as one side.

use std::fs;
use std::path::Path;

use pairpane_core::model::{CompareStatus, EntryKind};

use super::*;
use crate::commands::archives::fixtures::{collecting, item, tar_of};
use crate::commands::testing::{Gate, elsewhere, folder_with, gated, wait_until};

type Log = Arc<Mutex<Vec<CompareEvent>>>;

fn plain() -> CompareOptions {
    CompareOptions {
        subfolders: true,
        content: false,
        hidden: false,
    }
}

pub(in crate::commands) fn folder(path: &str) -> CompareRoot {
    CompareRoot::Folder { path: path.to_owned() }
}

pub(in crate::commands) fn archive(path: &Path, dir: &str) -> CompareRoot {
    CompareRoot::Archive {
        archive: path.to_string_lossy().into_owned(),
        dir: dir.to_owned(),
    }
}

/// Two folders: the left holds `a.txt`, `c.log` and `sub/b.txt`, the right holds `c.log`.
fn pair() -> (tempfile::TempDir, String, tempfile::TempDir, String) {
    let (left, left_path) = folder_with(&[("a.txt", b"alpha"), ("c.log", b"gamma")]);
    fs::create_dir(left.path().join("sub")).expect("folder");
    fs::write(left.path().join("sub").join("b.txt"), b"beta").expect("file");
    let (right, right_path) = folder_with(&[("c.log", b"gamma")]);

    (left, left_path, right, right_path)
}

fn ended(log: &Log) -> bool {
    let events = log.lock().expect("not poisoned");

    matches!(
        events.last(),
        Some(CompareEvent::Done { .. } | CompareEvent::Error { .. })
    )
}

/// Starts a comparison and waits for its last event.
fn scanned(
    comparer: &Comparer,
    sides: (CompareRoot, CompareRoot),
    options: CompareOptions,
) -> (CompareId, Vec<CompareEvent>) {
    let (channel, log) = collecting();
    let id = comparer
        .start(&Archives::default(), sides, options, channel)
        .expect("started");
    wait_until("the comparison to end", || ended(&log));
    let events = log.lock().expect("not poisoned").clone();

    (id, events)
}

fn statuses(events: &[CompareEvent]) -> Vec<(String, CompareStatus)> {
    let mut rows: Vec<(String, CompareStatus)> = events
        .iter()
        .flat_map(|event| match event {
            CompareEvent::Rows { rows } => rows.clone(),
            _ => Vec::new(),
        })
        .map(|row| (row.path, row.status))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    rows
}

/// Starts a scan of two folders and waits until it has delivered its first event, so it is running (and held at the gate).
fn held(comparer: &Comparer, left: &str, right: &str) -> (CompareId, Arc<Gate>, Log) {
    let (channel, gate, log) = gated();
    let id = comparer
        .start(&Archives::default(), (folder(left), folder(right)), plain(), channel)
        .expect("started");
    wait_until("the first event", || !log.lock().expect("not poisoned").is_empty());

    (id, gate, log)
}

/// A folder of enough files that the scan sends rows before it is done, so it can be held in the middle of its walk.
fn crowd() -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().expect("folder");
    for number in 0..300 {
        fs::write(folder.path().join(format!("f{number:03}.txt")), b"x").expect("file");
    }
    let path = folder.path().to_string_lossy().into_owned();

    (folder, path)
}

fn ends_cancelled(gate: &Gate, log: &Log) {
    gate.release();
    wait_until("the comparison to end", || ended(log));

    assert!(matches!(
        log.lock().expect("not poisoned").last(),
        Some(CompareEvent::Done { cancelled: true, .. })
    ));
}

#[test]
fn a_scan_streams_its_rows_and_ends_with_one_done() {
    let (_left, left, _right, right) = pair();

    let (id, events) = scanned(&Comparer::default(), (folder(&left), folder(&right)), plain());

    assert_eq!(id, CompareId(1));
    let found = statuses(&events);
    assert_eq!(found[0], ("a.txt".to_owned(), CompareStatus::LeftOnly));
    assert_eq!(found[2], ("sub".to_owned(), CompareStatus::LeftOnly));
    assert_eq!(found.len(), 3);
    let done: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, CompareEvent::Done { .. }))
        .collect();
    assert_eq!(
        done,
        [&CompareEvent::Done {
            rows: 3,
            skipped: 0,
            truncated: false,
            cancelled: false
        }]
    );
    assert!(matches!(events.last(), Some(CompareEvent::Done { .. })));
}

#[test]
fn what_cannot_be_compared_is_refused_before_anything_starts() {
    let (_left, left, _right, right) = pair();
    let comparer = Comparer::default();
    let start = |sides: (CompareRoot, CompareRoot), options: CompareOptions| {
        comparer.start(&Archives::default(), sides, options, Channel::new(|_| Ok(())))
    };
    let tar = archive(Path::new(&left).join("x.tar").as_path(), "");

    let relative = start((folder("relative/dir"), folder(&right)), plain());
    let both = start((tar.clone(), tar.clone()), plain());
    let content = start(
        (tar, folder(&right)),
        CompareOptions {
            content: true,
            ..plain()
        },
    );

    for refused in [relative, both, content] {
        assert_eq!(refused.expect_err("refused").code(), IpcErrorCode::InvalidInput);
    }
    assert!(comparer.lock().is_empty(), "nothing was kept");
}

#[test]
fn folders_that_cannot_be_compared_are_reported_as_the_scan_s_error_without_a_path() {
    let (left, left_path, _right, _) = pair();
    let inside = left.path().join("sub").to_string_lossy().into_owned();
    let file = left.path().join("a.txt").to_string_lossy().into_owned();

    let same = scanned(&Comparer::default(), (folder(&left_path), folder(&left_path)), plain()).1;
    let nested = scanned(&Comparer::default(), (folder(&left_path), folder(&inside)), plain()).1;
    let not_a_folder = scanned(&Comparer::default(), (folder(&file), folder(&inside)), plain()).1;
    let missing = scanned(&Comparer::default(), (folder(&elsewhere()), folder(&inside)), plain()).1;

    for events in [&same, &nested, &not_a_folder] {
        let [CompareEvent::Error { error }] = events.as_slice() else {
            panic!("one error and nothing else: {events:?}");
        };
        assert_eq!(error.code, IpcErrorCode::InvalidInput);
        assert!(!error.message.contains(&left_path) && !error.message.contains("a.txt"));
    }
    assert!(matches!(missing.as_slice(), [CompareEvent::Error { error }] if error.code == IpcErrorCode::NotFound));
}

#[test]
fn a_third_scan_at_once_is_busy_until_one_ends() {
    let (_folder, root) = crowd();
    let (_other, other) = crowd();
    let comparer = Comparer::default();
    let (_, first_gate, first_log) = held(&comparer, &root, &other);
    let (_, second_gate, _) = held(&comparer, &root, &other);

    let error = comparer
        .start(
            &Archives::default(),
            (folder(&root), folder(&other)),
            plain(),
            Channel::new(|_| Ok(())),
        )
        .expect_err("busy");
    assert_eq!(error.code(), IpcErrorCode::Busy);
    assert_eq!(comparer.lock().len(), 2, "the refused scan is not kept");

    first_gate.release();
    wait_until("the first scan to end", || ended(&first_log));
    wait_until("a place to be free", || {
        comparer
            .start(
                &Archives::default(),
                (folder(&root), folder(&other)),
                plain(),
                Channel::new(|_| Ok(())),
            )
            .is_ok()
    });

    second_gate.release();
}

#[test]
fn cancelling_ends_a_scan_with_a_cancelled_done() {
    let (_folder, left) = crowd();
    let (_other, right) = pair_right();
    let comparer = Comparer::default();
    let (id, gate, log) = held(&comparer, &left, &right);

    comparer.cancel(id).expect("known");
    ends_cancelled(&gate, &log);

    assert!(
        comparer.get(id).is_ok(),
        "a cancelled scan is still kept until it is forgotten"
    );
}

fn pair_right() -> (tempfile::TempDir, String) {
    folder_with(&[("other.txt", b"x")])
}

#[test]
fn an_unknown_scan_is_not_found_and_forgetting_stops_and_drops_one() {
    let (_folder, left) = crowd();
    let (_other, right) = pair_right();
    let comparer = Comparer::default();

    assert_eq!(
        comparer.cancel(CompareId(9)).expect_err("unknown").code(),
        IpcErrorCode::NotFound
    );
    assert_eq!(
        comparer.forget(CompareId(9)).expect_err("unknown").code(),
        IpcErrorCode::NotFound
    );

    let (id, gate, log) = held(&comparer, &left, &right);

    comparer.forget(id).expect("known");
    ends_cancelled(&gate, &log);

    assert_eq!(comparer.get(id).expect_err("gone").code(), IpcErrorCode::NotFound);
}

#[test]
fn the_oldest_scan_that_has_ended_is_dropped_to_keep_four_and_one_that_runs_never_is() {
    let comparer = Comparer::default();
    let scan = |ended: bool| {
        let scan = Arc::new(Scan::default());
        scan.finished.store(ended, Ordering::Release);

        scan
    };

    // 1 runs, 2 and 3 have ended, 4 runs.
    for (id, ended) in [(1, false), (2, true), (3, true), (4, false)] {
        comparer.store(CompareId(id), scan(ended)).expect("room");
    }

    comparer.store(CompareId(5), scan(false)).expect("room by dropping 2");
    assert!(comparer.get(CompareId(2)).is_err(), "the oldest ended scan went");
    for kept in [1, 3, 4, 5] {
        assert!(comparer.get(CompareId(kept)).is_ok(), "{kept}");
    }

    // 3 is the only one that has ended; dropping it leaves four that run.
    comparer.store(CompareId(6), scan(false)).expect("room by dropping 3");
    let error = comparer.store(CompareId(7), scan(false)).expect_err("all run");
    assert_eq!(error.code(), IpcErrorCode::Busy);
    assert_eq!(comparer.lock().len(), STORED);
}

#[test]
fn a_scan_whose_channel_is_gone_stops() {
    let (_left, left, _right, right) = pair();
    let comparer = Comparer::default();
    let gone = Channel::new(|_| Err(tauri::Error::Io(std::io::Error::other("closed"))));

    let id = comparer
        .start(&Archives::default(), (folder(&left), folder(&right)), plain(), gone)
        .expect("started");

    let scan = comparer.get(id).expect("kept");
    wait_until("the scan to end", || scan.finished());
    assert!(scan.cancel.load(Ordering::Relaxed));
}

#[test]
fn shutting_down_stops_every_scan() {
    let (_folder, left) = crowd();
    let (_other, right) = pair_right();
    let comparer = Comparer::default();
    let (_, gate, log) = held(&comparer, &left, &right);

    comparer.shut_down();
    ends_cancelled(&gate, &log);
}

#[test]
fn an_archive_is_one_side_of_a_scan_by_its_index() {
    let (folder_side, path) = folder_with(&[("a.txt", b"alpha")]);
    let bytes = tar_of(&[
        item("a.txt", b"alpha"),
        item("docs/", b""),
        item("docs/b.txt", b"beta"),
        item("../evil.txt", b"x"),
    ]);
    fs::write(folder_side.path().join("pack.tar"), &bytes).expect("archive");
    let tar = archive(&folder_side.path().join("pack.tar"), "");
    // The folder holds the archive too, so only what the archive has too is the same.
    let (_other, right) = folder_with(&[("a.txt", b"alpha"), ("only-here.txt", b"x")]);

    let (_, events) = scanned(&Comparer::default(), (tar, folder(&right)), plain());

    let found = statuses(&events);
    assert_eq!(
        found.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
        ["a.txt", "docs", "only-here.txt"]
    );
    assert_eq!(found[1].1, CompareStatus::LeftOnly);
    assert_eq!(found[2].1, CompareStatus::RightOnly);
    // The entry whose name is refused is counted, never a row.
    assert!(
        events
            .iter()
            .any(|event| matches!(event, CompareEvent::Done { skipped: 1, .. }))
    );
    let docs = events
        .iter()
        .flat_map(|event| match event {
            CompareEvent::Rows { rows } => rows.clone(),
            _ => Vec::new(),
        })
        .find(|row| row.path == "docs")
        .expect("docs");
    let facts = docs.left.expect("left");
    assert_eq!((facts.kind, facts.files, facts.bytes), (EntryKind::Dir, 1, 4));
    assert!(!path.is_empty());
}

#[test]
fn an_archive_that_cannot_be_read_is_reported_as_the_scan_s_error_without_a_path() {
    let (folder_side, path) = folder_with(&[("broken.tar", b"this is not a tar")]);
    let tar = archive(&folder_side.path().join("broken.tar"), "");
    let (_other, right) = folder_with(&[("a.txt", b"alpha")]);

    let (_, events) = scanned(&Comparer::default(), (tar, folder(&right)), plain());

    let [CompareEvent::Error { error }] = events.as_slice() else {
        panic!("one error and nothing else: {events:?}");
    };
    assert!(
        !error.message.contains(&path) && !error.message.contains("broken.tar"),
        "{error:?}"
    );
}

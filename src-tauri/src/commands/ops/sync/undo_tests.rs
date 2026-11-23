//! Tests of `ops_sync_undo` through the slot of the sync that can be undone: a sync put back for real, the holding folders
//! that go when the undo is no longer offered, and what is refused.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pairpane_core::model::{IpcErrorCode, JobId, OpsEvent, Outcome};

use super::tests::{between, facts_of, to_right};
use crate::commands::compare::tests::folder;
use crate::commands::ops::tests::{received, running};
use crate::commands::testing::folder_with;

/// Waits for the job `id` to end, and gives how.
fn ended(events: &Arc<Mutex<Vec<OpsEvent>>>, id: JobId) -> Outcome {
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
        let outcome = received(events).into_iter().find_map(|event| match event {
            OpsEvent::Ended { id: done, outcome, .. } if done == id => Some(outcome),
            _ => None,
        });
        if let Some(outcome) = outcome {
            return outcome;
        }
        assert!(Instant::now() < deadline, "the job ended");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn holdings(dir: &Path) -> usize {
    fs::read_dir(dir)
        .expect("listed")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".pairpane-sync-"))
        .count()
}

#[test]
fn the_last_sync_is_put_back_once_and_its_holding_folder_goes() {
    let (left, left_path) = folder_with(&[("a.txt", b"newer"), ("b.txt", b"new")]);
    let (right, right_path) = folder_with(&[("a.txt", b"old")]);
    let (ops, _, events) = running();
    // The scan saw the right's file as it is.
    let items = vec![
        pairpane_core::model::SyncItem {
            target: Some(facts_of(&right.path().join("a.txt"))),
            ..to_right("a.txt", facts_of(&left.path().join("a.txt")))
        },
        to_right("b.txt", facts_of(&left.path().join("b.txt"))),
    ];

    let id = between(&ops, folder(&left_path), folder(&right_path), items).expect("accepted");
    assert_eq!(ended(&events, id), Outcome::Done);
    assert_eq!(fs::read(right.path().join("a.txt")).expect("replaced"), b"newer");
    assert_eq!(holdings(right.path()), 1);

    let undo = ops.sync_undo.undo(&ops.engine, id).expect("queued");
    assert_eq!(ended(&events, undo), Outcome::Done);

    assert_eq!(fs::read(right.path().join("a.txt")).expect("put back"), b"old");
    assert!(!right.path().join("b.txt").exists());
    assert_eq!(holdings(right.path()), 0);
    let again = ops.sync_undo.undo(&ops.engine, id).expect_err("undone already");
    assert_eq!(again.code(), IpcErrorCode::NotFound);
}

#[test]
fn a_newer_sync_takes_the_undo_and_the_older_one_s_kept_files_go() {
    let (left, left_path) = folder_with(&[("a.txt", b"newer")]);
    let (right, right_path) = folder_with(&[("a.txt", b"old")]);
    let (ops, _, events) = running();
    let replacing = pairpane_core::model::SyncItem {
        target: Some(facts_of(&right.path().join("a.txt"))),
        ..to_right("a.txt", facts_of(&left.path().join("a.txt")))
    };

    let first = between(&ops, folder(&left_path), folder(&right_path), vec![replacing]).expect("accepted");
    assert_eq!(ended(&events, first), Outcome::Done);
    assert_eq!(holdings(right.path()), 1);
    fs::write(left.path().join("c.txt"), b"c").expect("written");
    let second = between(
        &ops,
        folder(&left_path),
        folder(&right_path),
        vec![to_right("c.txt", facts_of(&left.path().join("c.txt")))],
    )
    .expect("accepted");

    // Let go of on a thread of its own, away from the command.
    let deadline = Instant::now() + Duration::from_secs(10);
    while holdings(right.path()) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        holdings(right.path()),
        0,
        "the first sync's kept file went when the second was queued"
    );
    let refused = ops.sync_undo.undo(&ops.engine, first).expect_err("not the last sync");
    assert_eq!(refused.code(), IpcErrorCode::NotFound);
    assert_eq!(ended(&events, second), Outcome::Done);
    ops.sync_undo.shut_down();
    assert!(right.path().join("c.txt").exists());
}

//! Tests of `ops_sync`: what is refused before a job starts, and folders and an archive synchronized for real through the
//! engine.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use pairpane_core::model::{EntryKind, IpcErrorCode, OpKind, OpsEvent, Outcome, SyncFacts};
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::archives::fixtures::{archive_in_folder, encrypted, item, tar_of, zip_of};
use crate::commands::compare::tests::{archive, folder};
use crate::commands::ops::tests::{received, running, wait_for_end};
use crate::commands::testing::folder_with;

/// The `mtime` the archives of the fixtures give their entries, in milliseconds (`14000000000` octal seconds).
const ENTRY_TIME: i64 = 1_610_612_736_000;

fn facts_of(path: &Path) -> SyncFacts {
    let meta = fs::metadata(path).expect("metadata");
    let modified = meta
        .modified()
        .expect("modified")
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch");

    SyncFacts {
        kind: EntryKind::File,
        size: meta.len(),
        modified: Some(i64::try_from(modified.as_millis()).expect("millis")),
    }
}

/// A file of an archive of the fixtures.
fn entry_of(size: u64) -> SyncFacts {
    SyncFacts {
        kind: EntryKind::File,
        size,
        modified: Some(ENTRY_TIME),
    }
}

fn to_right(path: &str, source: SyncFacts) -> SyncItem {
    SyncItem {
        path: path.to_owned(),
        direction: SyncDirection::ToRight,
        source,
        target: None,
    }
}

/// Asks `ops` to synchronize `items` between the two sides.
fn between(ops: &Ops, left: CompareRoot, right: CompareRoot, items: Vec<SyncItem>) -> CommandResult<JobId> {
    block_on(ops.sync(&Archives::default(), (left, right), items))
}

fn refused_as_invalid(refused: CommandResult<JobId>) {
    assert_eq!(refused.expect_err("refused").code(), IpcErrorCode::InvalidInput);
}

fn was_added_as_sync(events: &std::sync::Arc<std::sync::Mutex<Vec<OpsEvent>>>, id: JobId) -> bool {
    received(events)
        .iter()
        .any(|event| matches!(event, OpsEvent::Added { job } if job.id == id && job.kind == OpKind::Sync))
}

#[test]
fn two_folders_are_synchronized_by_a_sync_job() {
    let (left, left_path) = folder_with(&[("a.txt", b"alpha")]);
    let (right, right_path) = folder_with(&[]);
    let (ops, _, events) = running();
    let items = vec![to_right("a.txt", facts_of(&left.path().join("a.txt")))];

    let id = between(&ops, folder(&left_path), folder(&right_path), items).expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert!(was_added_as_sync(&events, id));
    assert_eq!(fs::read(right.path().join("a.txt")).expect("copied"), b"alpha");
}

#[test]
fn an_item_that_changed_since_the_scan_is_skipped_by_the_job() {
    let (left, left_path) = folder_with(&[("a.txt", b"alpha")]);
    let (right, right_path) = folder_with(&[]);
    let (ops, _, events) = running();
    let mut stale = facts_of(&left.path().join("a.txt"));
    stale.size += 1;

    between(
        &ops,
        folder(&left_path),
        folder(&right_path),
        vec![to_right("a.txt", stale)],
    )
    .expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert!(!right.path().join("a.txt").exists());
    let skipped = received(&events).into_iter().find_map(|event| match event {
        OpsEvent::Ended { skipped, .. } => Some(skipped),
        _ => None,
    });
    assert_eq!(skipped, Some(1));
}

#[test]
fn an_archive_is_copied_from_on_either_side() {
    let bytes = tar_of(&[item("docs/", b""), item("docs/guide.txt", b"guide text")]);
    let (_folder, archive_path) = archive_in_folder("pack.tar", &bytes);
    let tar = || archive(Path::new(&archive_path), "");
    let (target, target_path) = folder_with(&[]);
    let (ops, _, events) = running();

    let id = between(
        &ops,
        tar(),
        folder(&target_path),
        vec![to_right("docs/guide.txt", entry_of(10))],
    )
    .expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert!(was_added_as_sync(&events, id));
    assert_eq!(
        fs::read(target.path().join("docs").join("guide.txt")).expect("extracted"),
        b"guide text"
    );

    // The archive on the right, the items going left.
    let (again, again_path) = folder_with(&[]);
    let (ops, _, events) = running();
    let left_ward = SyncItem {
        direction: SyncDirection::ToLeft,
        ..to_right("docs/guide.txt", entry_of(10))
    };

    between(&ops, folder(&again_path), tar(), vec![left_ward]).expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert_eq!(
        fs::read(again.path().join("docs").join("guide.txt")).expect("extracted"),
        b"guide text"
    );
}

#[test]
fn an_archive_is_never_copied_into_and_two_archives_are_refused() {
    let bytes = tar_of(&[item("readme.txt", b"hello")]);
    let (_folder, archive_path) = archive_in_folder("pack.tar", &bytes);
    let tar = || archive(Path::new(&archive_path), "");
    let (source, source_path) = folder_with(&[("a.txt", b"alpha")]);
    let (ops, _, _events) = running();
    let item = to_right("a.txt", facts_of(&source.path().join("a.txt")));

    refused_as_invalid(between(&ops, folder(&source_path), tar(), vec![item.clone()]));
    refused_as_invalid(between(&ops, tar(), tar(), vec![item]));
}

#[test]
fn paths_that_climb_or_nest_and_plans_that_do_not_match_are_refused() {
    let (source, source_path) = folder_with(&[("a.txt", b"alpha")]);
    let (_target, target_path) = folder_with(&[]);
    let (ops, _, _events) = running();
    let file = facts_of(&source.path().join("a.txt"));
    let try_sync = |items: Vec<SyncItem>| between(&ops, folder(&source_path), folder(&target_path), items);

    for bad in ["../a.txt", "/etc/passwd", "a/../b", ""] {
        refused_as_invalid(try_sync(vec![to_right(bad, file)]));
    }
    refused_as_invalid(try_sync(vec![to_right("a", file), to_right("a/b", file)]));
    refused_as_invalid(try_sync(vec![to_right("a.txt", file), to_right("a.txt", file)]));
    refused_as_invalid(try_sync(Vec::new()));
    refused_as_invalid(between(
        &ops,
        folder("relative"),
        folder(&target_path),
        vec![to_right("a.txt", file)],
    ));
}

#[test]
fn encrypted_entries_without_a_password_fail_before_the_job_starts() {
    let bytes = zip_of(&[encrypted("secret.txt", b"hidden")]);
    let (_folder, archive_path) = archive_in_folder("pack.zip", &bytes);
    let (_target, target_path) = folder_with(&[]);
    let (ops, _, events) = running();

    let refused = between(
        &ops,
        archive(Path::new(&archive_path), ""),
        folder(&target_path),
        vec![to_right("secret.txt", entry_of(6))],
    );

    assert_eq!(refused.expect_err("refused").code(), IpcErrorCode::PasswordRequired);
    assert!(
        received(&events)
            .iter()
            .all(|event| !matches!(event, OpsEvent::Added { .. }))
    );
}

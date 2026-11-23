//! The undo of a synchronization: what it made is removed and what it replaced is put back, only while each is still what
//! the job left, and the holding folders go when the undo is over.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{Outcome, SyncDirection};

use super::sync_tests::{Pair, file, is_done, plan};
use super::testing::{Ended, Rig, read, task, tree};
use super::{Refused, Request, SyncLog, SyncPlan, SyncRoot, Task};

use SyncDirection::ToRight;

/// The synchronization of `rels` to the right, as the scan sees the two folders now, writing to `log`.
fn sync(pair: &Pair, rels: &[&str], log: &SyncLog) -> Ended {
    let plans: Vec<SyncPlan> = rels
        .iter()
        .map(|rel| plan(&pair.left(), &pair.right(), rel, ToRight))
        .collect();

    Rig::new().run(task(
        &pair.left(),
        rels,
        &pair.right(),
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Folder,
            plans,
            removals: Vec::new(),
            permanent: false,
            log: log.clone(),
        },
    ))
}

/// The undo of the synchronization `log` describes.
fn undo_of(pair: &Pair, log: &SyncLog) -> Request {
    task(
        &pair.right(),
        &log.names().iter().map(String::as_str).collect::<Vec<_>>(),
        &pair.right(),
        Task::SyncUndo { log: log.clone() },
    )
}

/// The right folder's tree without the holding folders.
fn right_tree(pair: &Pair) -> Vec<String> {
    tree(&pair.right())
        .into_iter()
        .filter(|path| !path.starts_with(".pairpane-sync-"))
        .collect()
}

fn holdings(dir: &Path) -> usize {
    fs::read_dir(dir)
        .expect("listed")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".pairpane-sync-"))
        .count()
}

/// A pair where the sync replaces `over.txt`, makes `new.txt`, the folder `fresh` (whole) and `deep/er/in.txt` (with
/// its folders).
fn scene() -> Pair {
    let pair = Pair::new();
    file(&pair.left().join("over.txt"), b"newer", 300);
    file(&pair.right().join("over.txt"), b"older", 200);
    file(&pair.left().join("new.txt"), b"new", 100);
    file(&pair.left().join("fresh/a.txt"), b"a", 100);
    file(&pair.left().join("deep/er/in.txt"), b"in", 100);
    file(&pair.right().join("keep.txt"), b"keep", 100);

    pair
}

const ITEMS: [&str; 4] = ["over.txt", "new.txt", "fresh", "deep/er/in.txt"];

#[test]
fn an_undo_puts_back_what_was_replaced_and_removes_what_was_made() {
    let pair = scene();
    let before = right_tree(&pair);
    let log = SyncLog::default();

    is_done(&sync(&pair, &ITEMS, &log), 0);
    assert_eq!(read(&pair.right().join("over.txt")), b"newer");
    assert_eq!(holdings(&pair.right()), 1, "the old file is kept aside");
    assert_eq!(log.undoable(), 4);

    let ended = Rig::new().run(undo_of(&pair, &log));

    is_done(&ended, 0);
    assert_eq!(right_tree(&pair), before);
    assert_eq!(read(&pair.right().join("over.txt")), b"older");
    assert_eq!(holdings(&pair.right()), 0, "the holding folder goes with the undo");
}

#[test]
fn what_changed_since_the_sync_is_left_and_counted() {
    let pair = scene();
    let log = SyncLog::default();
    is_done(&sync(&pair, &ITEMS, &log), 0);
    // Edited, edited, grown and gone.
    file(&pair.right().join("over.txt"), b"mine now", 400);
    file(&pair.right().join("new.txt"), b"mine too", 400);
    file(&pair.right().join("fresh/b.txt"), b"b", 100);
    fs::remove_file(pair.right().join("deep/er/in.txt")).expect("removed");

    let ended = Rig::new().run(undo_of(&pair, &log));

    is_done(&ended, 4);
    assert_eq!(read(&pair.right().join("over.txt")), b"mine now");
    assert_eq!(read(&pair.right().join("new.txt")), b"mine too");
    assert_eq!(read(&pair.right().join("fresh/b.txt")), b"b");
    assert!(
        !pair.right().join("deep").exists(),
        "the folders it made are gone once empty"
    );
    assert_eq!(holdings(&pair.right()), 0);
}

#[test]
fn a_sync_that_is_not_over_or_names_that_do_not_match_are_refused() {
    let pair = scene();
    let log = SyncLog::default();
    let engine = Rig::new().engine;

    assert_eq!(
        engine
            .submit(task(
                &pair.right(),
                &["x"],
                &pair.right(),
                Task::SyncUndo { log: log.clone() }
            ))
            .unwrap_err(),
        Refused::InvalidTask
    );

    is_done(&sync(&pair, &ITEMS, &log), 0);
    assert_eq!(
        engine
            .submit(task(&pair.right(), &["x"], &pair.right(), Task::SyncUndo { log }))
            .unwrap_err(),
        Refused::InvalidTask
    );
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_since_the_sync_leads_the_undo_nowhere() {
    let pair = scene();
    let log = SyncLog::default();
    is_done(&sync(&pair, &ITEMS, &log), 0);
    let elsewhere = pair.root.path().join("elsewhere");
    file(&elsewhere.join("er/in.txt"), b"in", 100);
    fs::rename(pair.right().join("deep"), pair.root.path().join("deep.old")).expect("moved aside");
    std::os::unix::fs::symlink(&elsewhere, pair.right().join("deep")).expect("link");

    let ended = Rig::new().run(undo_of(&pair, &log));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 1);
    assert_eq!(read(&elsewhere.join("er/in.txt")), b"in");
}

#[test]
fn an_undo_that_did_not_go_through_keeps_the_old_files_until_the_log_is_released() {
    let pair = scene();
    let log = SyncLog::default();
    is_done(&sync(&pair, &ITEMS, &log), 0);

    // An undo that was cancelled (or never ran) leaves them, so it can be asked for again.
    log.begin_undo();
    log.end_undo(false);
    assert_eq!(holdings(&pair.right()), 1);
    assert!(!log.undone());

    // Released while an undo runs: they go once it is over.
    log.begin_undo();
    log.release();
    assert_eq!(holdings(&pair.right()), 1);
    log.end_undo(false);
    assert_eq!(holdings(&pair.right()), 0);
}

#[test]
fn a_log_released_while_its_sync_runs_drops_its_holdings_when_the_sync_ends() {
    let pair = scene();
    let log = SyncLog::default();
    log.release();

    is_done(&sync(&pair, &ITEMS, &log), 0);

    assert_eq!(holdings(&pair.right()), 0);
    assert_eq!(read(&pair.right().join("over.txt")), b"newer");
}

/// Only the change time tells (Unix): on Windows such a file is left, the safe side.
#[cfg(unix)]
#[test]
fn a_created_file_that_says_it_is_from_the_future_is_still_undone() {
    let pair = Pair::new();
    file(&pair.left().join("future.txt"), b"later", 10_000_000_000);
    let log = SyncLog::default();
    is_done(&sync(&pair, &["future.txt"], &log), 0);

    let ended = Rig::new().run(undo_of(&pair, &log));

    is_done(&ended, 0);
    assert!(!pair.right().join("future.txt").exists());
}

/// The holding folder swapped for another folder of the user's after the sync: the undo puts nothing of it back, and
/// letting go of the log removes nothing of it.
#[cfg(unix)]
#[test]
fn a_folder_put_at_the_holding_folder_s_name_is_never_used_or_removed() {
    let pair = scene();
    let log = SyncLog::default();
    is_done(&sync(&pair, &ITEMS, &log), 0);
    let holding = log.holdings().pop().expect("a holding folder");
    fs::rename(&holding, pair.root.path().join("moved-away")).expect("moved away");
    file(&holding.join("1"), b"not the old file", 100);
    file(&holding.join("work.txt"), b"the user's work", 100);

    let ended = Rig::new().run(undo_of(&pair, &log));
    log.release();

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        ended.errors.len(),
        1,
        "the replaced file is not put back: {:?}",
        ended.errors
    );
    assert_eq!(read(&pair.right().join("over.txt")), b"newer");
    assert_eq!(read(&holding.join("1")), b"not the old file");
    assert_eq!(read(&holding.join("work.txt")), b"the user's work");
}

/// A folder that appeared where an archive's folder was to be extracted is not the job's: the undo leaves it.
#[test]
fn a_folder_that_was_not_extracted_is_never_removed_by_the_undo() {
    use super::extract_tests::{fixture, scene as archive_scene};
    use crate::archive::testing::MOMENT;
    use crate::model::{EntryKind, SyncFacts};

    let root = archive_scene("x.tar", &fixture());
    file(&root.path().join("dst/docs/mine.txt"), b"mine", 100);
    let entry = |kind, size| SyncFacts {
        kind,
        size,
        modified: if kind == EntryKind::Dir { None } else { Some(MOMENT) },
    };
    let log = SyncLog::default();
    let ended = Rig::new().run(task(
        &root.path().join("src"),
        &["a.txt", "docs"],
        &root.path().join("dst"),
        Task::Sync {
            left: SyncRoot::Archive {
                name: "x.tar".to_owned(),
                inner: String::new(),
                source: super::ArchiveJob::default(),
            },
            right: SyncRoot::Folder,
            plans: vec![
                SyncPlan {
                    direction: ToRight,
                    source: entry(EntryKind::File, 5),
                    target: None,
                },
                SyncPlan {
                    direction: ToRight,
                    source: entry(EntryKind::Dir, 0),
                    target: None,
                },
            ],
            removals: Vec::new(),
            permanent: false,
            log: log.clone(),
        },
    ));
    is_done(&ended, 1);
    assert_eq!(log.names(), ["a.txt"]);

    let undo = task(
        &root.path().join("dst"),
        &["a.txt"],
        &root.path().join("dst"),
        Task::SyncUndo { log },
    );
    is_done(&Rig::new().run(undo), 0);

    assert!(!root.path().join("dst/a.txt").exists());
    assert_eq!(read(&root.path().join("dst/docs/mine.txt")), b"mine");
}

//! The multi-rename through the commands: what `ops_task` refuses, the request it builds, and the undo slot (the token, a
//! rename that still runs, one that renamed nothing, and a rename that replaced it).

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pairpane_core::model::{IpcErrorCode, JobId, JobTask, OpKind, OpsEvent, Outcome};
use pairpane_core::ops::{Request, Task};

use super::tasks::task_request;
use super::tests::{received, running};
use super::*;
use crate::commands::testing::{names, root};

fn renaming(targets: &[&str]) -> JobTask {
    JobTask::Rename {
        targets: names(targets),
    }
}

fn code(result: CommandResult<Request>) -> IpcErrorCode {
    result.expect_err("refused").code()
}

/// How the job `id` ended, and how many items it skipped; waited for.
fn ended(events: &Arc<Mutex<Vec<OpsEvent>>>, id: JobId) -> (Outcome, u32) {
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
        let found = received(events).into_iter().find_map(|event| match event {
            OpsEvent::Ended {
                id: ended,
                outcome,
                skipped,
                ..
            } if ended == id => Some((outcome, skipped)),
            _ => None,
        });
        if let Some(found) = found {
            return found;
        }
        assert!(Instant::now() < deadline, "the job ended");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn files(folder: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(folder)
        .expect("folder")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();

    found
}

fn path_of(folder: &tempfile::TempDir) -> String {
    folder.path().to_string_lossy().into_owned()
}

fn rename(ops: &Ops, folder: &tempfile::TempDir, items: &[&str], targets: &[&str]) -> CommandResult<JobId> {
    ops.task(path_of(folder), names(items), renaming(targets))
}

#[test]
fn a_rename_request_works_in_place_and_keeps_its_items_and_new_names_in_order() {
    let request = task_request(root(), names(&["a", "b"]), renaming(&["x", "y"])).expect("valid request");

    assert_eq!(request.kind, OpKind::Rename);
    assert_eq!(request.to, request.from, "in place");
    assert_eq!(request.names, names(&["a", "b"]));
    let Task::Rename { targets, undo, .. } = request.task else {
        panic!("a rename");
    };
    assert_eq!((targets, undo), (names(&["x", "y"]), false));
}

#[test]
fn new_names_that_do_not_line_up_with_the_items_are_refused() {
    assert_eq!(
        code(task_request(root(), names(&["a", "b"]), renaming(&["x"]))),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(task_request(root(), names(&["a"]), renaming(&["x", "y"]))),
        IpcErrorCode::InvalidInput
    );
    // An item sent twice is dropped by the check of the names, and the new names would shift by one.
    assert_eq!(
        code(task_request(root(), names(&["a", "a", "b"]), renaming(&["x", "y"]))),
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        code(task_request(root(), names(&["../a"]), renaming(&["x"]))),
        IpcErrorCode::InvalidInput
    );
}

#[test]
fn the_engine_refuses_new_names_that_are_not_plain_equal_to_the_old_or_repeated() {
    let ops = Ops::default();
    let folder = tempfile::tempdir().expect("folder");

    for (items, targets) in [
        (&["a"][..], &["../x"][..]),
        (&["a"], &["x/y"]),
        (&["a"], &[""]),
        (&["a"], &["a"]),
        (&["a", "b"], &["x", "x"]),
    ] {
        let refused = rename(&ops, &folder, items, targets).expect_err("refused");

        assert_eq!(refused.code(), IpcErrorCode::InvalidInput, "{items:?} to {targets:?}");
    }
}

#[test]
fn a_rename_job_renames_and_its_undo_puts_the_names_back() {
    let (ops, folder, events) = running();
    fs::write(folder.path().join("a.txt"), "a").expect("file");
    fs::write(folder.path().join("b.txt"), "b").expect("file");

    let job = rename(&ops, &folder, &["a.txt", "b.txt"], &["b.txt", "a.txt"]).expect("accepted");
    assert_eq!(ended(&events, job).0, Outcome::Done);
    assert_eq!(fs::read_to_string(folder.path().join("a.txt")).expect("file"), "b");

    let undo = ops.rename_undo.undo(&ops.engine, job).expect("an undo job");
    let (outcome, skipped) = ended(&events, undo);

    assert_eq!((outcome, skipped), (Outcome::Done, 0));
    assert_eq!(fs::read_to_string(folder.path().join("a.txt")).expect("file"), "a");
    assert_eq!(files(folder.path()), ["a.txt", "b.txt"]);
}

#[test]
fn an_undo_skips_what_changed_since_and_says_how_many() {
    let (ops, folder, events) = running();
    fs::write(folder.path().join("a"), "a").expect("file");
    fs::write(folder.path().join("b"), "b").expect("file");
    let job = rename(&ops, &folder, &["a", "b"], &["x", "y"]).expect("accepted");
    ended(&events, job);
    // `x` is gone, and the old name of `y` is taken.
    fs::remove_file(folder.path().join("x")).expect("removed");
    fs::write(folder.path().join("b"), "new b").expect("file");

    let undo = ops.rename_undo.undo(&ops.engine, job).expect("an undo job");

    assert_eq!(ended(&events, undo), (Outcome::Done, 2));
    assert_eq!(files(folder.path()), ["b", "y"]);
    assert_eq!(fs::read_to_string(folder.path().join("b")).expect("file"), "new b");
}

#[test]
fn a_token_that_is_not_the_latest_rename_has_nothing_to_undo_and_a_later_batch_replaces_it() {
    let (ops, folder, events) = running();
    for name in ["a", "b"] {
        fs::write(folder.path().join(name), name).expect("file");
    }
    let first = rename(&ops, &folder, &["a"], &["a2"]).expect("accepted");
    ended(&events, first);
    let second = rename(&ops, &folder, &["b"], &["b2"]).expect("accepted");
    ended(&events, second);

    let stale = ops.rename_undo.undo(&ops.engine, first).expect_err("replaced");
    let unknown = ops.rename_undo.undo(&ops.engine, JobId(9_999)).expect_err("unknown");

    assert_eq!(
        (stale.code(), unknown.code()),
        (IpcErrorCode::NotFound, IpcErrorCode::NotFound)
    );
    assert!(
        ops.rename_undo.undo(&ops.engine, second).is_ok(),
        "the latest one can be undone"
    );
}

#[test]
fn an_undo_is_done_once_and_has_no_undo_of_its_own() {
    let (ops, folder, events) = running();
    fs::write(folder.path().join("a"), "a").expect("file");
    let job = rename(&ops, &folder, &["a"], &["b"]).expect("accepted");
    ended(&events, job);

    let undo = ops.rename_undo.undo(&ops.engine, job).expect("an undo job");
    ended(&events, undo);

    for token in [job, undo] {
        let refused = ops.rename_undo.undo(&ops.engine, token).expect_err("nothing to undo");
        assert_eq!(refused.code(), IpcErrorCode::NotFound);
    }
}

#[test]
fn a_rename_that_renamed_nothing_has_nothing_to_undo() {
    let (ops, folder, events) = running();

    let job = rename(&ops, &folder, &["gone"], &["x"]).expect("accepted");
    ended(&events, job);

    let refused = ops.rename_undo.undo(&ops.engine, job).expect_err("nothing renamed");
    assert_eq!(refused.code(), IpcErrorCode::NotFound);
}

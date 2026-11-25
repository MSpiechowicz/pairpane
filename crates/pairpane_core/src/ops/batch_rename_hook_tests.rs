//! Multi-rename when something happens between its steps: a cancel, a file that appears at a new name, a folder that is
//! swapped for another. The hook of the job (`after_aside`) runs just after an item was moved aside.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pretty_assertions::assert_eq;

use crate::model::{ControlAction, JobId, JobPhase, OpsEvent, Outcome};

use super::batch_rename_tests::renaming;
use super::partial::run_token;
use super::testing::{Rig, put, read, temp, tree};
use super::{Engine, RenameLog, hooks::Hooks};

fn folder_with(names: &[&str]) -> tempfile::TempDir {
    let root = temp();
    for name in names {
        put(root.path(), &format!("src/{name}"), name.as_bytes());
    }

    root
}

/// A rig whose job runs `hook` with the temporary path of the `n`th item moved aside, counting from 1.
fn rig_with(hook: impl Fn(usize, &Path) + Send + Sync + 'static) -> Rig {
    let calls = AtomicUsize::new(0);

    Rig::hooked(|hooks: &mut Hooks| {
        hooks.after_aside = Some(Arc::new(move |path| {
            hook(calls.fetch_add(1, Ordering::SeqCst) + 1, path)
        }));
    })
}

/// What the items `a` and `b` of `folder` hold.
fn contents(folder: &Path) -> (Vec<u8>, Vec<u8>) {
    (read(&folder.join("a")), read(&folder.join("b")))
}

/// Swaps the items `a` and `b` of `src` with `rig`, and checks that no item failed.
fn swap_without_errors(rig: &mut Rig, src: &Path) {
    let ended = rig.run(renaming(src, &[("a", "b"), ("b", "a")], false, &RenameLog::default()));

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
}

fn temporary_names(src: &Path) -> Vec<String> {
    tree(src)
        .into_iter()
        .filter(|name| name.starts_with(".pairpane-rename-"))
        .collect()
}

#[test]
fn a_cancel_between_the_steps_puts_the_items_that_are_aside_back() {
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    let (reached, reached_rx) = mpsc::channel::<()>();
    let (go, go_rx) = mpsc::channel::<()>();
    let (reached, go_rx) = (Mutex::new(reached), Mutex::new(go_rx));
    let mut rig = rig_with(move |_, _| {
        reached.lock().expect("lock").send(()).expect("the test listens");
        // Waits for the cancel; a timeout only means the test is over.
        let _ = go_rx.lock().expect("lock").recv_timeout(Duration::from_secs(20));
    });
    let log = RenameLog::default();
    let job = rig.start(renaming(&src, &[("a", "b"), ("b", "a")], false, &log));

    reached_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the first item is aside");
    rig.engine.control(job, ControlAction::Cancel).expect("known job");
    go.send(()).expect("the hook waits");
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert_eq!(tree(&src), ["a", "b"], "nothing is left under a temporary name");
    assert_eq!(
        (read(&src.join("a")), read(&src.join("b"))),
        (b"a".to_vec(), b"b".to_vec())
    );
    assert_eq!(log.done(), Vec::new());
    assert!(log.finished());
}

#[test]
fn a_file_at_a_new_name_after_the_items_are_aside_is_never_replaced_and_what_can_goes_back() {
    // `b` appears when both items are aside. The first one cannot go to it and returns to `a`, which makes the second one's
    // new name taken, and its own name is taken too, so it stays under its temporary name and says so.
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    let outside = src.clone();
    let mut rig = rig_with(move |count, _| {
        if count == 2 {
            fs::write(outside.join("b"), b"outside").expect("file");
        }
    });
    let log = RenameLog::default();

    let ended = rig.run(renaming(&src, &[("a", "b"), ("b", "a")], false, &log));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 2, "{:?}", ended.errors);
    let by_path = |path: &str| ended.errors.iter().find(|error| error.path == path).expect("an error");
    assert_eq!(by_path("a").error.message, "something with this name exists already");
    assert!(
        by_path("b")
            .error
            .message
            .starts_with("the item was left under the name .pairpane-rename-"),
        "{}",
        by_path("b").error.message
    );
    assert_eq!(read(&src.join("a")), b"a");
    assert_eq!(read(&src.join("b")), b"outside", "what was there is untouched");
    let aside = temporary_names(&src);
    assert_eq!(aside.len(), 1);
    assert_eq!(read(&src.join(&aside[0])), b"b");
    assert_eq!(log.done(), Vec::new(), "nothing was renamed");
}

#[test]
fn an_item_whose_step_fails_leaves_those_that_wait_for_its_name_where_they_were() {
    // `a` goes aside for `b`; `c` appears; `b` cannot go to it, so `b` stays, and `a` cannot take its name and returns.
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    let outside = src.clone();
    let mut rig = rig_with(move |_, _| {
        fs::write(outside.join("c"), b"outside").expect("file");
    });
    let log = RenameLog::default();

    let ended = rig.run(renaming(&src, &[("a", "b"), ("b", "c")], false, &log));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 2, "{:?}", ended.errors);
    assert!(
        ended
            .errors
            .iter()
            .all(|error| error.error.message == "something with this name exists already")
    );
    assert_eq!(tree(&src), ["a", "b", "c"]);
    assert_eq!(
        ["a", "b", "c"].map(|name| read(&src.join(name))),
        [b"a".to_vec(), b"b".to_vec(), b"outside".to_vec()]
    );
    assert_eq!(log.done(), Vec::new());
}

#[test]
fn a_temporary_name_that_is_taken_is_skipped_and_left_alone() {
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    // The first name the job (the first of this engine) would try.
    let taken = format!(".pairpane-rename-{}-1-1.tmp", run_token());
    put(root.path(), &format!("src/{taken}"), b"someone else's");
    let mut rig = Rig::new();

    swap_without_errors(&mut rig, &src);
    assert_eq!(temporary_names(&src), std::slice::from_ref(&taken));
    assert_eq!(read(&src.join(&taken)), b"someone else's");
    assert_eq!(contents(&src), (b"b".to_vec(), b"a".to_vec()));
}

#[cfg(unix)]
#[test]
fn the_renames_go_on_in_the_folder_that_was_opened_when_its_path_is_given_to_another() {
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    let moved = root.path().join("moved");
    let (from, to) = (src.clone(), moved.clone());
    let mut rig = rig_with(move |count, _| {
        if count == 1 {
            fs::rename(&from, &to).expect("the folder moves");
            put(from.parent().expect("a parent"), "src/a", b"decoy a");
            put(from.parent().expect("a parent"), "src/b", b"decoy b");
        }
    });

    swap_without_errors(&mut rig, &src);
    assert_eq!(
        contents(&moved),
        (b"b".to_vec(), b"a".to_vec()),
        "the folder that was opened is the one renamed"
    );
    assert_eq!(tree(&moved), ["a", "b"]);
    assert_eq!(
        contents(&src),
        (b"decoy a".to_vec(), b"decoy b".to_vec()),
        "what is at the path now is untouched"
    );
}

#[test]
fn the_log_is_finished_when_the_job_stops_unexpectedly() {
    let root = folder_with(&["a"]);
    let log = RenameLog::default();
    let mut rig = Rig::hooked(|hooks| hooks.panic_on_start = true);

    let ended = rig.run(renaming(&root.path().join("src"), &[("a", "b")], false, &log));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert!(log.finished());
}

#[test]
fn a_rename_cancelled_after_some_items_reports_exactly_those_in_its_last_progress() {
    let root = folder_with(&["a", "b", "c", "d"]);
    let src = root.path().join("src");
    // The hook cancels the job when the second item is over; the job is known once it was started and is held at the
    // start of its renaming until then.
    let target: Arc<Mutex<Option<(Engine, JobId)>>> = Arc::default();
    let items = AtomicUsize::new(0);
    let mut hooks = Hooks::default();
    let found = Arc::clone(&target);
    hooks.after_item = Some(Arc::new(move |_| {
        if items.fetch_add(1, Ordering::SeqCst) + 1 == 2
            && let Some((engine, job)) = &*found.lock().expect("lock")
        {
            engine.control(*job, ControlAction::Cancel).expect("known job");
        }
    }));
    let mut rig = Rig::with(hooks, |engine, event| {
        if let OpsEvent::Phase {
            id,
            phase: JobPhase::Renaming,
        } = event
        {
            engine.control(*id, ControlAction::Pause).expect("known job");
        }
    });
    let log = RenameLog::default();
    let job = rig.start(renaming(
        &src,
        &[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")],
        false,
        &log,
    ));

    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    *target.lock().expect("lock") = Some((rig.engine.clone(), job));
    rig.engine.control(job, ControlAction::Resume).expect("known job");
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert_eq!(tree(&src), ["1", "2", "c", "d"]);
    assert_eq!(log.done().len(), 2);
    let last = rig.seen.iter().rev().find_map(|event| match event {
        OpsEvent::Progress {
            id,
            done_files,
            total_files,
            ..
        } if *id == job => Some((*done_files, *total_files)),
        _ => None,
    });
    assert_eq!(
        last,
        Some((2, 4)),
        "the last progress before the end counts the items done"
    );
}

#[test]
fn an_item_aside_says_what_it_was_by_its_temporary_name() {
    // Were the app to stop here (a crash, the power going off), the item would stay under this name: it holds the old
    // name, so the item can still be told (#74).
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let names = Arc::clone(&seen);
    let mut rig = rig_with(move |_, path| {
        let name = path.file_name().expect("a name").to_string_lossy().into_owned();
        names.lock().expect("names").push(name);
    });

    swap_without_errors(&mut rig, &src);

    let seen = seen.lock().expect("names").clone();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|name| name.ends_with("-a") || name.ends_with("-b")),
        "{seen:?}"
    );
    assert_eq!(contents(&src), (b"b".to_vec(), b"a".to_vec()));
}

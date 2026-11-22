//! Multi-rename through jobs, on real files: swaps, cycles and chains, nothing replaced, the log of what was renamed, the
//! undo job, and what `submit` refuses.

use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{IpcErrorCode, JobPhase, OpsEvent, Outcome};

use super::testing::{Ended, Rig, phases, put, read, task, temp, tree};
use super::{Refused, RenameLog, Task, name_fits};

/// A rename of the items `pairs` (old name, new name) of `dir`, writing to `log`.
pub(super) fn renaming(dir: &Path, pairs: &[(&str, &str)], undo: bool, log: &RenameLog) -> super::Request {
    let names: Vec<&str> = pairs.iter().map(|pair| pair.0).collect();
    let targets = pairs.iter().map(|pair| pair.1.to_owned()).collect();

    task(
        dir,
        &names,
        dir,
        Task::Rename {
            targets,
            undo,
            log: log.clone(),
        },
    )
}

/// Files of the given names in a new folder `root/src`, each holding its own name.
fn folder_with(names: &[&str]) -> tempfile::TempDir {
    let root = temp();
    for name in names {
        put(root.path(), &format!("src/{name}"), name.as_bytes());
    }

    root
}

fn run_rename(src: &Path, pairs: &[(&str, &str)]) -> (Ended, RenameLog) {
    let log = RenameLog::default();
    let ended = Rig::new().run(renaming(src, pairs, false, &log));

    (ended, log)
}

/// The pairs the log holds, in a fixed order.
fn done(log: &RenameLog) -> Vec<(String, String)> {
    let mut done = log.done();
    done.sort();

    done
}

/// `pairs` as the log holds them.
pub(super) fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(from, to)| ((*from).to_owned(), (*to).to_owned()))
        .collect()
}

#[test]
fn items_are_renamed_and_the_log_holds_the_pairs() {
    let root = folder_with(&["a.txt", "b.txt", "keep"]);
    let src = root.path().join("src");

    let (ended, log) = run_rename(&src, &[("a.txt", "x.txt"), ("b.txt", "y.txt")]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&src), ["keep", "x.txt", "y.txt"]);
    assert_eq!(read(&src.join("x.txt")), b"a.txt");
    assert_eq!(done(&log), owned(&[("a.txt", "x.txt"), ("b.txt", "y.txt")]));
    assert!(log.finished());
}

#[test]
fn the_job_goes_through_the_renaming_phase_and_counts_each_item() {
    let root = folder_with(&["a", "b", "c"]);
    let src = root.path().join("src");
    let mut rig = Rig::new();
    let job = rig.start(renaming(
        &src,
        &[("a", "1"), ("b", "2"), ("c", "3")],
        false,
        &RenameLog::default(),
    ));

    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(phases(&rig).contains(&JobPhase::Renaming));
    let last = rig.seen.iter().rev().find_map(|event| match event {
        OpsEvent::Progress {
            done_files,
            total_files,
            ..
        } => Some((*done_files, *total_files)),
        _ => None,
    });
    assert_eq!(last, Some((3, 3)));
}

#[test]
fn two_items_swap_names_and_nothing_is_left_behind() {
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");

    let (ended, log) = run_rename(&src, &[("a", "b"), ("b", "a")]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&src), ["a", "b"]);
    assert_eq!(
        (read(&src.join("a")), read(&src.join("b"))),
        (b"b".to_vec(), b"a".to_vec())
    );
    assert_eq!(done(&log), owned(&[("a", "b"), ("b", "a")]));
}

#[test]
fn three_items_pass_their_names_around_a_cycle() {
    let root = folder_with(&["a", "b", "c"]);
    let src = root.path().join("src");

    let (ended, _) = run_rename(&src, &[("a", "b"), ("b", "c"), ("c", "a")]);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&src), ["a", "b", "c"]);
    let contents: Vec<_> = ["a", "b", "c"].iter().map(|name| read(&src.join(name))).collect();
    assert_eq!(contents, [b"c".to_vec(), b"a".to_vec(), b"b".to_vec()]);
}

#[test]
fn a_chain_shifts_the_names_along_whatever_the_order_of_the_items() {
    let root = folder_with(&["1", "2", "3"]);
    let src = root.path().join("src");

    let (ended, log) = run_rename(&src, &[("3", "4"), ("1", "2"), ("2", "3")]);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&src), ["2", "3", "4"]);
    assert_eq!(
        ["2", "3", "4"].map(|name| read(&src.join(name))),
        [b"1".to_vec(), b"2".to_vec(), b"3".to_vec()]
    );
    assert_eq!(log.done().len(), 3);
}

#[test]
fn a_change_of_case_only_is_done() {
    let root = folder_with(&["a.txt", "b.txt"]);
    let src = root.path().join("src");

    let (ended, _) = run_rename(&src, &[("a.txt", "A.TXT"), ("b.txt", "B.txt")]);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&src), ["A.TXT", "B.txt"]);
}

#[test]
fn a_new_name_that_something_has_is_never_replaced_and_the_item_stays() {
    let root = folder_with(&["a", "b", "taken"]);
    let src = root.path().join("src");

    let (ended, log) = run_rename(&src, &[("a", "taken"), ("b", "c")]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "a");
    assert_eq!(ended.errors[0].error.message, "something with this name exists already");
    assert_eq!(tree(&src), ["a", "c", "taken"]);
    assert_eq!(read(&src.join("taken")), b"taken");
    assert_eq!(done(&log), owned(&[("b", "c")]), "only what was done is logged");
}

#[test]
fn an_item_that_is_gone_is_reported_and_the_others_go_on() {
    let root = folder_with(&["a"]);
    let src = root.path().join("src");

    let (ended, log) = run_rename(&src, &[("gone", "x"), ("a", "b")]);

    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "gone");
    assert_eq!(ended.errors[0].error.code, IpcErrorCode::NotFound);
    assert_eq!(tree(&src), ["b"]);
    assert_eq!(done(&log), owned(&[("a", "b")]));
}

#[test]
fn a_folder_and_a_dangling_link_are_renamed_like_files() {
    let root = folder_with(&["a"]);
    let src = root.path().join("src");
    std::fs::create_dir(src.join("dir")).expect("folder");
    #[cfg(unix)]
    std::os::unix::fs::symlink("nowhere", src.join("link")).expect("link");
    let renames: &[(&str, &str)] = if cfg!(unix) {
        &[("dir", "d2"), ("link", "l2"), ("a", "b")]
    } else {
        &[("dir", "d2"), ("a", "b")]
    };

    let (ended, _) = run_rename(&src, renames);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    let expected: &[&str] = if cfg!(unix) { &["b", "d2/", "l2"] } else { &["b", "d2/"] };
    assert_eq!(tree(&src), expected);
}

#[test]
fn the_log_is_finished_also_when_the_job_cannot_start() {
    let root = temp();
    let missing = root.path().join("nothing");
    let log = RenameLog::default();

    let ended = Rig::new().run(renaming(&missing, &[("a", "b")], false, &log));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert!(log.finished());
    assert_eq!(log.done(), Vec::new());
}

#[test]
fn an_undo_job_restores_names_and_skips_what_changed_since() {
    let root = folder_with(&["x", "y"]);
    let src = root.path().join("src");
    // `a` was taken since, `z` is gone.
    put(root.path(), "src/a", b"someone else's");
    let log = RenameLog::expecting(&src);

    let ended = Rig::new().run(renaming(&src, &[("x", "a"), ("y", "b"), ("z", "c")], true, &log));

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "skipped, not failed: {:?}", ended.errors);
    assert_eq!(ended.skipped, 2);
    assert_eq!(tree(&src), ["a", "b", "x"]);
    assert_eq!(read(&src.join("a")), b"someone else's");
    assert_eq!(read(&src.join("b")), b"y");
    assert_eq!(done(&log), owned(&[("y", "b")]));
}

#[test]
fn undoing_a_swap_swaps_back() {
    let root = folder_with(&["a", "b"]);
    let src = root.path().join("src");
    let (_, log) = run_rename(&src, &[("a", "b"), ("b", "a")]);
    let reversed: Vec<(String, String)> = log.done().into_iter().map(|(from, to)| (to, from)).collect();
    let reversed: Vec<(&str, &str)> = reversed.iter().map(|(from, to)| (from.as_str(), to.as_str())).collect();

    let ended = Rig::new().run(renaming(&src, &reversed, true, &log.undoing()));

    assert_eq!((ended.skipped, ended.errors.len()), (0, 0));
    assert_eq!(
        (read(&src.join("a")), read(&src.join("b"))),
        (b"a".to_vec(), b"b".to_vec())
    );
}

#[test]
fn an_undo_without_the_identity_of_the_folder_it_belongs_to_is_refused() {
    let root = folder_with(&["y"]);
    let src = root.path().join("src");

    let ended = Rig::new().run(renaming(&src, &[("y", "b")], true, &RenameLog::default()));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(tree(&src), ["y"]);
}

#[cfg(unix)]
#[test]
fn an_undo_is_refused_in_a_folder_that_replaced_the_one_it_belongs_to() {
    let root = folder_with(&["x"]);
    let src = root.path().join("src");
    let (ended, log) = run_rename(&src, &[("x", "y")]);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    // The folder is moved away and a link to another folder, which holds an item `y` too, takes its place.
    let other = root.path().join("other");
    put(root.path(), "other/y", b"not ours");
    std::fs::rename(&src, root.path().join("moved")).expect("moved");
    std::os::unix::fs::symlink(&other, &src).expect("link");

    let ended = Rig::new().run(renaming(&src, &[("y", "x")], true, &log.undoing()));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(tree(&other), ["y"]);
    assert_eq!(read(&other.join("y")), b"not ours");
    assert_eq!(tree(&root.path().join("moved")), ["y"]);
}

#[test]
fn an_undo_in_the_same_folder_puts_the_names_back() {
    let root = folder_with(&["x"]);
    let src = root.path().join("src");
    let (_, log) = run_rename(&src, &[("x", "y")]);

    let ended = Rig::new().run(renaming(&src, &[("y", "x")], true, &log.undoing()));

    assert_eq!((ended.skipped, ended.errors.len()), (0, 0));
    assert_eq!(tree(&src), ["x"]);
}

/// What `submit` says to a rename of `pairs`.
fn submitted(pairs: &[(&str, &str)]) -> Result<(), Refused> {
    let root = folder_with(&[]);
    let rig = Rig::new();

    rig.engine
        .submit(renaming(root.path(), pairs, false, &RenameLog::default()))
        .map(|_| ())
}

#[test]
fn submit_refuses_new_names_that_do_not_fit_the_items() {
    assert_eq!(submitted(&[("a", "b")]), Ok(()));
    assert_eq!(submitted(&[("a", "a")]), Err(Refused::InvalidTask), "the same name");
    assert_eq!(
        submitted(&[("a", "x"), ("b", "x")]),
        Err(Refused::InvalidTask),
        "one new name twice"
    );
    for bad in ["", ".", "..", "x/y", "x\0y"] {
        assert_eq!(submitted(&[("a", bad)]), Err(Refused::InvalidTask), "{bad:?}");
    }
    assert_eq!(
        submitted(&[("a", "x"), ("a", "y")]),
        Err(Refused::InvalidNames),
        "one item twice"
    );
    assert_eq!(submitted(&[]), Err(Refused::InvalidNames), "no items");
}

#[test]
fn submit_refuses_a_name_that_is_too_long_and_names_that_are_too_many_bytes() {
    let long = "n".repeat(super::MAX_NAME_UNITS + 1);
    assert!(!name_fits(&long));

    assert_eq!(submitted(&[("a", &long)]), Err(Refused::NamesTooLarge));
}

#[test]
fn submit_refuses_targets_that_are_not_one_for_each_item() {
    let root = folder_with(&[]);
    let rig = Rig::new();
    let mut request = renaming(root.path(), &[("a", "b"), ("c", "d")], false, &RenameLog::default());
    if let Task::Rename { targets, .. } = &mut request.task {
        targets.pop();
    }

    assert_eq!(rig.engine.submit(request), Err(Refused::InvalidTask));
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn submit_refuses_new_names_that_differ_in_case_only_where_the_filesystem_ignores_case() {
    assert_eq!(submitted(&[("a", "X"), ("b", "x")]), Err(Refused::InvalidTask));
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn submit_takes_new_names_that_differ_in_case_only_where_the_filesystem_tells_them_apart() {
    assert_eq!(submitted(&[("a", "X"), ("b", "x")]), Ok(()));
}

//! A target that exists: the questions, the answers, "apply to all", and the rules around them.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::{Answer, OpsEvent, Outcome};

use super::MAX_NAME_UNITS;
use super::conflict::free_name;
use super::testing::{Rig, put, read, set_times, temp, tree};
use super::validate::{name_fits, name_units};

fn set_modified(path: &Path, seconds: u64) {
    set_times(path, SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
}

fn questions_asked(rig: &Rig) -> usize {
    rig.seen
        .iter()
        .filter(|event| matches!(event, OpsEvent::Conflict { .. }))
        .count()
}

/// A folder where each of `names` is "new" in `src` and "old" in `dst`.
fn clashing(names: &[&str]) -> TempDir {
    let root = temp();

    for name in names {
        put(root.path(), &format!("src/{name}"), b"new");
        put(root.path(), &format!("dst/{name}"), b"old");
    }

    root
}

#[test]
fn free_names_count_up_from_two() {
    let root = temp();
    for name in ["a.txt", "noext", ".hidden"] {
        put(root.path(), name, b"1");
    }
    let free = |name: &str| free_name(&root.path().join(name)).expect("a name");

    assert_eq!(free("a.txt"), root.path().join("a (2).txt"));
    put(root.path(), "a (2).txt", b"2");
    assert_eq!(free("a.txt"), root.path().join("a (3).txt"));
    assert_eq!(free("noext"), root.path().join("noext (2)"));
    assert_eq!(free(".hidden"), root.path().join(".hidden (2)"));
}

#[test]
fn a_free_name_stays_within_the_length_of_a_file_name_by_cutting_the_stem_by_whole_chars() {
    let root = temp();
    // As long as a name may be, in chars that take two bytes and a last one that takes one, so a cut by bytes
    // would fall inside a char.
    let mut stem = String::new();
    while name_units(&format!("{stem}éa.txt")) <= MAX_NAME_UNITS {
        stem.push('é');
    }
    stem.push('a');
    let long = format!("{stem}.txt");
    assert!(name_fits(&long), "{long}");

    let free = free_name(&root.path().join(&long)).expect("a name");
    let name = free.file_name().expect("a name").to_str().expect("text");

    assert!(name_fits(name), "{} units", name_units(name));
    assert!(name.starts_with('é') && name.ends_with(" (2).txt"), "{name}");

    // One that cannot be made to fit at all is refused.
    let no_room = format!("a.{}", "x".repeat(MAX_NAME_UNITS - 2));
    assert!(free_name(&root.path().join(no_room)).is_err());
}

#[test]
fn the_question_says_what_both_sides_are_and_suggests_a_name() {
    let root = clashing(&["a.txt"]);
    put(root.path(), "src/a.txt", b"new content");
    set_modified(&root.path().join("src/a.txt"), 2_000_000_000);
    set_modified(&root.path().join("dst/a.txt"), 1_000_000_000);
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["a.txt"]);
    let question = rig.wait_conflict(job);

    assert_eq!(
        (question.job, question.question, question.name.as_str()),
        (job, 1, "a.txt")
    );
    assert_eq!(question.suggested, "a (2).txt");
    assert_eq!(
        (question.source.size, question.source.modified, question.source.dir),
        (11, Some(2_000_000_000_000), false)
    );
    assert_eq!(
        (question.target.size, question.target.modified, question.target.dir),
        (3, Some(1_000_000_000_000), false)
    );
    rig.engine.cancel_all();
    rig.wait_ended(job);
}

#[test]
fn skip_leaves_the_target_and_a_moved_source_where_they_are() {
    let root = clashing(&["a.txt"]);
    let mut rig = Rig::new();

    let job = rig.move_in(root.path(), &["a.txt"]);
    rig.answer_next(job, Answer::Skip, false);
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 1);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"old");
    assert_eq!(read(&root.path().join("src/a.txt")), b"new");
}

#[test]
fn overwrite_replaces_the_target() {
    let root = clashing(&["a.txt"]);
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["a.txt"]);
    rig.answer_next(job, Answer::Overwrite, false);
    rig.wait_ended(job);

    assert_eq!(read(&root.path().join("dst/a.txt")), b"new");
    assert_eq!(tree(&root.path().join("dst")), ["a.txt"]);
}

#[test]
fn a_moved_file_overwrites_by_renaming() {
    let root = clashing(&["a.txt"]);
    let mut rig = Rig::new();

    let job = rig.move_in(root.path(), &["a.txt"]);
    rig.answer_next(job, Answer::Overwrite, false);
    let ended = rig.wait_ended(job);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(root.path()), ["dst/", "dst/a.txt", "src/"]);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"new");
}

#[test]
fn rename_copies_under_the_suggested_name_and_keeps_the_original() {
    let root = clashing(&["a.txt"]);
    put(root.path(), "dst/a (2).txt", b"taken");
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["a.txt"]);
    let question = rig.answer_next(job, Answer::Rename, false);
    rig.wait_ended(job);

    assert_eq!(question.suggested, "a (3).txt");
    assert_eq!(read(&root.path().join("dst/a.txt")), b"old");
    assert_eq!(read(&root.path().join("dst/a (2).txt")), b"taken");
    assert_eq!(read(&root.path().join("dst/a (3).txt")), b"new");
}

#[test]
fn overwrite_older_replaces_an_older_target_and_keeps_a_newer_one() {
    let root = clashing(&["older.txt", "newer.txt"]);
    for name in ["older.txt", "newer.txt"] {
        set_modified(&root.path().join("src").join(name), 1_500_000_000);
    }
    set_modified(&root.path().join("dst/older.txt"), 1_000_000_000);
    set_modified(&root.path().join("dst/newer.txt"), 2_000_000_000);
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["older.txt", "newer.txt"]);
    rig.answer_next(job, Answer::OverwriteOlder, true);
    rig.wait_ended(job);

    assert_eq!(read(&root.path().join("dst/older.txt")), b"new");
    assert_eq!(read(&root.path().join("dst/newer.txt")), b"old");
}

#[test]
fn apply_to_all_asks_only_once() {
    let root = clashing(&["a", "b", "c"]);
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["a", "b", "c"]);
    rig.answer_next(job, Answer::Overwrite, true);
    rig.wait_ended(job);

    assert_eq!(questions_asked(&rig), 1);
    assert_eq!(rig.ended_count(job), 1);
    for name in ["a", "b", "c"] {
        assert_eq!(read(&root.path().join("dst").join(name)), b"new", "{name}");
    }
}

#[test]
fn without_apply_to_all_every_conflict_asks() {
    let root = clashing(&["a", "b"]);
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["a", "b"]);
    let first = rig.answer_next(job, Answer::Skip, false);
    let second = rig.answer_next(job, Answer::Skip, false);
    let ended = rig.wait_ended(job);

    assert_eq!((first.question, second.question), (1, 2));
    assert_eq!(ended.skipped, 2);
}

#[test]
fn compare_reports_whether_the_files_are_identical_and_the_question_stays_open() {
    let root = temp();
    for (name, source, target) in [
        ("same.txt", "equal bytes", "equal bytes"),
        ("other.txt", "one thing!", "another!!!"),
    ] {
        put(root.path(), &format!("src/{name}"), source.as_bytes());
        put(root.path(), &format!("dst/{name}"), target.as_bytes());
    }
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["same.txt", "other.txt"]);
    let mut results = Vec::new();
    for _ in 0..2 {
        // "All" is ignored for a compare, and the question stays open for its final answer.
        let question = rig.answer_next(job, Answer::Compare, true);
        let OpsEvent::Compared {
            question: number,
            identical,
            ..
        } = rig.wait(|event| matches!(event, OpsEvent::Compared { .. }))
        else {
            unreachable!("a compared event");
        };
        assert_eq!(number, question.question);
        results.push((question.name, identical));

        rig.engine
            .answer(job, question.question, Answer::Skip, false)
            .expect("still open");
    }
    rig.wait_ended(job);

    results.sort();
    assert_eq!(
        results,
        [("other.txt".to_owned(), false), ("same.txt".to_owned(), true)]
    );
}

#[test]
fn overwriting_a_file_with_itself_skips() {
    let root = temp();
    put(root.path(), "src/a.txt", b"only copy");
    let mut rig = Rig::new();

    // Copying a file into its own folder: the target is the source.
    let job = rig.start(super::testing::copy(
        &root.path().join("src"),
        &["a.txt"],
        &root.path().join("src"),
    ));
    rig.answer_next(job, Answer::Overwrite, false);
    rig.wait_ended(job);

    assert_eq!(read(&root.path().join("src/a.txt")), b"only copy");
    assert_eq!(tree(&root.path().join("src")), ["a.txt"]);
}

#[test]
fn copying_a_file_into_its_own_folder_can_rename() {
    let root = temp();
    put(root.path(), "src/a.txt", b"content");
    let mut rig = Rig::new();

    let job = rig.start(super::testing::copy(
        &root.path().join("src"),
        &["a.txt"],
        &root.path().join("src"),
    ));
    rig.answer_next(job, Answer::Rename, false);
    rig.wait_ended(job);

    assert_eq!(tree(&root.path().join("src")), ["a (2).txt", "a.txt"]);
    assert_eq!(read(&root.path().join("src/a (2).txt")), b"content");
}

#[test]
fn folders_merge_and_only_files_that_exist_are_asked_about() {
    let root = temp();
    put(root.path(), "src/tree/new.txt", b"new");
    put(root.path(), "src/tree/both.txt", b"source");
    put(root.path(), "src/tree/sub/deep.txt", b"deep");
    put(root.path(), "dst/tree/both.txt", b"target");
    put(root.path(), "dst/tree/only-there.txt", b"keep");
    let mut rig = Rig::new();

    let job = rig.copy_in(root.path(), &["tree"]);
    rig.answer_next(job, Answer::Skip, false);
    rig.wait_ended(job);

    assert_eq!(questions_asked(&rig), 1);
    assert_eq!(
        tree(&root.path().join("dst/tree")),
        ["both.txt", "new.txt", "only-there.txt", "sub/", "sub/deep.txt"]
    );
    assert_eq!(read(&root.path().join("dst/tree/both.txt")), b"target");
    assert_eq!(read(&root.path().join("dst/tree/new.txt")), b"new");
}

#[test]
fn a_file_over_a_folder_and_a_folder_over_a_file_fail_the_item_without_asking() {
    let root = temp();
    put(root.path(), "src/file", b"file");
    put(root.path(), "src/folder/inner", b"inner");
    fs::create_dir_all(root.path().join("dst/file")).expect("folder");
    put(root.path(), "dst/folder", b"a file");
    let mut rig = Rig::new();

    let ended = rig.run_copy(root.path(), &["file", "folder"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(questions_asked(&rig), 0);
    let mut failed: Vec<&str> = ended.errors.iter().map(|error| error.path.as_str()).collect();
    failed.sort_unstable();
    assert_eq!(failed, ["file", "folder"]);
    assert_eq!(read(&root.path().join("dst/folder")), b"a file");
}

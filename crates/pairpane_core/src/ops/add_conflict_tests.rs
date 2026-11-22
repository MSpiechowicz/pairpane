//! The questions an add job asks when a file of the archive has the name of a new one, the folders that merge, the files and
//! folders that cannot replace each other, and an archive that changes under the job: it is refused, and what was there
//! stays as it was.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::archive::{Fingerprint, Input, build_index};
use crate::model::{Answer, ArchiveFormat, ConflictInfo, ControlAction, JobPhase, OpsEvent, Outcome};

use super::ArchiveJob;
use super::add_tests::{KINDS, add, archived, knows, new_ones, old_ones, password, paths, scene_with};
use super::hooks::Hooks;
use super::pack_tests::{at, cancelling_rig, expected};
use super::testing::{Ended, Rig, has_no_partial_files, long_ago, pattern, put, read, set_times};

/// A scene whose `add` has the file `a.txt` (which the archive has, as "alpha") with other content, modified at `seconds`.
fn clashing(name: &str, format: ArchiveFormat, secret: Option<&str>, seconds: u64) -> TempDir {
    let root = scene_with(name, format, secret, false);
    put(root.path(), "add/a.txt", b"ALPHA NEW");
    set_times(&root.path().join("add/a.txt"), at(seconds));

    root
}

/// The job that adds `a.txt` to the archive, answers its question with `answer`, and how it ended with the question.
fn answered(root: &TempDir, name: &str, secret: Option<&str>, answer: Answer) -> (Ended, ConflictInfo) {
    let mut rig = Rig::new();

    let job = rig.start(add(root.path(), &["a.txt"], name, "", knows(secret)));
    let question = rig.answer_next(job, answer, false);

    (rig.wait_ended(job), question)
}

/// The content of `path` in the entries.
fn content_of(entries: &[super::pack_tests::Seen], path: &str) -> Vec<Vec<u8>> {
    entries
        .iter()
        .filter(|seen| seen.0 == path)
        .map(|seen| seen.4.clone())
        .collect()
}

#[test]
fn overwrite_leaves_the_old_entry_out_and_writes_the_new_one() {
    for (name, format, secret, _) in KINDS {
        let root = clashing(name, format, secret, 2_000_000_000);

        let (ended, question) = answered(&root, name, secret, Answer::Overwrite);

        assert_eq!((ended.outcome, ended.skipped), (Outcome::Done, 0), "{name}");
        assert_eq!(question.name, "a.txt", "{name}");
        assert_eq!(question.suggested, "a (2).txt", "{name}");
        assert_eq!(
            (question.target.size, question.source.size, question.target.dir),
            (5, 9, false),
            "{name}"
        );
        assert!(!question.comparable, "{name}");
        assert_eq!(question.target.modified, Some(1_000_000_001_000), "{name}");
        let entries = archived(&root, name, format, secret);
        assert_eq!(
            content_of(&entries, "a.txt"),
            [b"ALPHA NEW".to_vec()],
            "{name}: one entry has the name"
        );
        assert_eq!(entries.len(), expected().len(), "{name}");
    }
}

#[test]
fn skip_adds_nothing_and_leaves_the_archive_as_it_was() {
    for (name, format, secret, _) in KINDS {
        let root = clashing(name, format, secret, 2_000_000_000);
        let before = read(&root.path().join("dst").join(name));

        let (ended, _) = answered(&root, name, secret, Answer::Skip);

        assert_eq!((ended.outcome, ended.skipped), (Outcome::Done, 1), "{name}");
        assert_eq!(
            read(&root.path().join("dst").join(name)),
            before,
            "{name}: not even rewritten"
        );
    }
}

#[test]
fn rename_adds_the_file_under_a_free_name_that_neither_the_archive_nor_the_job_uses() {
    for (name, format, secret, _) in KINDS {
        let root = clashing(name, format, secret, 2_000_000_000);
        // The archive has "a (2).txt" already: the free name is the next one.
        put(root.path(), "add/other.txt", b"other");
        let source = knows(secret);
        let mut rig = Rig::new();
        let job = rig.start(add(root.path(), &["a.txt", "other.txt"], name, "", source));

        let question = rig.answer_next(job, Answer::Rename, false);
        let ended = rig.wait_ended(job);

        assert_eq!(ended.outcome, Outcome::Done, "{name}");
        assert_eq!(question.suggested, "a (2).txt", "{name}");
        let entries = archived(&root, name, format, secret);
        assert_eq!(content_of(&entries, "a.txt"), [b"alpha".to_vec()], "{name}");
        assert_eq!(content_of(&entries, "a (2).txt"), [b"ALPHA NEW".to_vec()], "{name}");
        assert_eq!(old_ones(&entries).len(), expected().len(), "{name}");
        assert_eq!(
            new_ones(&entries).iter().map(|new| new.0.as_str()).collect::<Vec<_>>(),
            ["a (2).txt", "other.txt"],
            "{name}"
        );
    }
}

#[test]
fn the_free_name_is_the_next_one_when_the_archive_has_the_first() {
    let root = clashing("a.zip", ArchiveFormat::Zip, None, 2_000_000_000);
    let mut rig = Rig::new();
    let first = rig.start(add(root.path(), &["a.txt"], "a.zip", "", knows(None)));
    rig.answer_next(first, Answer::Rename, false);
    rig.wait_ended(first);

    let second = rig.start(add(root.path(), &["a.txt"], "a.zip", "", knows(None)));
    let question = rig.answer_next(second, Answer::Rename, false);
    rig.wait_ended(second);

    assert_eq!(question.suggested, "a (3).txt");
    assert_eq!(
        paths(&archived(&root, "a.zip", ArchiveFormat::Zip, None))
            .into_iter()
            .filter(|path| path.starts_with("a "))
            .collect::<Vec<_>>(),
        ["a (2).txt", "a (3).txt"]
    );
}

#[test]
fn overwrite_older_replaces_an_entry_older_than_the_new_file_and_keeps_a_newer_one() {
    for (name, format, secret, _) in KINDS {
        for (seconds, replaced) in [(2_000_000_000, true), (500_000_000, false)] {
            let root = clashing(name, format, secret, seconds);

            let (ended, _) = answered(&root, name, secret, Answer::OverwriteOlder);

            assert_eq!(ended.outcome, Outcome::Done, "{name}");
            assert_eq!(ended.skipped, u32::from(!replaced), "{name}");
            let content = content_of(&archived(&root, name, format, secret), "a.txt");
            assert_eq!(
                content,
                [if replaced {
                    b"ALPHA NEW".to_vec()
                } else {
                    b"alpha".to_vec()
                }],
                "{name}"
            );
        }
    }
}

#[test]
fn an_answer_for_all_decides_the_next_conflicts_without_asking() {
    let root = clashing("a.zip", ArchiveFormat::Zip, None, 2_000_000_000);
    put(root.path(), "add/docs/b.txt", b"B NEW");
    set_times(&root.path().join("add/docs/b.txt"), at(2_000_000_000));
    let mut rig = Rig::new();

    let job = rig.start(add(root.path(), &["a.txt", "docs"], "a.zip", "", knows(None)));
    rig.answer_next(job, Answer::Overwrite, true);
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    let entries = archived(&root, "a.zip", ArchiveFormat::Zip, None);
    assert_eq!(content_of(&entries, "a.txt"), [b"ALPHA NEW".to_vec()]);
    assert_eq!(content_of(&entries, "docs/b.txt"), [b"B NEW".to_vec()]);
    assert_eq!(content_of(&entries, "docs/deep/c.txt"), [b"gamma".to_vec()]);
}

#[test]
fn a_folder_the_archive_has_is_merged_and_not_asked_about() {
    for (name, format, secret, _) in KINDS {
        let root = scene_with(name, format, secret, false);
        put(root.path(), "add/docs/added.txt", b"added");

        let ended = Rig::new().run(add(root.path(), &["docs"], name, "", knows(secret)));

        assert_eq!(
            (ended.outcome, ended.skipped, ended.errors.len()),
            (Outcome::Done, 0, 0),
            "{name}"
        );
        let entries = archived(&root, name, format, secret);
        assert_eq!(
            old_ones(&entries),
            expected(),
            "{name}: the folder and what is in it stay"
        );
        assert_eq!(content_of(&entries, "docs/added.txt"), [b"added".to_vec()], "{name}");
    }
}

#[test]
fn a_file_where_the_archive_has_a_folder_and_a_folder_where_it_has_a_file_fail_their_items_only() {
    for (name, format, secret, _) in KINDS {
        let root = scene_with(name, format, secret, false);
        put(root.path(), "add/docs", b"a file named like a folder");
        put(root.path(), "add/a.txt/inside.txt", b"in a folder named like a file");

        let ended = Rig::new().run(add(root.path(), &["docs", "a.txt", "new.txt"], name, "", knows(secret)));

        assert_eq!(ended.outcome, Outcome::Done, "{name}");
        let mut failed: Vec<&str> = ended.errors.iter().map(|error| error.path.as_str()).collect();
        failed.sort_unstable();
        assert_eq!(failed, ["a.txt", "docs"], "{name}");
        assert!(
            ended
                .errors
                .iter()
                .all(|error| error.error.message.starts_with("a folder of the archive")
                    || error.error.message.starts_with("a file of the archive")),
            "{name}"
        );
        let entries = archived(&root, name, format, secret);
        assert_eq!(old_ones(&entries), expected(), "{name}");
        assert_eq!(
            new_ones(&entries).iter().map(|new| new.0.as_str()).collect::<Vec<_>>(),
            ["new.txt"],
            "{name}: nothing below the failed folder was added"
        );
    }
}

/// The index and the state of the archive `name` of the scene, as a page that listed it would hand them over.
fn listed(root: &TempDir, name: &str, format: ArchiveFormat, secret: Option<&str>) -> ArchiveJob {
    let path = root.path().join("dst").join(name);
    let file = fs::File::open(&path).expect("the archive");
    let password = password(secret);
    let index = build_index(
        format,
        &Input {
            file: &file,
            path: Some(&path),
        },
        &AtomicBool::new(false),
        password.as_deref(),
    )
    .expect("an index");

    ArchiveJob {
        index: Some(Arc::new(index)),
        fingerprint: Some(Fingerprint::of(&path, &file.metadata().expect("metadata"))),
        password,
    }
}

/// Appends a byte to the archive: its size and time change.
fn touch(root: &TempDir, name: &str) -> Vec<u8> {
    let path = root.path().join("dst").join(name);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("the archive")
        .write_all(b"\0")
        .expect("appended");

    read(&path)
}

fn refused_as_changed(ended: &Ended) {
    let Outcome::Failed { error } = &ended.outcome else {
        panic!("the job must fail: {:?}", ended.outcome);
    };

    assert_eq!(error.message, "the archive changed; open it again");
}

#[test]
fn an_archive_that_changed_since_it_was_listed_is_refused_before_anything_is_written() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    let source = listed(&root, "a.zip", ArchiveFormat::Zip, None);
    let changed = touch(&root, "a.zip");

    let ended = Rig::new().run(add(root.path(), &["new.txt"], "a.zip", "", source));

    refused_as_changed(&ended);
    assert_eq!(read(&root.path().join("dst/a.zip")), changed);
    assert!(has_no_partial_files(&root.path().join("dst")));
}

#[test]
fn an_index_without_the_state_it_was_made_from_is_not_trusted() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    let source = ArchiveJob {
        fingerprint: None,
        ..listed(&root, "a.zip", ArchiveFormat::Zip, None)
    };

    let ended = Rig::new().run(add(root.path(), &["new.txt"], "a.zip", "", source));

    refused_as_changed(&ended);
}

#[test]
fn an_archive_that_changes_while_the_new_one_is_written_is_not_replaced() {
    for (name, format, secret, _) in KINDS.iter().filter(|kind| kind.2.is_none()) {
        let root = scene_with(name, *format, *secret, false);
        let source = listed(&root, name, *format, *secret);
        let mut rig = Rig::paused_in(JobPhase::Packing);

        let job = rig.start(add(root.path(), &["new.txt"], name, "", source));
        rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
        // Its time changes under the job (a write that does not disturb what the job reads).
        let path = root.path().join("dst").join(name);
        let before = read(&path);
        set_times(&path, long_ago());
        rig.engine.control(job, ControlAction::Resume).expect("known job");
        let ended = rig.wait_ended(job);

        refused_as_changed(&ended);
        assert_eq!(read(&path), before, "{name}");
        assert!(has_no_partial_files(&root.path().join("dst")), "{name}");
    }
}

#[test]
fn an_archive_replaced_by_another_file_while_the_new_one_is_written_is_not_replaced() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    let source = listed(&root, "a.zip", ArchiveFormat::Zip, None);
    let mut rig = Rig::paused_in(JobPhase::Packing);

    let job = rig.start(add(root.path(), &["new.txt"], "a.zip", "", source));
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    // The same bytes, the same size, in a new file: the identity of the file is what tells it.
    let path = root.path().join("dst/a.zip");
    let bytes = read(&path);
    let times = fs::metadata(&path).expect("metadata").modified().expect("time");
    fs::remove_file(&path).expect("removed");
    put(root.path(), "dst/a.zip", &bytes);
    set_times(&path, times);
    rig.engine.control(job, ControlAction::Resume).expect("known job");
    let ended = rig.wait_ended(job);

    // Where the system says nothing of which file it is (no identity), the same size and time cannot tell it.
    if !matches!(ended.outcome, Outcome::Done) {
        refused_as_changed(&ended);
        assert_eq!(read(&path), bytes);
    }
    assert!(has_no_partial_files(&root.path().join("dst")));
}

#[test]
fn a_cancel_in_the_middle_of_the_work_leaves_the_archive_and_no_temporary_file() {
    for (name, format, secret, _) in KINDS {
        let root = scene_with(name, format, secret, false);
        put(root.path(), "add/big.bin", &pattern(16 * 1024 * 1024));
        let before = read(&root.path().join("dst").join(name));
        let mut rig = cancelling_rig();

        let ended = rig.run(add(root.path(), &["big.bin"], name, "", knows(secret)));

        assert_eq!(ended.outcome, Outcome::Cancelled, "{name}");
        assert_eq!(read(&root.path().join("dst").join(name)), before, "{name}");
        assert!(has_no_partial_files(&root.path().join("dst")), "{name}");
    }
}

#[test]
fn a_disk_with_less_room_than_the_reserve_fails_the_job_and_leaves_the_archive() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    let before = read(&root.path().join("dst/a.zip"));
    let hooks = Hooks {
        free_reserve: Some(u64::MAX / 2),
        ..Hooks::default()
    };

    let ended = Rig::hooked(|set| *set = hooks).run(add(root.path(), &["new.txt"], "a.zip", "", knows(None)));

    let Outcome::Failed { error } = ended.outcome else {
        panic!("the job must fail: {:?}", ended.outcome);
    };
    assert_eq!(error.message, "there is not enough free space left on the disk");
    assert_eq!(read(&root.path().join("dst/a.zip")), before);
    assert!(has_no_partial_files(&root.path().join("dst")));
}

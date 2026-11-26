//! Downloads through the real engine: `pairpane_core::ops::Engine` copying and moving from the in-process server over
//! real SFTP into a temporary folder. The tests of what goes wrong are in `transfer_down_stop`.

use std::fs;
use std::sync::atomic::Ordering;
use std::time::Duration;

use pairpane_core::model::{Answer, ControlAction, OpKind, OpsEvent, Outcome};
use pretty_assertions::assert_eq;

use super::down_rig::{Rig, long_ago, modified, set_times, tree};
use super::transfer_fs::pattern;
use crate::test_server::Settings;

/// The tree every download test uses, below `/data` of the server.
fn put_tree(rig: &Rig) {
    rig.h.server.put("data/tree/a.txt", b"alpha");
    rig.h.server.put("data/tree/deep/b.bin", &pattern(700_001));
    rig.h.server.put("data/tree/deep/empty.txt", b"");
    rig.h.server.put("data/c.txt", b"c");
}

const TREE: [&str; 6] = [
    "c.txt",
    "tree/",
    "tree/a.txt",
    "tree/deep/",
    "tree/deep/b.bin",
    "tree/deep/empty.txt",
];

fn assert_whole(rig: &Rig) {
    let local = rig.local.path();

    assert_eq!(rig.local_tree(), TREE);
    assert_eq!(fs::read(local.join("tree/a.txt")).expect("file"), b"alpha");
    assert!(fs::read(local.join("tree/deep/b.bin")).expect("file") == pattern(700_001));
    assert_eq!(fs::read(local.join("c.txt")).expect("file"), b"c");
}

#[test]
fn a_file_and_a_tree_are_copied_with_their_bytes_and_times() {
    let mut rig = Rig::new(Settings::default());

    put_tree(&rig);

    for name in ["data/tree/a.txt", "data/tree/deep/b.bin", "data/c.txt"] {
        set_times(&rig.on_server(name));
    }

    #[cfg(unix)]
    set_times(&rig.on_server("data/tree/deep"));

    let ended = rig.download(OpKind::Copy, "/data", &["tree", "c.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!((ended.skipped, ended.errors.len()), (0, 0), "{:?}", ended.errors);
    assert_whole(&rig);

    for name in ["tree/a.txt", "tree/deep/b.bin", "c.txt"] {
        assert_eq!(modified(&rig.local.path().join(name)), long_ago(), "{name}");
    }

    #[cfg(unix)]
    assert_eq!(modified(&rig.local.path().join("tree/deep")), long_ago());

    assert_eq!(
        tree(&rig.on_server("data")).len(),
        TREE.len(),
        "a copy leaves the server as it was"
    );
    assert_eq!(rig.h.server.counts.removes.load(Ordering::Relaxed), 0);
    rig.no_handles_left();
}

#[test]
fn a_move_removes_the_server_files_once_they_are_here() {
    let mut rig = Rig::new(Settings::default());

    put_tree(&rig);

    let ended = rig.download(OpKind::Move, "/data", &["tree", "c.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!((ended.skipped, ended.errors.len()), (0, 0), "{:?}", ended.errors);
    assert_whole(&rig);
    assert_eq!(
        tree(&rig.on_server("data")),
        Vec::<String>::new(),
        "files and folders are gone from the server"
    );
    assert_eq!(
        rig.h.server.counts.removes.load(Ordering::Relaxed),
        TREE.len(),
        "one removal for each file and folder"
    );
    rig.no_handles_left();
}

#[test]
fn a_move_keeps_the_file_on_the_server_when_it_is_not_taken() {
    let mut rig = Rig::new(Settings::default());

    rig.h.server.put("data/a.txt", b"from the server");
    rig.h.server.put("data/b.txt", b"b");
    fs::write(rig.local.path().join("a.txt"), b"mine").expect("file");

    let job = rig.start(rig.request(OpKind::Move, "/data", &["a.txt", "b.txt"]));
    let question = match rig.wait(|event| matches!(event, OpsEvent::Conflict { .. })) {
        OpsEvent::Conflict { conflict } => conflict,
        other => unreachable!("{other:?}"),
    };

    rig.engine
        .answer(job, question.question, Answer::Skip, false)
        .expect("an open question");

    let ended = rig.ended(job);

    assert_eq!(
        (ended.outcome, ended.skipped, ended.errors.len()),
        (Outcome::Done, 1, 0)
    );
    assert_eq!(fs::read(rig.local.path().join("a.txt")).expect("file"), b"mine");
    assert_eq!(rig.local_tree(), ["a.txt", "b.txt"]);
    assert_eq!(
        tree(&rig.on_server("data")),
        ["a.txt"],
        "only what was moved is removed"
    );
}

#[test]
fn replies_shorter_than_asked_give_the_same_files() {
    let mut rig = Rig::new(Settings {
        short_reads: Some(5_000),
        ..Settings::default()
    });

    put_tree(&rig);

    let ended = rig.download(OpKind::Copy, "/data", &["tree", "c.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 0, "{:?}", ended.errors);
    assert_whole(&rig);
}

#[test]
fn a_verified_download_passes() {
    let mut rig = Rig::new(Settings::default());

    put_tree(&rig);

    for kind in [OpKind::Copy, OpKind::Move] {
        let mut request = rig.request(kind, "/data", &["tree", "c.txt"]);

        request.verify = true;

        let ended = rig.run(request);

        assert_eq!(ended.outcome, Outcome::Done, "{kind:?}");
        assert_eq!(ended.errors.len(), 0, "{kind:?}: {:?}", ended.errors);
        assert_whole(&rig);

        if kind == OpKind::Copy {
            fs::remove_dir_all(rig.local.path().join("tree")).expect("removed");
            fs::remove_file(rig.local.path().join("c.txt")).expect("removed");
        }
    }

    assert_eq!(tree(&rig.on_server("data")), Vec::<String>::new());
}

#[test]
fn a_paused_job_longer_than_the_idle_time_goes_on_over_the_same_connection() {
    let mut rig = Rig::idling(
        Settings {
            stall_reads_after: Some(10),
            ..Settings::default()
        },
        Duration::from_millis(200),
    );

    rig.h.server.put("data/big.bin", &pattern(3_000_000));

    let job = rig.start(rig.request(OpKind::Copy, "/data", &["big.bin"]));

    // The file is open and being read: the server holds some of the requests back. Pausing, and then letting them
    // through, stops the job at its next checkpoint, in the middle of the file.
    rig.wait_for_reads(11);
    rig.control(job, ControlAction::Pause);
    rig.h.server.counts.release_reads();

    // Several times the idle time, with nothing moving.
    std::thread::sleep(Duration::from_millis(1_200));

    assert!(!rig.has_ended(job), "the job is paused");
    assert_eq!(rig.h.server.counts.open_handles.load(Ordering::Relaxed), 1);

    rig.control(job, ControlAction::Resume);

    let ended = rig.ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 0, "{:?}", ended.errors);
    assert!(fs::read(rig.local.path().join("big.bin")).expect("file") == pattern(3_000_000));
    assert_eq!(
        rig.h.server.counts.connections.load(Ordering::Relaxed),
        1,
        "the open file kept the connection"
    );
    rig.no_handles_left();
}

#[test]
fn a_server_folder_that_is_the_target_folder_is_refused_before_anything_is_written() {
    for kind in [OpKind::Copy, OpKind::Move] {
        let mut rig = Rig::new(Settings::default());

        rig.h.server.put("data/a.txt", b"mine");

        let served = rig.h.server.root.path().join("data");
        let before = tree(rig.h.server.root.path());
        let request = rig.request_to(kind, "/data", &["a.txt"], &served);
        let ended = rig.run(request);

        assert_eq!(
            ended.failure(),
            "the folder of the items is the target folder",
            "{kind:?}"
        );
        assert_eq!(
            tree(rig.h.server.root.path()),
            before,
            "no probe is left and nothing was touched"
        );
        assert_eq!(fs::read(served.join("a.txt")).expect("file"), b"mine");
        assert_eq!(rig.h.server.counts.removes.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn a_server_folder_is_not_copied_into_itself() {
    let mut rig = Rig::new(Settings::default());

    rig.h.server.put("data/d/x.txt", b"x");

    let before = tree(rig.h.server.root.path());
    let into = rig.h.server.root.path().join("data/d/in");
    let request = rig.request_to(OpKind::Move, "/data", &["d"], &into);
    let ended = rig.run(request);

    assert_eq!(ended.failure(), "a folder cannot be copied or moved into itself");
    assert_eq!(
        tree(rig.h.server.root.path()),
        before,
        "the folder made for it is removed again"
    );
    assert_eq!(rig.h.server.counts.removes.load(Ordering::Relaxed), 0);
}

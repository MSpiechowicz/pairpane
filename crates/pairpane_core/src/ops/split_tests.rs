//! Split jobs on real folders: the parts, the `.crc` file, and what is left behind when a job stops.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

use crate::model::{ControlAction, JobPhase, OpsEvent, Outcome};

use super::hooks::Hooks;
use super::testing::{Ended, Rig, has_no_partial_files, pattern, phases, put, read, task, temp, tree};
use super::{Refused, Task};

const PART: u64 = 2048;

fn split(part_size: u64) -> Task {
    Task::Split { part_size }
}

/// Splits `root/src/<name>` into `root/out`, to its end.
fn run_split(root: &Path, name: &str, part_size: u64) -> (Rig, Ended) {
    let mut rig = Rig::new();
    let src = root.join("src");

    let ended = rig.run(task(&src, &[name], &root.join("out"), split(part_size)));

    (rig, ended)
}

fn failure(ended: &Ended) -> String {
    match &ended.outcome {
        Outcome::Failed { error } => error.message.clone(),
        other => panic!("the job fails: {other:?}"),
    }
}

fn crc_of(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);

    hasher.finalize()
}

#[test]
fn the_parts_join_into_the_file_and_the_crc_file_says_what_it_is() {
    let root = temp();
    let bytes = pattern(5000);
    put(root.path(), "src/big.bin", &bytes);

    let (rig, ended) = run_split(root.path(), "big.bin", PART);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    let out = root.path().join("out");
    assert_eq!(tree(&out), ["big.bin.001", "big.bin.002", "big.bin.003", "big.bin.crc"]);
    let joined: Vec<u8> = ["001", "002", "003"]
        .iter()
        .flat_map(|number| read(&out.join(format!("big.bin.{number}"))))
        .collect();
    assert_eq!(joined, bytes);
    assert_eq!(read(&out.join("big.bin.001")).len(), 2048);
    assert_eq!(read(&out.join("big.bin.003")).len(), 5000 - 2 * 2048);
    assert_eq!(
        read(&out.join("big.bin.crc")),
        format!("filename=big.bin\r\nsize=5000\r\ncrc32={:08X}\r\n", crc_of(&bytes)).into_bytes()
    );
    assert!(phases(&rig).contains(&JobPhase::Splitting));
    assert_eq!(read(&root.path().join("src/big.bin")), bytes, "the file stays");
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn a_file_that_fills_its_parts_exactly_makes_no_empty_part() {
    let root = temp();
    put(root.path(), "src/big.bin", &pattern(4096));

    let (_, ended) = run_split(root.path(), "big.bin", PART);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        tree(&root.path().join("out")),
        ["big.bin.001", "big.bin.002", "big.bin.crc"]
    );
}

#[cfg(unix)]
#[test]
fn the_parts_get_the_permissions_of_the_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp();
    put(root.path(), "src/big.bin", &pattern(3000));
    fs::set_permissions(root.path().join("src/big.bin"), fs::Permissions::from_mode(0o640)).expect("mode");

    run_split(root.path(), "big.bin", PART);

    let mode = |name: &str| {
        fs::metadata(root.path().join("out").join(name))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!((mode("big.bin.001"), mode("big.bin.002")), (0o640, 0o640));
}

#[test]
fn the_target_folder_is_made_and_the_parts_can_go_beside_the_file() {
    let root = temp();
    put(root.path(), "src/big.bin", &pattern(3000));
    let mut rig = Rig::new();
    let src = root.path().join("src");

    let ended = rig.run(task(&src, &["big.bin"], &src, split(PART)));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(tree(&src), ["big.bin", "big.bin.001", "big.bin.002", "big.bin.crc"]);

    let (_, ended) = run_split(root.path(), "big.bin", PART);
    assert_eq!(ended.outcome, Outcome::Done);
    assert!(root.path().join("out/big.bin.001").is_file(), "made the folder");
}

#[test]
fn a_file_that_needs_more_than_999_parts_is_refused_before_anything_is_written() {
    let root = temp();
    put(root.path(), "src/big.bin", &vec![7; 1000 * 1024]);

    let (_, ended) = run_split(root.path(), "big.bin", 1024);

    assert_eq!(
        failure(&ended),
        "the file would need more than 999 parts; choose a bigger part size"
    );
    assert!(
        !root.path().join("out").exists(),
        "the folder the job made is removed again"
    );

    let (_, ended) = run_split(root.path(), "big.bin", 1026);
    assert_eq!(ended.outcome, Outcome::Done, "999 parts are the most that fit");
    assert_eq!(tree(&root.path().join("out")).len(), 999 + 1);
}

#[test]
fn an_empty_file_and_a_part_that_is_too_small_are_refused() {
    let root = temp();
    put(root.path(), "src/empty", b"");
    let rig = Rig::new();
    let src = root.path().join("src");

    let (_, ended) = run_split(root.path(), "empty", PART);
    assert_eq!(failure(&ended), "an empty file cannot be split");

    assert_eq!(
        rig.engine
            .submit(task(&src, &["empty"], &src, split(1023)))
            .expect_err("refused"),
        Refused::InvalidTask
    );
}

#[cfg(unix)]
#[test]
fn a_link_and_a_folder_are_not_split() {
    let root = temp();
    put(root.path(), "src/real.bin", &pattern(3000));
    put(root.path(), "src/dir/x", b"x");
    std::os::unix::fs::symlink(root.path().join("src/real.bin"), root.path().join("src/link.bin")).expect("link");

    for name in ["link.bin", "dir"] {
        let (_, ended) = run_split(root.path(), name, PART);

        assert_eq!(failure(&ended), "only a regular file can be split", "{name}");
    }
}

#[test]
fn a_part_name_that_is_taken_fails_the_job_and_nothing_is_overwritten() {
    let root = temp();
    put(root.path(), "src/big.bin", &pattern(5000));
    put(root.path(), "out/big.bin.002", b"mine");

    let (_, ended) = run_split(root.path(), "big.bin", PART);

    assert_eq!(failure(&ended), "something with this name exists already");
    assert_eq!(tree(&root.path().join("out")), ["big.bin.002"]);
    assert_eq!(read(&root.path().join("out/big.bin.002")), b"mine");
}

#[test]
fn a_name_that_gets_taken_while_the_job_runs_removes_what_it_made() {
    let root = temp();
    put(root.path(), "src/big.bin", &pattern(5000));
    let late = root.path().join("out/big.bin.003");
    let hooks = Hooks {
        every_chunk: true,
        ..Hooks::default()
    };
    // The third part's name is taken after the first part is written: a race no check before the job can see.
    let mut rig = Rig::with(hooks, move |_, event| {
        if matches!(event, OpsEvent::Progress { done_bytes, .. } if *done_bytes >= PART) {
            fs::write(&late, b"mine").expect("written");
        }
    });
    let src = root.path().join("src");

    let ended = rig.run(task(&src, &["big.bin"], &root.path().join("out"), split(PART)));

    assert_eq!(failure(&ended), "something with this name exists already");
    assert_eq!(tree(&root.path().join("out")), ["big.bin.003"]);
    assert_eq!(read(&root.path().join("out/big.bin.003")), b"mine", "not overwritten");
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn a_cancel_after_two_parts_removes_the_parts_and_leaves_nothing() {
    let root = temp();
    put(root.path(), "src/big.bin", &pattern(7000));
    let hooks = Hooks {
        every_chunk: true,
        ..Hooks::default()
    };
    let mut rig = Rig::with(hooks, |engine, event| {
        if let OpsEvent::Progress { id, done_bytes, .. } = event
            && *done_bytes >= 2 * PART
        {
            engine.control(*id, ControlAction::Pause).expect("known job");
        }
    });
    let src = root.path().join("src");

    let job = rig.start(task(&src, &["big.bin"], &root.path().join("out"), split(PART)));
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));

    // The pause is asked for while the second part is written; the job stops once that part is in place.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.path().join("out/big.bin.002").exists() {
        assert!(Instant::now() < deadline, "the second part is put in place");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        tree(&root.path().join("out")),
        ["big.bin.001", "big.bin.002"],
        "two parts are in place while it waits"
    );

    rig.engine.control(job, ControlAction::Cancel).expect("known job");

    assert_eq!(rig.wait_ended(job).outcome, Outcome::Cancelled);
    assert!(!root.path().join("out").exists(), "the parts and the folder are gone");
    assert!(has_no_partial_files(root.path()));
}

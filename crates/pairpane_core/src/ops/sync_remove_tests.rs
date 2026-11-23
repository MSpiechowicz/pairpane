//! Synchronize jobs that mirror: the items only the side written to has are removed after the copies, each only while it is
//! still what the scan saw, and what the job refuses to remove when it is submitted.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use pretty_assertions::assert_eq;

use crate::model::{EntryKind, OpsEvent, Outcome, PaneId, SyncDirection, SyncFacts};

use super::sync::facts;
use super::sync_tests::{Pair, file, is_done, moment, plan};
use super::testing::{Ended, Rig, read, set_times, task};
use super::{ArchiveJob, Refused, Request, SyncPlan, SyncRemove, SyncRoot, Task};

use SyncDirection::ToRight;

/// The files below `folder` and their bytes, links left out and never followed.
pub(super) fn count(folder: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);

    for entry in fs::read_dir(folder).expect("listed") {
        let entry = entry.expect("entry");
        let kind = entry.file_type().expect("type");

        if kind.is_file() {
            files += 1;
            bytes += entry.metadata().expect("meta").len();
        } else if kind.is_dir() {
            let (more, more_bytes) = count(&entry.path());
            files += more;
            bytes += more_bytes;
        }
    }

    (files, bytes)
}

/// What the scan would have said of `rel` on `side`, held in `root`: the item as it is now.
pub(super) fn removal(root: &Path, rel: &str, side: PaneId) -> SyncRemove {
    let path = root.join(rel);
    let seen = facts(&fs::symlink_metadata(&path).expect("there"));
    let (files, bytes) = if seen.kind == EntryKind::Dir {
        count(&path)
    } else {
        (0, 0)
    };

    SyncRemove {
        side,
        seen,
        files,
        bytes,
        scanned: millis(SystemTime::now()) + 3_000,
    }
}

/// `time` in milliseconds since the Unix epoch.
pub(super) fn millis(time: SystemTime) -> i64 {
    i64::try_from(time.duration_since(UNIX_EPOCH).expect("after the epoch").as_millis()).expect("millis")
}

/// The job that copies `copies` and then removes `removed` between `left` and `right`, both folders.
pub(super) fn mirror(
    left: &Path,
    right: &Path,
    copies: &[(&str, SyncPlan)],
    removed: &[(&str, SyncRemove)],
    permanent: bool,
) -> Request {
    let names: Vec<&str> = copies
        .iter()
        .map(|(rel, _)| *rel)
        .chain(removed.iter().map(|(rel, _)| *rel))
        .collect();

    task(
        left,
        &names,
        right,
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Folder,
            plans: copies.iter().map(|(_, plan)| *plan).collect(),
            removals: removed.iter().map(|(_, removal)| *removal).collect(),
            permanent,
        },
    )
}

pub(super) fn run(request: Request) -> Ended {
    Rig::new().run(request)
}

#[test]
fn a_mirror_copies_and_then_removes_what_only_the_target_has() {
    let pair = Pair::new();
    file(&pair.left().join("new.txt"), b"new", 100);
    file(&pair.right().join("gone.txt"), b"gone", 100);
    file(&pair.right().join("old/deep/inner.txt"), b"inner", 100);
    file(&pair.right().join("kept.txt"), b"kept", 100);
    let (left, right) = (pair.left(), pair.right());
    let copy = plan(&left, &right, "new.txt", ToRight);
    let file_removal = removal(&right, "gone.txt", PaneId::Right);
    let folder_removal = removal(&right, "old", PaneId::Right);

    let ended = run(mirror(
        &left,
        &right,
        &[("new.txt", copy)],
        &[("gone.txt", file_removal), ("old", folder_removal)],
        true,
    ));

    is_done(&ended, 0);
    assert_eq!(read(&right.join("new.txt")), b"new");
    assert!(!right.join("gone.txt").exists());
    assert!(!right.join("old").exists());
    assert_eq!(read(&right.join("kept.txt")), b"kept", "what is not asked for stays");
}

#[test]
fn a_job_of_removals_alone_runs_and_says_it_is_removing() {
    let pair = Pair::new();
    file(&pair.left().join("only-left.txt"), b"left", 100);
    let left = pair.left();
    let only = removal(&left, "only-left.txt", PaneId::Left);
    let mut rig = Rig::new();

    let ended = rig.run(mirror(&left, &pair.right(), &[], &[("only-left.txt", only)], true));

    is_done(&ended, 0);
    assert!(!left.join("only-left.txt").exists());
    assert!(rig.seen.iter().any(|event| matches!(
        event,
        OpsEvent::Phase {
            phase: crate::model::JobPhase::Removing,
            ..
        }
    )));
}

#[test]
fn an_item_that_changed_since_the_scan_or_is_gone_is_kept_and_counted() {
    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("edited.txt"), b"before", 100);
    file(&right.join("touched.txt"), b"same", 100);
    file(&right.join("gone.txt"), b"gone", 100);
    file(&right.join("grown/a.txt"), b"a", 100);
    file(&right.join("swapped"), b"a file", 100);
    let removals = [
        ("edited.txt", removal(&right, "edited.txt", PaneId::Right)),
        ("touched.txt", removal(&right, "touched.txt", PaneId::Right)),
        ("gone.txt", removal(&right, "gone.txt", PaneId::Right)),
        ("grown", removal(&right, "grown", PaneId::Right)),
        ("swapped", removal(&right, "swapped", PaneId::Right)),
    ];
    file(&right.join("edited.txt"), b"after, longer", 100);
    file(&right.join("touched.txt"), b"same", 200);
    fs::remove_file(right.join("gone.txt")).expect("removed");
    file(&right.join("grown/deeper/b.txt"), b"b", 100);
    fs::remove_file(right.join("swapped")).expect("removed");
    fs::create_dir(right.join("swapped")).expect("a folder now");

    let ended = run(mirror(&pair.left(), &right, &[], &removals, true));

    is_done(&ended, 5);
    assert_eq!(read(&right.join("edited.txt")), b"after, longer");
    assert_eq!(read(&right.join("touched.txt")), b"same");
    assert_eq!(read(&right.join("grown/deeper/b.txt")), b"b");
    assert!(right.join("swapped").is_dir());
}

#[test]
fn a_folder_with_anything_changed_after_the_scan_began_is_kept_even_with_the_same_totals() {
    let pair = Pair::new();
    let right = pair.right();
    for rel in ["untouched/a.txt", "swapped/a.txt", "edited/a.txt", "future/deep/a.txt"] {
        file(&right.join(rel), b"a", 100);
    }
    // The scan begins once the setup is older than the slack the times are given.
    std::thread::sleep(std::time::Duration::from_millis(2_100));
    let scanned = millis(SystemTime::now());
    let mut rels = vec!["untouched", "swapped", "future"];
    if cfg!(unix) {
        rels.push("edited");
    }
    let removals: Vec<(&str, SyncRemove)> = rels
        .iter()
        .map(|rel| {
            let mut seen = removal(&right, rel, PaneId::Right);
            seen.scanned = scanned;
            (*rel, seen)
        })
        .collect();
    // A file swapped for another of the same size (the folder's time moves), one that says it is from the future, and
    // (Unix) one edited in place with the same size and its old time put back (its change time moves).
    fs::remove_file(right.join("swapped/a.txt")).expect("removed");
    file(&right.join("swapped/b.txt"), b"b", 100);
    set_times(&right.join("future/deep/a.txt"), moment(10_000_000_000));
    file(&right.join("edited/a.txt"), b"b", 100);

    let ended = run(mirror(&pair.left(), &right, &[], &removals, true));

    is_done(&ended, u32::try_from(rels.len() - 1).expect("few"));
    assert!(!right.join("untouched").exists(), "nothing in it changed");
    assert!(right.join("swapped/b.txt").exists());
    assert!(right.join("future/deep/a.txt").exists());
    assert!(right.join("edited/a.txt").exists());
}

#[test]
fn a_folder_that_is_or_holds_a_mounted_drive_is_never_emptied_to_the_trash_or_for_good() {
    for permanent in [true, false] {
        let pair = Pair::new();
        let right = pair.right();
        file(&right.join("drive/precious.txt"), b"keep me", 100);
        file(&right.join("outer/inner/drive/precious.txt"), b"keep me", 100);
        let removals = [
            ("drive", removal(&right, "drive", PaneId::Right)),
            ("outer", removal(&right, "outer", PaneId::Right)),
        ];
        let hooks = super::hooks::Hooks {
            mount_names: vec!["drive".to_owned()],
            ..Default::default()
        };

        let ended = Rig::with(hooks, |_, _| {}).run(mirror(&pair.left(), &right, &[], &removals, permanent));

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.errors.len(), 2, "{:?}", ended.errors);
        assert!(
            ended
                .errors
                .iter()
                .all(|error| error.error.message.contains("mounted drive"))
        );
        assert_eq!(read(&right.join("drive/precious.txt")), b"keep me");
        assert_eq!(read(&right.join("outer/inner/drive/precious.txt")), b"keep me");
    }
}

#[test]
fn a_wide_folder_is_walked_and_removed() {
    let pair = Pair::new();
    let right = pair.right();
    for index in 0..600 {
        file(&right.join(format!("wide/{index}/a.txt")), b"a", 100);
    }
    let seen = removal(&right, "wide", PaneId::Right);

    let ended = run(mirror(&pair.left(), &right, &[], &[("wide", seen)], true));

    is_done(&ended, 0);
    assert!(!right.join("wide").exists());
}

#[test]
fn a_folder_that_holds_the_archive_compared_is_kept() {
    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("box/x.zip"), b"not really a zip", 100);
    let seen = removal(&right, "box", PaneId::Right);
    let request = task(
        &right.join("box"),
        &["box"],
        &right,
        Task::Sync {
            left: SyncRoot::Archive {
                name: "x.zip".to_owned(),
                inner: String::new(),
                source: ArchiveJob::default(),
            },
            right: SyncRoot::Folder,
            plans: Vec::new(),
            removals: vec![seen],
            permanent: true,
        },
    );

    let ended = run(request);

    is_done(&ended, 1);
    assert!(right.join("box/x.zip").exists());
}

#[test]
fn what_a_mirror_cannot_ask_for_is_refused_when_the_job_is_submitted() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    file(&pair.right().join("b.txt"), b"b", 100);
    let (left, right) = (pair.left(), pair.right());
    let good = removal(&right, "b.txt", PaneId::Right);
    let engine = Rig::new().engine;
    let archive = SyncRoot::Archive {
        name: "x.zip".to_owned(),
        inner: String::new(),
        source: ArchiveJob::default(),
    };
    let submit = |names: &[&str], right_root: &SyncRoot, removals: Vec<SyncRemove>| {
        engine.submit(task(
            &left,
            names,
            &right,
            Task::Sync {
                left: SyncRoot::Folder,
                right: right_root.clone(),
                plans: Vec::new(),
                removals,
                permanent: true,
            },
        ))
    };
    let other = SyncRemove {
        seen: SyncFacts {
            kind: EntryKind::Other,
            ..good.seen
        },
        ..good
    };

    assert_eq!(
        submit(&["b.txt"], &archive, vec![good]).unwrap_err(),
        Refused::InvalidTask
    );
    assert_eq!(
        submit(&["b.txt"], &SyncRoot::Folder, vec![other]).unwrap_err(),
        Refused::InvalidTask
    );
    assert_eq!(
        submit(&["b.txt"], &SyncRoot::Folder, vec![good, good]).unwrap_err(),
        Refused::InvalidTask
    );
    assert_eq!(
        submit(&["../b.txt"], &SyncRoot::Folder, vec![good]).unwrap_err(),
        Refused::InvalidNames
    );
    let twins = submit(&["b.txt", "B.TXT"], &SyncRoot::Folder, vec![good, good]);
    if crate::listing::platform::CASE_INSENSITIVE {
        assert_eq!(
            twins.unwrap_err(),
            Refused::InvalidNames,
            "one item under two spellings"
        );
    } else {
        assert!(twins.is_ok());
    }
    assert!(submit(&["b.txt"], &SyncRoot::Folder, vec![good]).is_ok());
}

#[test]
fn a_trash_that_refuses_fails_the_item_and_never_deletes_it_for_good() {
    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("a.txt"), b"keep me", 100);
    let seen = removal(&right, "a.txt", PaneId::Right);
    let hooks = super::hooks::Hooks {
        refuse_trash: true,
        ..Default::default()
    };

    let ended = Rig::with(hooks, |_, _| {}).run(mirror(&pair.left(), &right, &[], &[("a.txt", seen)], false));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(read(&right.join("a.txt")), b"keep me");
}

#[test]
fn an_item_whose_name_the_source_has_by_now_is_kept() {
    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("a.txt"), b"right", 100);
    let seen = removal(&right, "a.txt", PaneId::Right);
    file(&pair.left().join("a.txt"), b"new on the left", 100);

    let ended = run(mirror(&pair.left(), &right, &[], &[("a.txt", seen)], true));

    is_done(&ended, 1);
    assert_eq!(read(&right.join("a.txt")), b"right");
}

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
fn count(folder: &Path) -> (u64, u64) {
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
fn removal(root: &Path, rel: &str, side: PaneId) -> SyncRemove {
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
        scanned: millis(SystemTime::now()) + 1_000,
    }
}

/// `time` in milliseconds since the Unix epoch.
fn millis(time: SystemTime) -> i64 {
    i64::try_from(time.duration_since(UNIX_EPOCH).expect("after the epoch").as_millis()).expect("millis")
}

/// The job that copies `copies` and then removes `removed` between `left` and `right`, both folders.
fn mirror(
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

fn run(request: Request) -> Ended {
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
fn a_folder_with_anything_newer_than_the_scan_in_it_is_kept_even_with_the_same_totals() {
    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("swapped/a.txt"), b"a", 100);
    file(&right.join("edited/a.txt"), b"a", 100);
    file(&right.join("future/a.txt"), b"a", 100);
    file(&right.join("future/deep/b.txt"), b"b", 100);
    let removals = ["swapped", "edited", "future"].map(|rel| {
        let mut seen = removal(&right, rel, PaneId::Right);
        seen.scanned = millis(moment(200));
        set_times(&right.join(rel), moment(150));
        (rel, seen)
    });
    set_times(&right.join("future/deep"), moment(150));
    // A file swapped for another of the same size (the folder's time moves), one edited in place with the same size, and
    // one that says it is from the future.
    fs::remove_file(right.join("swapped/a.txt")).expect("removed");
    file(&right.join("swapped/b.txt"), b"b", 100);
    file(&right.join("edited/a.txt"), b"b", 300);
    set_times(&right.join("future/deep/b.txt"), moment(1_000_000_000));

    let ended = run(mirror(&pair.left(), &right, &[], &removals, true));

    is_done(&ended, 3);
    assert!(right.join("swapped/b.txt").exists());
    assert_eq!(read(&right.join("edited/a.txt")), b"b");
    assert!(right.join("future/deep/b.txt").exists());
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

/// `right/dir/a.txt` to be removed, and a lookalike `elsewhere/a.txt` beside the two folders that must survive.
#[cfg(unix)]
fn lookalikes() -> (Pair, std::path::PathBuf, SyncRemove) {
    let pair = Pair::new();
    let elsewhere = pair.root.path().join("elsewhere");
    file(&elsewhere.join("a.txt"), b"keep me", 100);
    file(&pair.right().join("dir/a.txt"), b"keep me", 100);
    let seen = removal(&pair.right(), "dir/a.txt", PaneId::Right);

    (pair, elsewhere, seen)
}

#[cfg(unix)]
#[test]
fn a_link_on_the_way_is_never_followed() {
    let (pair, elsewhere, seen) = lookalikes();
    let right = pair.right();
    fs::remove_dir_all(right.join("dir")).expect("removed");
    std::os::unix::fs::symlink(&elsewhere, right.join("dir")).expect("link");

    let ended = run(mirror(&pair.left(), &right, &[], &[("dir/a.txt", seen)], true));

    is_done(&ended, 1);
    assert_eq!(read(&elsewhere.join("a.txt")), b"keep me");
}

#[cfg(unix)]
#[test]
fn a_link_where_the_scan_saw_a_folder_is_kept_and_not_followed() {
    let pair = Pair::new();
    let right = pair.right();
    let elsewhere = pair.root.path().join("elsewhere");
    file(&elsewhere.join("a.txt"), b"keep me", 100);
    std::os::unix::fs::symlink(&elsewhere, right.join("link")).expect("link");
    // A link is `other` in the facts, which a removal cannot be asked for.
    let seen = SyncRemove {
        side: PaneId::Right,
        seen: SyncFacts {
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
        },
        files: 1,
        bytes: 7,
        scanned: millis(SystemTime::now()),
    };

    let ended = run(mirror(&pair.left(), &right, &[], &[("link", seen)], true));

    is_done(&ended, 1);
    assert!(
        fs::symlink_metadata(right.join("link")).is_ok(),
        "the link is not as the scan saw it"
    );
    assert_eq!(read(&elsewhere.join("a.txt")), b"keep me");
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

#[cfg(unix)]
#[test]
fn a_folder_that_holds_a_file_it_cannot_read_is_kept_and_reported() {
    use std::os::unix::fs::PermissionsExt;

    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("dir/locked/a.txt"), b"a", 100);
    let seen = removal(&right, "dir", PaneId::Right);
    fs::set_permissions(right.join("dir/locked"), fs::Permissions::from_mode(0o000)).expect("locked");
    // Root reads every folder: there is nothing to show then.
    let readable = fs::read_dir(right.join("dir/locked")).is_ok();

    let ended = run(mirror(&pair.left(), &right, &[], &[("dir", seen)], true));

    fs::set_permissions(right.join("dir/locked"), fs::Permissions::from_mode(0o755)).expect("unlocked");
    if readable {
        return;
    }
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(read(&right.join("dir/locked/a.txt")), b"a");
}

/// Removals to the Trash of the user, from a folder under the home folder (a temporary folder may be on a disk with no
/// Trash); what the test leaves in the Trash is purged.
#[cfg(target_os = "linux")]
mod to_the_trash {
    use std::fs;

    use tempfile::TempDir;

    use super::super::trash::{home_trash, purge_below};
    use super::{PaneId, file, is_done, mirror, removal, run};

    struct Place(TempDir);

    impl Drop for Place {
        fn drop(&mut self) {
            if let Some(trash) = home_trash() {
                purge_below(&trash, self.0.path());
            }
        }
    }

    #[test]
    fn a_removal_goes_to_the_trash_unless_it_is_for_good() {
        let home = std::env::home_dir().expect("a home directory");
        let place = Place(
            tempfile::Builder::new()
                .prefix("pairpane-mirror-test-")
                .tempdir_in(home)
                .expect("temp dir under the home directory"),
        );
        let (left, right) = (place.0.path().join("left"), place.0.path().join("right"));
        fs::create_dir_all(&left).expect("left");
        file(&right.join("dir/a.txt"), b"a", 100);
        let seen = removal(&right, "dir", PaneId::Right);

        let ended = run(mirror(&left, &right, &[], &[("dir", seen)], false));

        is_done(&ended, 0);
        assert!(!right.join("dir").exists());
        let trash = home_trash().expect("a home Trash");
        let found = fs::read_dir(trash.join("info"))
            .expect("the Trash has records")
            .filter_map(Result::ok)
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .any(|record| record.contains(&right.join("dir").to_string_lossy().into_owned()));
        assert!(found, "the folder is in the Trash");
    }
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

/// A folder on the way swapped for a link to another folder after it was looked at, before it is entered: nothing in the
/// other folder is removed, for good or to the Trash.
#[cfg(unix)]
#[test]
fn a_folder_on_the_way_swapped_for_a_link_mid_job_leads_nowhere() {
    use std::sync::Arc;

    for permanent in [true, false] {
        let (pair, elsewhere, seen) = lookalikes();
        let right = pair.right();
        let (dir, target) = (right.join("dir"), elsewhere.clone());
        let hooks = super::hooks::Hooks {
            before_open: Some(Arc::new(move |walked: &str| {
                if walked == "dir" {
                    fs::rename(&dir, dir.with_extension("old")).expect("moved aside");
                    std::os::unix::fs::symlink(&target, &dir).expect("link");
                }
            })),
            ..Default::default()
        };

        let ended =
            Rig::with(hooks, |_, _| {}).run(mirror(&pair.left(), &right, &[], &[("dir/a.txt", seen)], permanent));

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
        assert_eq!(read(&elsewhere.join("a.txt")), b"keep me");
        assert_eq!(read(&right.join("dir.old/a.txt")), b"keep me");
    }
}

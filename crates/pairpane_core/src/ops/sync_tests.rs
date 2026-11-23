//! Synchronize jobs between two folders: what is copied, what is left because the scan no longer holds, what is refused, and
//! the links that are never followed. The items that come out of an archive are in `sync_archive_tests`.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pretty_assertions::assert_eq;

use crate::model::{OpKind, Outcome, SyncDirection, SyncFacts};

use super::sync::facts;
use super::testing::{Ended, Rig, has_no_partial_files, modified, read, set_times, task, temp, tree, write};
use super::{SyncPlan, SyncRoot, Task};

use SyncDirection::{ToLeft, ToRight};

/// `moment(n)`: a time `n` seconds after the epoch, a time files are given so they differ.
pub(super) fn moment(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_000_000_000 + seconds)
}

/// Writes `bytes` to `path` and gives it the time `seconds`.
pub(super) fn file(path: &Path, bytes: &[u8], seconds: u64) {
    write(path, bytes);
    set_times(path, moment(seconds));
}

fn seen(path: &Path) -> Option<SyncFacts> {
    fs::symlink_metadata(path).ok().map(|meta| facts(&meta))
}

/// What the scan would have said of `rel` going `direction` between the folders: both sides as they are now.
pub(super) fn plan(left: &Path, right: &Path, rel: &str, direction: SyncDirection) -> SyncPlan {
    let (source, target) = match direction {
        ToRight => (left, right),
        ToLeft => (right, left),
    };

    SyncPlan {
        direction,
        source: seen(&source.join(rel)).expect("a source"),
        target: seen(&target.join(rel)),
    }
}

/// The job that copies `rels`, each the way `direction` says, between `left` and `right` as the scan saw them now.
pub(super) fn sync_of(left: &Path, right: &Path, rels: &[&str], direction: SyncDirection) -> super::Request {
    let plans = rels.iter().map(|rel| plan(left, right, rel, direction)).collect();

    with_plans(left, right, rels, plans)
}

pub(super) fn with_plans(left: &Path, right: &Path, rels: &[&str], plans: Vec<SyncPlan>) -> super::Request {
    task(
        left,
        rels,
        right,
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Folder,
            plans,
            removals: Vec::new(),
            permanent: false,
        },
    )
}

/// A left and a right folder under one temporary one.
pub(super) struct Pair {
    pub(super) root: tempfile::TempDir,
}

impl Pair {
    pub(super) fn new() -> Self {
        let root = temp();
        fs::create_dir_all(root.path().join("left")).expect("left");
        fs::create_dir_all(root.path().join("right")).expect("right");

        Self { root }
    }

    pub(super) fn left(&self) -> std::path::PathBuf {
        self.root.path().join("left")
    }

    pub(super) fn right(&self) -> std::path::PathBuf {
        self.root.path().join("right")
    }

    fn run(&self, rels: &[&str], direction: SyncDirection) -> Ended {
        Rig::new().run(sync_of(&self.left(), &self.right(), rels, direction))
    }
}

pub(super) fn is_done(ended: &Ended, skipped: u32) {
    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(ended.skipped, skipped);
}

#[test]
fn a_new_file_and_a_newer_one_are_copied_with_their_times_and_the_folders_they_need() {
    let pair = Pair::new();
    file(&pair.left().join("new.txt"), b"new", 100);
    file(&pair.left().join("deep/er/inside.txt"), b"inside", 100);
    file(&pair.left().join("newer.txt"), b"fresh", 300);
    file(&pair.right().join("newer.txt"), b"stale", 200);

    let ended = pair.run(&["new.txt", "deep/er/inside.txt", "newer.txt"], ToRight);

    is_done(&ended, 0);
    assert_eq!(read(&pair.right().join("new.txt")), b"new");
    assert_eq!(read(&pair.right().join("deep/er/inside.txt")), b"inside");
    assert_eq!(read(&pair.right().join("newer.txt")), b"fresh");
    assert_eq!(modified(&pair.right().join("newer.txt")), moment(300));
    assert_eq!(modified(&pair.right().join("deep/er/inside.txt")), moment(100));
    assert!(has_no_partial_files(pair.root.path()));
}

#[test]
fn the_other_direction_copies_from_the_right_to_the_left() {
    let pair = Pair::new();
    file(&pair.right().join("only-right.txt"), b"right", 100);
    file(&pair.right().join("both.txt"), b"fresh", 300);
    file(&pair.left().join("both.txt"), b"stale", 200);

    let ended = pair.run(&["only-right.txt", "both.txt"], ToLeft);

    is_done(&ended, 0);
    assert_eq!(read(&pair.left().join("only-right.txt")), b"right");
    assert_eq!(read(&pair.left().join("both.txt")), b"fresh");
    assert_eq!(
        read(&pair.right().join("both.txt")),
        b"fresh",
        "the source is left alone"
    );
}

#[test]
fn the_job_is_a_sync_and_names_nothing_but_the_items_it_copies() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    file(&pair.right().join("extra.txt"), b"extra", 100);
    let rig = Rig::new();

    let id = rig.start(sync_of(&pair.left(), &pair.right(), &["a.txt"], ToRight));
    let mut rig = rig;
    let ended = rig.wait_ended(id);

    is_done(&ended, 0);
    // Nothing is deleted: what only the target has stays.
    assert_eq!(tree(&pair.right()), ["a.txt", "extra.txt"]);
    assert!(rig.seen.iter().any(|event| matches!(
        event,
        crate::model::OpsEvent::Added { job } if job.kind == OpKind::Sync
    )));
}

#[test]
fn a_source_that_changed_since_the_scan_is_skipped_and_counted() {
    let pair = Pair::new();
    file(&pair.left().join("size.txt"), b"one", 100);
    file(&pair.left().join("time.txt"), b"one", 100);
    file(&pair.left().join("gone.txt"), b"one", 100);
    file(&pair.left().join("same.txt"), b"one", 100);
    let request = sync_of(
        &pair.left(),
        &pair.right(),
        &["size.txt", "time.txt", "gone.txt", "same.txt"],
        ToRight,
    );

    file(&pair.left().join("size.txt"), b"longer", 100);
    set_times(&pair.left().join("time.txt"), moment(101));
    fs::remove_file(pair.left().join("gone.txt")).expect("remove");
    let ended = Rig::new().run(request);

    is_done(&ended, 3);
    assert_eq!(tree(&pair.right()), ["same.txt"]);
}

#[test]
fn a_time_that_differs_by_a_millisecond_is_a_change() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"one", 100);
    let request = sync_of(&pair.left(), &pair.right(), &["a.txt"], ToRight);

    set_times(&pair.left().join("a.txt"), moment(100) + Duration::from_millis(1));
    let ended = Rig::new().run(request);

    is_done(&ended, 1);
    assert!(tree(&pair.right()).is_empty());
}

#[test]
fn a_target_that_changed_or_appeared_since_the_scan_is_not_overwritten() {
    let pair = Pair::new();
    file(&pair.left().join("edited.txt"), b"from left", 300);
    file(&pair.right().join("edited.txt"), b"old", 200);
    file(&pair.left().join("appeared.txt"), b"from left", 300);
    file(&pair.left().join("removed.txt"), b"from left", 300);
    file(&pair.right().join("removed.txt"), b"old", 200);
    file(&pair.left().join("untouched.txt"), b"from left", 300);
    let request = sync_of(
        &pair.left(),
        &pair.right(),
        &["edited.txt", "appeared.txt", "removed.txt", "untouched.txt"],
        ToRight,
    );

    file(&pair.right().join("edited.txt"), b"edited meanwhile", 250);
    file(&pair.right().join("appeared.txt"), b"appeared meanwhile", 250);
    fs::remove_file(pair.right().join("removed.txt")).expect("remove");
    let ended = Rig::new().run(request);

    is_done(&ended, 3);
    assert_eq!(read(&pair.right().join("edited.txt")), b"edited meanwhile");
    assert_eq!(read(&pair.right().join("appeared.txt")), b"appeared meanwhile");
    assert!(!pair.right().join("removed.txt").exists());
    assert_eq!(read(&pair.right().join("untouched.txt")), b"from left");
}

#[test]
fn a_folder_that_only_one_side_has_is_copied_whole() {
    let pair = Pair::new();
    file(&pair.left().join("tree/a.txt"), b"a", 100);
    file(&pair.left().join("tree/sub/b.txt"), b"b", 200);
    file(&pair.left().join("tree/.hidden"), b"h", 300);
    fs::create_dir_all(pair.left().join("tree/empty")).expect("empty");

    let ended = pair.run(&["tree"], ToRight);

    is_done(&ended, 0);
    assert_eq!(
        tree(&pair.right()),
        [
            "tree/",
            "tree/.hidden",
            "tree/a.txt",
            "tree/empty/",
            "tree/sub/",
            "tree/sub/b.txt"
        ]
    );
    assert_eq!(modified(&pair.right().join("tree/sub/b.txt")), moment(200));
}

#[test]
fn a_folder_that_appeared_at_the_target_since_the_scan_is_skipped() {
    let pair = Pair::new();
    file(&pair.left().join("tree/a.txt"), b"a", 100);
    let request = sync_of(&pair.left(), &pair.right(), &["tree"], ToRight);

    fs::create_dir_all(pair.right().join("tree")).expect("appeared");
    let ended = Rig::new().run(request);

    is_done(&ended, 1);
    assert_eq!(tree(&pair.right()), ["tree/"]);
}

#[test]
fn a_file_at_a_folders_place_is_never_replaced_by_a_folder() {
    let pair = Pair::new();
    file(&pair.left().join("tree/a.txt"), b"a", 100);
    let request = sync_of(&pair.left(), &pair.right(), &["tree"], ToRight);

    file(&pair.right().join("tree"), b"a file now", 100);
    let ended = Rig::new().run(request);

    is_done(&ended, 1);
    assert_eq!(read(&pair.right().join("tree")), b"a file now");
}

#[test]
fn a_cancel_before_the_copy_copies_nothing() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    let mut rig = Rig::paused_at_start();

    let job = rig.start(sync_of(&pair.left(), &pair.right(), &["a.txt"], ToRight));
    let ended = rig.cancel_when_paused(job);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert!(tree(&pair.right()).is_empty());
}

#[test]
fn two_folders_that_are_one_or_nested_fail_the_job_before_anything_is_copied() {
    let pair = Pair::new();
    file(&pair.left().join("a.txt"), b"a", 100);
    fs::create_dir_all(pair.left().join("inner")).expect("inner");

    for (left, right) in [
        (pair.left(), pair.left()),
        (pair.left(), pair.left().join("inner")),
        (pair.left().join("inner"), pair.left()),
    ] {
        let request = with_plans(
            &left,
            &right,
            &["a.txt"],
            vec![SyncPlan {
                direction: ToRight,
                source: seen(&pair.left().join("a.txt")).expect("source"),
                target: None,
            }],
        );

        let ended = Rig::new().run(request);

        assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    }
    assert_eq!(tree(&pair.left()), ["a.txt", "inner/"]);
}

#[cfg(unix)]
mod links {
    use std::fs;
    use std::os::unix::fs::symlink;

    use pretty_assertions::assert_eq;

    use crate::model::Outcome;

    use super::super::testing::{Rig, make_fifo, temp, tree};
    use super::SyncDirection::ToRight;
    use super::{Pair, file, is_done, plan, sync_of, with_plans};

    #[test]
    fn a_link_is_never_followed_as_a_source_and_is_skipped_and_counted() {
        let pair = Pair::new();
        let outside = temp();
        file(&outside.path().join("secret.txt"), b"secret", 100);
        file(&pair.left().join("real.txt"), b"x", 100);
        let plan_of_file = plan(&pair.left(), &pair.right(), "real.txt", ToRight);
        // The scan saw a file there; a link took its place.
        fs::remove_file(pair.left().join("real.txt")).expect("remove");
        symlink(outside.path().join("secret.txt"), pair.left().join("real.txt")).expect("link");

        let ended = Rig::new().run(with_plans(
            &pair.left(),
            &pair.right(),
            &["real.txt"],
            vec![plan_of_file],
        ));

        is_done(&ended, 1);
        assert!(tree(&pair.right()).is_empty());
    }

    #[test]
    fn a_link_to_a_folder_on_the_way_to_the_source_is_not_entered() {
        let pair = Pair::new();
        let outside = temp();
        file(&outside.path().join("f.txt"), b"outside", 100);
        file(&pair.left().join("d/f.txt"), b"inside", 100);
        let request = sync_of(&pair.left(), &pair.right(), &["d/f.txt"], ToRight);
        fs::remove_dir_all(pair.left().join("d")).expect("remove");
        symlink(outside.path(), pair.left().join("d")).expect("link");

        let ended = Rig::new().run(request);

        is_done(&ended, 1);
        assert!(tree(&pair.right()).is_empty());
    }

    #[test]
    fn a_link_to_a_folder_on_the_way_to_the_target_is_never_written_through() {
        let pair = Pair::new();
        let outside = temp();
        file(&pair.left().join("d/f.txt"), b"inside", 100);
        fs::create_dir_all(pair.right().join("d")).expect("folder");
        let request = sync_of(&pair.left(), &pair.right(), &["d/f.txt"], ToRight);
        fs::remove_dir(pair.right().join("d")).expect("remove");
        symlink(outside.path(), pair.right().join("d")).expect("link");

        let ended = Rig::new().run(request);

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
        assert!(tree(outside.path()).is_empty());
    }

    #[test]
    fn a_link_inside_a_folder_that_is_copied_whole_is_copied_as_a_link() {
        let pair = Pair::new();
        let outside = temp();
        file(&outside.path().join("secret.txt"), b"secret", 100);
        file(&pair.left().join("tree/a.txt"), b"a", 100);
        symlink(outside.path(), pair.left().join("tree/to-folder")).expect("link");
        symlink(outside.path().join("secret.txt"), pair.left().join("tree/to-file")).expect("link");

        let ended = pair.run(&["tree"], ToRight);

        is_done(&ended, 0);
        let copied = pair.right().join("tree");
        assert_eq!(fs::read_link(copied.join("to-folder")).expect("a link"), outside.path());
        assert_eq!(
            fs::read_link(copied.join("to-file")).expect("a link"),
            outside.path().join("secret.txt")
        );
        assert!(
            fs::symlink_metadata(copied.join("to-folder"))
                .expect("meta")
                .is_symlink()
        );
    }

    #[test]
    fn a_special_file_where_the_scan_saw_a_file_is_skipped() {
        let pair = Pair::new();
        file(&pair.left().join("pipe"), b"x", 100);
        let plan_of_file = plan(&pair.left(), &pair.right(), "pipe", ToRight);
        fs::remove_file(pair.left().join("pipe")).expect("remove");
        make_fifo(&pair.left().join("pipe"));

        let ended = Rig::new().run(with_plans(&pair.left(), &pair.right(), &["pipe"], vec![plan_of_file]));

        is_done(&ended, 1);
        assert!(tree(&pair.right()).is_empty());
    }
}

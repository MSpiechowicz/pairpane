//! Synchronize jobs whose source is an archive: the entries are extracted into the other folder with the same checks as a copy
//! between two folders.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::archive::testing::MOMENT;
use crate::model::{EntryKind, OpsEvent, SyncDirection, SyncFacts};

use super::extract_tests::{add, fixture, scene, tar_by};
use super::hooks::Hooks;
use super::sync::facts;
use super::sync_tests::{file, is_done, moment};
use super::testing::{Ended, Rig, modified, read, task, tree};
use super::{ArchiveJob, SyncPlan, SyncRoot, Task};

fn from_archive() -> SyncRoot {
    SyncRoot::Archive {
        name: "x.tar".to_owned(),
        inner: String::new(),
        source: ArchiveJob::default(),
    }
}

fn entry(kind: EntryKind, size: u64) -> SyncFacts {
    SyncFacts {
        kind,
        size,
        modified: Some(MOMENT),
    }
}

/// The plan of `rel` going from the archive (the left side) into `root/dst`, where `target` is what the scan saw there.
fn plan(source: SyncFacts, target: Option<SyncFacts>) -> SyncPlan {
    SyncPlan {
        direction: SyncDirection::ToRight,
        source,
        target,
    }
}

/// The synchronization of `rels` of the archive `x.tar` in `root/src` into `root/dst`.
fn job(root: &Path, rels: &[&str], plans: Vec<SyncPlan>) -> super::Request {
    task(
        &root.join("src"),
        rels,
        &root.join("dst"),
        Task::Sync {
            left: from_archive(),
            right: SyncRoot::Folder,
            plans,
            removals: Vec::new(),
            permanent: false,
        },
    )
}

fn run(root: &Path, rels: &[&str], plans: Vec<SyncPlan>) -> Ended {
    fs::create_dir_all(root.join("dst")).expect("dst");

    Rig::new().run(job(root, rels, plans))
}

#[test]
fn files_of_an_archive_are_extracted_with_their_times_and_the_folders_they_need() {
    let root = scene("x.tar", &fixture());

    let ended = run(
        root.path(),
        &["a.txt", "docs/b.txt"],
        vec![
            plan(entry(EntryKind::File, 5), None),
            plan(entry(EntryKind::File, 4), None),
        ],
    );

    is_done(&ended, 0);
    assert_eq!(tree(&root.path().join("dst")), ["a.txt", "docs/", "docs/b.txt"]);
    assert_eq!(read(&root.path().join("dst/docs/b.txt")), b"beta");
    assert_eq!(
        modified(&root.path().join("dst/a.txt")),
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(MOMENT as u64)
    );
}

#[test]
fn a_folder_of_an_archive_is_extracted_whole() {
    let root = scene("x.tar", &fixture());

    let ended = run(root.path(), &["docs"], vec![plan(entry(EntryKind::Dir, 0), None)]);

    is_done(&ended, 0);
    assert_eq!(
        tree(&root.path().join("dst")),
        ["docs/", "docs/b.txt", "docs/deep/", "docs/deep/c.txt"]
    );
}

#[test]
fn an_entry_that_is_not_what_the_scan_saw_is_skipped_and_counted() {
    let root = scene("x.tar", &fixture());

    let ended = run(
        root.path(),
        &["a.txt", "docs/b.txt", "docs/deep/c.txt", "missing.txt"],
        vec![
            // A size that is not the entry's, a time that is not, a kind that is not, an entry that is not there.
            plan(entry(EntryKind::File, 6), None),
            plan(
                SyncFacts {
                    modified: Some(MOMENT + 1),
                    ..entry(EntryKind::File, 4)
                },
                None,
            ),
            plan(entry(EntryKind::Dir, 0), None),
            plan(entry(EntryKind::File, 1), None),
        ],
    );

    is_done(&ended, 4);
    assert!(tree(&root.path().join("dst")).is_empty());
}

#[test]
fn a_target_that_is_not_what_the_scan_saw_is_left_alone_and_one_that_is_replaced() {
    let root = scene("x.tar", &fixture());
    file(&root.path().join("dst/a.txt"), b"edited meanwhile", 500);
    file(&root.path().join("dst/docs/b.txt"), b"old", 200);
    let old = facts(&fs::symlink_metadata(root.path().join("dst/docs/b.txt")).expect("meta"));
    let scene_of_a = SyncFacts {
        modified: Some(
            i64::try_from(
                moment(100)
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("time")
                    .as_millis(),
            )
            .expect("millis"),
        ),
        ..entry(EntryKind::File, 3)
    };

    let ended = run(
        root.path(),
        &["a.txt", "docs/b.txt"],
        vec![
            // The scan saw another file at `a.txt` than the one that is there now.
            plan(entry(EntryKind::File, 5), Some(scene_of_a)),
            plan(entry(EntryKind::File, 4), Some(old)),
        ],
    );

    is_done(&ended, 1);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"edited meanwhile");
    assert_eq!(read(&root.path().join("dst/docs/b.txt")), b"beta");
}

#[test]
fn a_folder_that_appeared_at_the_target_since_the_scan_is_skipped() {
    let root = scene("x.tar", &fixture());
    file(&root.path().join("dst/docs/mine.txt"), b"mine", 200);

    let ended = run(root.path(), &["docs"], vec![plan(entry(EntryKind::Dir, 0), None)]);

    is_done(&ended, 1);
    assert_eq!(tree(&root.path().join("dst")), ["docs/", "docs/mine.txt"]);
}

#[test]
fn an_entry_never_replaces_the_archive_file_itself() {
    let bytes = tar_by(|builder| add(builder, "x.tar", 0o644, b"not the archive"));
    let root = scene("x.tar", &bytes);
    let archive = root.path().join("src/x.tar");
    let target = facts(&fs::symlink_metadata(&archive).expect("meta"));

    // The archive's folder is the target too: the entry `x.tar` would land on the archive.
    let request = task(
        &root.path().join("src"),
        &["x.tar"],
        &root.path().join("src"),
        Task::Sync {
            left: from_archive(),
            right: SyncRoot::Folder,
            plans: vec![plan(entry(EntryKind::File, 15), Some(target))],
            removals: Vec::new(),
            permanent: false,
        },
    );
    let ended = Rig::new().run(request);

    is_done(&ended, 1);
    assert_eq!(read(&archive), bytes);
}

#[test]
fn a_synchronization_the_other_way_takes_the_archive_from_the_right() {
    let root = scene("x.tar", &fixture());
    fs::create_dir_all(root.path().join("dst")).expect("dst");
    let request = task(
        &root.path().join("src"),
        &["a.txt"],
        &root.path().join("dst"),
        Task::Sync {
            left: SyncRoot::Folder,
            right: from_archive(),
            plans: vec![SyncPlan {
                direction: SyncDirection::ToLeft,
                source: entry(EntryKind::File, 5),
                target: None,
            }],
            removals: Vec::new(),
            permanent: false,
        },
    );

    let ended = Rig::new().run(request);

    is_done(&ended, 0);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"alpha");
}

/// A rig whose sink, once the file `current` is being written, puts a file with the contents `mine` at `mine`.
fn rig_putting_when(current: &'static str, mine: std::path::PathBuf) -> Rig {
    let appeared = std::sync::atomic::AtomicBool::new(false);
    let hooks = Hooks {
        every_chunk: true,
        ..Hooks::default()
    };

    Rig::with(hooks, move |_, event| {
        if let OpsEvent::Progress {
            current: Some(writing), ..
        } = event
            && writing == current
            && !appeared.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            file(&mine, b"mine", 200);
        }
    })
}

#[test]
fn a_folder_that_appears_during_the_extraction_is_left_alone_and_counted_once() {
    let root = scene("x.tar", &fixture());
    fs::create_dir_all(root.path().join("dst")).expect("dst");
    let request = job(
        root.path(),
        &["a.txt", "docs"],
        vec![
            plan(entry(EntryKind::File, 5), None),
            plan(entry(EntryKind::Dir, 0), None),
        ],
    );

    // While `a.txt` is written the user makes `docs` and puts a file in it that an entry of the archive has the name of.
    let mut rig = rig_putting_when("a.txt", root.path().join("dst/docs/b.txt"));
    let ended = rig.run(request);

    is_done(&ended, 1);
    assert_eq!(tree(&root.path().join("dst")), ["a.txt", "docs/", "docs/b.txt"]);
    assert_eq!(read(&root.path().join("dst/docs/b.txt")), b"mine");
    assert_eq!(read(&root.path().join("dst/a.txt")), b"alpha");
}

#[test]
fn a_file_that_appears_inside_an_extracted_folder_is_kept() {
    let root = scene("x.tar", &fixture());
    fs::create_dir_all(root.path().join("dst")).expect("dst");
    let request = job(root.path(), &["docs"], vec![plan(entry(EntryKind::Dir, 0), None)]);

    // While `docs/b.txt` is written the user puts a file at the name of `docs/deep/c.txt`.
    let mut rig = rig_putting_when("docs/b.txt", root.path().join("dst/docs/deep/c.txt"));
    let ended = rig.run(request);

    is_done(&ended, 1);
    assert_eq!(read(&root.path().join("dst/docs/deep/c.txt")), b"mine");
    assert_eq!(read(&root.path().join("dst/docs/b.txt")), b"beta");
}

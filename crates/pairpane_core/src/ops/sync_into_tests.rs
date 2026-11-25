//! Synchronize jobs that write to an archive: copies into it and removals from it, all in one rewrite, each only while the
//! source and the archive are what the scan saw.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use std::sync::atomic::AtomicBool;

use crate::archive::rewrite::testing::seen;
use crate::archive::testing::MOMENT;
use crate::archive::{Fingerprint, Input, build_index};
use crate::model::{ArchiveFormat, EntryKind, Outcome, PaneId, SyncDirection, SyncFacts};

use super::extract_tests::{add, fixture, tar_by};
use super::hooks::Hooks;
use super::sync::facts;
use super::sync_tests::{file, is_done};
use super::testing::{Ended, Rig, task, temp};
use super::{ArchiveJob, SyncPlan, SyncRemove, SyncRoot, Task};

/// A scene: the archive `arch/x.tar` (the right side) and the folder `disk` (the left side).
fn scene(bytes: &[u8]) -> TempDir {
    let root = temp();
    fs::create_dir_all(root.path().join("arch")).expect("arch");
    fs::create_dir_all(root.path().join("disk")).expect("disk");
    fs::write(root.path().join("arch/x.tar"), bytes).expect("archive");

    root
}

fn entry(kind: EntryKind, size: u64) -> SyncFacts {
    SyncFacts {
        kind,
        size,
        modified: if kind == EntryKind::Dir { None } else { Some(MOMENT) },
    }
}

fn on_disk(path: &Path) -> SyncFacts {
    facts(&fs::symlink_metadata(path).expect("there"))
}

/// A copy of the job: its path and its plan.
type CopyRow = (String, SyncPlan);

/// A removal of the job: its path and what the scan saw.
type RemoveRow = (String, SyncRemove);

/// The copy of `rel` from the disk into the archive, which the scan saw holding `target`.
fn copy(root: &Path, rel: &str, target: Option<SyncFacts>) -> (String, SyncPlan) {
    let plan = SyncPlan {
        direction: SyncDirection::ToRight,
        source: on_disk(&root.join("disk").join(rel)),
        target,
    };

    (rel.to_owned(), plan)
}

/// The removal of `rel` from the archive, where the scan saw `seen` (and for a folder `files` files of `bytes`).
fn removal(rel: &str, seen: SyncFacts, (files, bytes): (u64, u64)) -> (String, SyncRemove) {
    let removal = SyncRemove {
        side: PaneId::Right,
        seen,
        files,
        bytes,
        scanned: i64::MAX,
    };

    (rel.to_owned(), removal)
}

/// The job that applies `copies` and `removals` to the folder `inner` of the archive, with `hooks`.
fn run(root: &Path, inner: &str, (copies, removals): (&[CopyRow], &[RemoveRow]), hooks: Hooks) -> Ended {
    let names: Vec<&str> = copies
        .iter()
        .map(|(rel, _)| rel.as_str())
        .chain(removals.iter().map(|(rel, _)| rel.as_str()))
        .collect();
    let request = task(
        &root.join("arch"),
        &names,
        &root.join("disk"),
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Archive {
                name: "x.tar".to_owned(),
                inner: inner.to_owned(),
                source: listed(&root.join("arch/x.tar")),
            },
            plans: copies.iter().map(|(_, plan)| *plan).collect(),
            removals: removals.iter().map(|(_, removal)| *removal).collect(),
            permanent: false,
            log: super::SyncLog::default(),
        },
    );

    Rig::with(hooks, |_, _| {}).run(request)
}

/// What the caller brings of the archive at `path`: its index and the state of the file now.
fn listed(path: &Path) -> ArchiveJob {
    let file = fs::File::open(path).expect("the archive");
    let input = Input {
        file: &file,
        path: Some(path),
    };
    let index = build_index(ArchiveFormat::Tar, &input, &AtomicBool::new(false), None).expect("an index");

    ArchiveJob {
        index: Some(Arc::new(index)),
        fingerprint: Some(Fingerprint::of(path, &file.metadata().expect("metadata"))),
        ..ArchiveJob::default()
    }
}

/// The paths of the archive and the contents of its files.
fn contents(root: &Path) -> Vec<(String, Vec<u8>)> {
    let bytes = fs::read(root.join("arch/x.tar")).expect("the archive");

    seen(&bytes, ArchiveFormat::Tar, None)
        .into_iter()
        .map(|entry| (entry.path, entry.content))
        .collect()
}

fn pairs(items: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
    items
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.to_vec()))
        .collect()
}

#[test]
fn copies_into_an_archive_and_removals_from_it_are_one_rewrite() {
    let root = scene(&fixture());
    let disk = root.path().join("disk");
    file(&disk.join("new.txt"), b"new", 100);
    file(&disk.join("a.txt"), b"alpha, newer", 300);
    file(&disk.join("fresh/inside.txt"), b"inside", 100);
    let copies = [
        copy(root.path(), "new.txt", None),
        copy(root.path(), "a.txt", Some(entry(EntryKind::File, 5))),
        copy(root.path(), "fresh", None),
    ];
    let removals = [removal("docs", entry(EntryKind::Dir, 0), (2, 9))];

    let ended = run(root.path(), "", (&copies, &removals), Hooks::default());

    is_done(&ended, 0);
    assert_eq!(
        contents(root.path()),
        pairs(&[
            ("a.txt", b"alpha, newer"),
            ("fresh", b""),
            ("fresh/inside.txt", b"inside"),
            ("new.txt", b"new"),
        ])
    );
}

#[test]
fn into_a_folder_of_the_archive_the_paths_are_below_it() {
    let root = scene(&fixture());
    file(&root.path().join("disk/b.txt"), b"beta, newer", 300);
    let copies = [copy(root.path(), "b.txt", Some(entry(EntryKind::File, 4)))];
    let removals = [removal("deep/c.txt", entry(EntryKind::File, 5), (0, 0))];

    let ended = run(root.path(), "docs", (&copies, &removals), Hooks::default());

    is_done(&ended, 0);
    assert_eq!(
        contents(root.path()),
        pairs(&[
            ("a.txt", b"alpha"),
            ("docs", b""),
            ("docs/b.txt", b"beta, newer"),
            ("docs/deep", b""),
        ])
    );
}

#[test]
fn what_is_not_as_the_scan_saw_it_is_left_alone_and_nothing_changed_keeps_the_archive() {
    let root = scene(&fixture());
    let disk = root.path().join("disk");
    file(&disk.join("a.txt"), b"alpha, newer", 300);
    file(&disk.join("gone.txt"), b"gone", 100);
    file(&disk.join("edited.txt"), b"edited", 100);
    file(&disk.join("docs/b.txt"), b"beta, mine", 100);
    let copies = [
        // The archive's file is not the size the scan saw, a file the scan saw free is taken, the source is gone or edited.
        copy(root.path(), "a.txt", Some(entry(EntryKind::File, 6))),
        copy(root.path(), "docs/b.txt", None),
        copy(root.path(), "gone.txt", None),
        copy(root.path(), "edited.txt", None),
    ];
    fs::remove_file(disk.join("gone.txt")).expect("removed");
    file(&disk.join("edited.txt"), b"edited, longer", 100);
    // A folder that holds other than the scan counted.
    let removals = [removal("docs/deep", entry(EntryKind::Dir, 0), (2, 9))];
    let before = fs::read(root.path().join("arch/x.tar")).expect("the archive");

    let ended = run(root.path(), "", (&copies, &removals), Hooks::default());

    is_done(&ended, 5);
    assert_eq!(
        fs::read(root.path().join("arch/x.tar")).expect("the archive"),
        before,
        "not rewritten"
    );
}

#[test]
fn a_file_of_the_archive_on_the_way_fails_the_item() {
    let root = scene(&fixture());
    file(&root.path().join("disk/a.txt/inside.txt"), b"inside", 100);
    let copies = [copy(root.path(), "a.txt/inside.txt", None)];

    let ended = run(root.path(), "", (&copies, &[]), Hooks::default());

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(contents(root.path()).len(), 5, "the archive is as it was");
}

#[test]
fn an_archive_changed_while_the_new_one_is_written_is_not_replaced() {
    let root = scene(&fixture());
    file(&root.path().join("disk/new.txt"), b"new", 100);
    let copies = [copy(root.path(), "new.txt", None)];
    let archive = root.path().join("arch/x.tar");
    let changed = tar_by(|builder| add(builder, "other.txt", 0o644, b"someone else's"));
    let (target, bytes) = (archive.clone(), changed.clone());
    let hooks = Hooks {
        after_temp: Some(Arc::new(move |_| fs::write(&target, &bytes).expect("written"))),
        ..Hooks::default()
    };

    let ended = run(root.path(), "", (&copies, &[]), hooks);

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(fs::read(&archive).expect("the archive"), changed);
}

/// The job that copies `rel` of the folder `disk` (where the scan saw it as it is now) into the archive `arch/x.tar`.
fn copy_from(root: &Path, disk: &Path, rel: &str) -> super::Request {
    task(
        &root.join("arch"),
        &[rel],
        disk,
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Archive {
                name: "x.tar".to_owned(),
                inner: String::new(),
                source: listed(&root.join("arch/x.tar")),
            },
            plans: vec![SyncPlan {
                direction: SyncDirection::ToRight,
                source: on_disk(&disk.join(rel)),
                target: None,
            }],
            removals: Vec::new(),
            permanent: false,
            log: super::SyncLog::default(),
        },
    )
}

#[test]
fn the_archive_or_a_folder_that_holds_it_is_never_copied_into_it() {
    let root = scene(&fixture());
    let before = fs::read(root.path().join("arch/x.tar")).expect("the archive");

    // The archive's own folder compared with it: its file is a row; and its parent compared: its folder is a row.
    let itself = Rig::new().run(copy_from(root.path(), &root.path().join("arch"), "x.tar"));
    let holder = Rig::new().run(copy_from(root.path(), root.path(), "arch"));

    is_done(&itself, 1);
    is_done(&holder, 1);
    assert_eq!(fs::read(root.path().join("arch/x.tar")).expect("the archive"), before);
}

/// The job that removes the file `a.txt`, as the scan saw it, from the archive.
fn remove_a_txt(root: &Path) -> Ended {
    let removals = [removal("a.txt", entry(EntryKind::File, 5), (0, 0))];

    run(root, "", (&[], &removals), Hooks::default())
}

#[test]
fn removals_alone_rewrite_the_archive_without_reading_the_folder() {
    let root = scene(&fixture());
    fs::remove_dir(root.path().join("disk")).expect("no folder to read");
    fs::create_dir(root.path().join("disk")).expect("an empty one");
    let ended = remove_a_txt(root.path());

    is_done(&ended, 0);
    assert!(contents(root.path()).iter().all(|(path, _)| path != "a.txt"));
}

#[test]
fn one_job_copies_out_of_the_archive_and_into_it() {
    let root = scene(&fixture());
    file(&root.path().join("disk/new.txt"), b"new", 100);
    let out = SyncPlan {
        direction: SyncDirection::ToLeft,
        source: entry(EntryKind::File, 5),
        target: None,
    };
    let copies = [("a.txt".to_owned(), out), copy(root.path(), "new.txt", None)];

    let ended = run(root.path(), "", (&copies, &[]), Hooks::default());

    is_done(&ended, 0);
    assert_eq!(fs::read(root.path().join("disk/a.txt")).expect("extracted"), b"alpha");
    assert!(contents(root.path()).contains(&("new.txt".to_owned(), b"new".to_vec())));
}

#[test]
fn an_entry_whose_name_the_source_folder_has_by_now_stays_in_the_archive() {
    // The scan saw `a.txt` only in the archive; the folder mirrored from has one by now (#75).
    let root = scene(&fixture());
    file(&root.path().join("disk/a.txt"), b"made since", 100);
    let ended = remove_a_txt(root.path());

    is_done(&ended, 1);
    assert!(contents(root.path()).iter().any(|(path, _)| path == "a.txt"));
}

/// The mirror from the archive to the folder that removes `only.txt` from the folder, with what the caller read of the
/// archive.
fn mirror_from_archive(root: &Path, source: ArchiveJob) -> Ended {
    let removal = SyncRemove {
        side: PaneId::Right,
        seen: on_disk(&root.join("disk/only.txt")),
        files: 0,
        bytes: 0,
        scanned: i64::MAX,
    };
    let request = task(
        &root.join("arch"),
        &["only.txt"],
        &root.join("disk"),
        Task::Sync {
            left: SyncRoot::Archive {
                name: "x.tar".to_owned(),
                inner: String::new(),
                source,
            },
            right: SyncRoot::Folder,
            plans: Vec::new(),
            removals: vec![removal],
            permanent: true,
            log: super::SyncLog::default(),
        },
    );

    Rig::new().run(request)
}

#[test]
fn a_mirror_from_an_archive_removes_from_the_folder_while_the_archive_is_the_one_compared() {
    let root = scene(&fixture());
    file(&root.path().join("disk/only.txt"), b"only here", 100);

    let ended = mirror_from_archive(root.path(), listed(&root.path().join("arch/x.tar")));

    is_done(&ended, 0);
    assert!(!root.path().join("disk/only.txt").exists());
}

#[test]
fn a_mirror_from_an_archive_that_changed_since_the_comparison_removes_nothing() {
    // The archive may hold `only.txt` by now: it is not read for the removals, so nothing is removed (#75).
    let root = scene(&fixture());
    file(&root.path().join("disk/only.txt"), b"only here", 100);
    let compared = listed(&root.path().join("arch/x.tar"));
    let mut grown = fs::read(root.path().join("arch/x.tar")).expect("the archive");
    grown.extend_from_slice(&[0; 512]);
    fs::write(root.path().join("arch/x.tar"), grown).expect("the archive changes");

    let ended = mirror_from_archive(root.path(), compared);

    is_done(&ended, 1);
    assert!(root.path().join("disk/only.txt").exists());

    // What the caller did not say of the archive is not trusted either.
    let ended = mirror_from_archive(root.path(), ArchiveJob::default());
    is_done(&ended, 1);
    assert!(root.path().join("disk/only.txt").exists());
}

#[test]
fn a_job_that_writes_into_the_archive_still_removes_from_the_folder() {
    // One job copies `new.txt` into the archive and removes `only.txt`, which the archive did not hold, from the folder:
    // its own rewrite of the archive is no reason to keep `only.txt` (#75).
    let root = scene(&fixture());
    file(&root.path().join("disk/new.txt"), b"new", 100);
    file(&root.path().join("disk/only.txt"), b"only here", 100);
    let (new, into) = copy(root.path(), "new.txt", None);
    let removal = SyncRemove {
        side: PaneId::Left,
        seen: on_disk(&root.path().join("disk/only.txt")),
        files: 0,
        bytes: 0,
        scanned: i64::MAX,
    };
    let request = task(
        &root.path().join("arch"),
        &[new.as_str(), "only.txt"],
        &root.path().join("disk"),
        Task::Sync {
            left: SyncRoot::Folder,
            right: SyncRoot::Archive {
                name: "x.tar".to_owned(),
                inner: String::new(),
                source: listed(&root.path().join("arch/x.tar")),
            },
            plans: vec![into],
            removals: vec![removal],
            permanent: true,
            log: super::SyncLog::default(),
        },
    );

    let ended = Rig::new().run(request);

    is_done(&ended, 0);
    assert!(contents(root.path()).contains(&("new.txt".to_owned(), b"new".to_vec())));
    assert!(!root.path().join("disk/only.txt").exists());
}

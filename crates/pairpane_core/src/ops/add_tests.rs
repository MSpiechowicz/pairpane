//! Add jobs on real folders: what each format reads back as after files and folders were added at the root and into an inner
//! folder, the encryption of the new entries, the mode of the archive, the progress, and what a job that has nothing to add
//! or lacks the password leaves behind.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::archive::rewrite::testing::{SECRET, TARS, archive};
use crate::archive::write::testing::Item;
use crate::archive::{Password, PasswordFault, fault_text};
use crate::model::{ArchiveFormat, IpcErrorCode, JobPhase, Outcome};

use super::ArchiveJob;
use super::hooks::Hooks;
use super::pack_tests::{Seen, expected, last_progress, read_back, scene};
use super::testing::{Rig, has_no_partial_files, phases, put, read, task, tree};
use super::{Request, Task};

/// The archives the tests change, by name: the format, the password and whether the 7z's names are encrypted too.
pub(super) const KINDS: [(&str, ArchiveFormat, Option<&str>, bool); 10] = [
    ("a.zip", ArchiveFormat::Zip, None, false),
    ("b.zip", ArchiveFormat::Zip, Some(SECRET), false),
    ("a.tar", ArchiveFormat::Tar, None, false),
    ("a.tar.gz", ArchiveFormat::TarGz, None, false),
    ("a.tar.xz", ArchiveFormat::TarXz, None, false),
    ("a.tar.zst", ArchiveFormat::TarZst, None, false),
    ("a.tar.bz2", ArchiveFormat::TarBz2, None, false),
    ("a.7z", ArchiveFormat::SevenZ, None, false),
    ("b.7z", ArchiveFormat::SevenZ, Some(SECRET), false),
    ("c.7z", ArchiveFormat::SevenZ, Some(SECRET), true),
];

/// The scene's items as the writers take them.
fn items() -> Vec<Item> {
    expected()
        .into_iter()
        .map(|(path, folder, mode, modified, content)| Item {
            name: path,
            mode: mode.unwrap_or(0o644),
            modified: modified.unwrap_or(0),
            content: (!folder).then_some(content),
        })
        .collect()
}

/// The scene with an archive `name` in `dst` (made as `format` with `password`), and the files to add in `add`.
pub(super) fn scene_with(name: &str, format: ArchiveFormat, password: Option<&str>, names: bool) -> TempDir {
    let root = scene();
    put(
        root.path(),
        &format!("dst/{name}"),
        &archive(format, &items(), password, names),
    );
    put(root.path(), "add/new.txt", b"new file");
    put(root.path(), "add/tree/x.txt", b"xxx");
    put(root.path(), "add/tree/sub/y.txt", b"yyy");
    fs::create_dir_all(root.path().join("add/tree/empty")).expect("folder");

    root
}

pub(super) fn password(text: Option<&str>) -> Option<Arc<Password>> {
    text.map(|text| Arc::new(Password::new(text.to_owned())))
}

/// What the job brings of the archive when it knows nothing: it reads the archive itself.
pub(super) fn knows(text: Option<&str>) -> ArchiveJob {
    ArchiveJob {
        index: None,
        fingerprint: None,
        password: password(text),
    }
}

/// The job that adds `names` of `root/add` to the folder `inner` of the archive `name` in `root/dst`.
pub(super) fn add(root: &Path, names: &[&str], name: &str, inner: &str, source: ArchiveJob) -> Request {
    task(
        &root.join("add"),
        names,
        &root.join("dst"),
        Task::AddToArchive {
            name: name.to_owned(),
            inner: inner.to_owned(),
            source,
        },
    )
}

/// What the archive `name` of the scene reads back as.
pub(super) fn archived(root: &TempDir, name: &str, format: ArchiveFormat, text: Option<&str>) -> Vec<Seen> {
    let secret = password(text);

    read_back(&root.path().join("dst").join(name), format, secret.as_deref()).expect("an archive that reads back")
}

/// The paths of `entries`, sorted.
pub(super) fn paths(entries: &[Seen]) -> Vec<&str> {
    let mut paths: Vec<&str> = entries.iter().map(|seen| seen.0.as_str()).collect();
    paths.sort_unstable();

    paths
}

/// The entries of `entries` that the scene's own items are, in the scene's order and as they are.
pub(super) fn old_ones(entries: &[Seen]) -> Vec<Seen> {
    let mut old: Vec<Seen> = entries
        .iter()
        .filter(|seen| expected().iter().any(|item| item.0 == seen.0))
        .cloned()
        .collect();
    old.sort_by(|a, b| a.0.cmp(&b.0));

    old
}

/// The entries that are not the scene's own, as path, whether a folder, and content, sorted.
pub(super) fn new_ones(entries: &[Seen]) -> Vec<(String, bool, Vec<u8>)> {
    let mut new: Vec<(String, bool, Vec<u8>)> = entries
        .iter()
        .filter(|seen| !expected().iter().any(|item| item.0 == seen.0))
        .map(|seen| (seen.0.clone(), seen.1, seen.4.clone()))
        .collect();
    new.sort();

    new
}

/// What the files of `add` read back as under the folder `prefix` of the archive.
fn added_under(prefix: &str) -> Vec<(String, bool, Vec<u8>)> {
    let under = |path: &str| format!("{prefix}{path}");
    let mut added = vec![
        (under("new.txt"), false, b"new file".to_vec()),
        (under("tree"), true, Vec::new()),
        (under("tree/empty"), true, Vec::new()),
        (under("tree/sub"), true, Vec::new()),
        (under("tree/sub/y.txt"), false, b"yyy".to_vec()),
        (under("tree/x.txt"), false, b"xxx".to_vec()),
    ];
    added.sort();

    added
}

fn sorted(mut entries: Vec<Seen>) -> Vec<Seen> {
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    entries
}

#[test]
fn files_and_folders_added_at_the_root_and_into_an_inner_folder_read_back_in_every_format() {
    for (name, format, secret, names) in KINDS {
        for (inner, prefix) in [("", ""), ("docs", "docs/")] {
            let root = scene_with(name, format, secret, names);

            let ended = Rig::new().run(add(root.path(), &["new.txt", "tree"], name, inner, knows(secret)));

            assert_eq!(
                (ended.outcome, ended.skipped, ended.errors.len()),
                (Outcome::Done, 0, 0),
                "{name} {inner}"
            );
            let entries = archived(&root, name, format, secret);
            assert_eq!(
                old_ones(&entries),
                sorted(expected()),
                "{name} {inner}: the old entries are as they were"
            );
            assert_eq!(new_ones(&entries), added_under(prefix), "{name} {inner}");
            assert_eq!(
                tree(&root.path().join("dst")),
                [name],
                "{name}: no temporary file is left"
            );
        }
    }
}

#[test]
fn the_tar_formats_are_all_covered_by_the_table() {
    let covered: Vec<ArchiveFormat> = KINDS.iter().map(|kind| kind.1).collect();

    assert!(TARS.iter().all(|format| covered.contains(format)));
}

#[test]
fn the_archive_keeps_its_permission_bits_and_the_job_reports_the_old_archive_and_the_new_files() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    let path = root.path().join("dst/a.zip");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("mode");
    }
    let before = read(&path).len() as u64;
    let mut rig = Rig::new();

    let job = rig.start(add(root.path(), &["new.txt"], "a.zip", "", knows(None)));
    let ended = rig.wait_ended(job);

    assert_eq!(ended.outcome, Outcome::Done);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o640
        );
    }
    let (done_bytes, total_bytes, done_files, total_files) = last_progress(&rig);
    assert_eq!(done_bytes, total_bytes);
    assert!(
        total_bytes >= 8,
        "the new file's bytes and what is copied of the old archive"
    );
    assert_eq!((done_files, total_files), (2, 2), "the new file and the old archive");
    assert!(phases(&rig).ends_with(&[JobPhase::Scanning, JobPhase::Packing]));
    assert!(read(&path).len() as u64 > before);
}

#[test]
fn an_encrypted_archive_without_its_password_or_with_a_wrong_one_is_refused_and_left_as_it_is() {
    for (name, format, secret, names) in KINDS.iter().filter(|kind| kind.2.is_some()) {
        for (given, fault) in [(None, PasswordFault::Needed), (Some("wrong"), PasswordFault::Wrong)] {
            // A 7z with encrypted names cannot even be listed with a wrong password.
            let root = scene_with(name, *format, *secret, *names);
            let before = read(&root.path().join("dst").join(name));

            let ended = Rig::new().run(add(root.path(), &["new.txt"], name, "", knows(given)));

            let Outcome::Failed { error } = ended.outcome else {
                panic!("{name}: the job must fail: {:?}", ended.outcome);
            };
            assert_eq!(error.code, IpcErrorCode::PasswordRequired, "{name}");
            assert_eq!(error.message, fault_text(fault), "{name}");
            assert_eq!(read(&root.path().join("dst").join(name)), before, "{name}");
            assert!(has_no_partial_files(&root.path().join("dst")), "{name}");
        }
    }
}

#[test]
fn a_job_with_nothing_to_add_fails_and_leaves_the_archive() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    let before = read(&root.path().join("dst/a.zip"));

    let ended = Rig::new().run(add(root.path(), &["gone"], "a.zip", "", knows(None)));

    let Outcome::Failed { error } = ended.outcome else {
        panic!("the job must fail: {:?}", ended.outcome);
    };
    assert_eq!(error.message, "there is nothing to add");
    assert_eq!(ended.errors.len(), 1, "the missing item is reported");
    assert_eq!(read(&root.path().join("dst/a.zip")), before);
}

#[test]
fn a_folder_that_is_not_in_the_archive_fails_the_job() {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);

    let ended = Rig::new().run(add(root.path(), &["new.txt"], "a.zip", "nowhere", knows(None)));

    let Outcome::Failed { error } = ended.outcome else {
        panic!("the job must fail: {:?}", ended.outcome);
    };
    assert_eq!(error.message, "the folder is not in the archive");
}

#[test]
fn what_the_archive_has_counts_against_the_limit_of_entries() {
    // The archive has 7 entries and the items 6 more.
    for (limit, done) in [(12, false), (13, true)] {
        let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
        let before = read(&root.path().join("dst/a.zip"));
        let hooks = Hooks {
            pack_limits: Some((limit, usize::MAX)),
            ..Hooks::default()
        };

        let ended =
            Rig::hooked(|set| *set = hooks).run(add(root.path(), &["new.txt", "tree"], "a.zip", "", knows(None)));

        assert_eq!(
            matches!(ended.outcome, Outcome::Done),
            done,
            "{limit}: {:?}",
            ended.outcome
        );
        assert_eq!(read(&root.path().join("dst/a.zip")) == before, !done, "{limit}");
    }
}

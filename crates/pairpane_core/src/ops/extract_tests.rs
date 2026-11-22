//! Extracting real archives (a plain tar, a tar.gz, a gzip file) through the engine: what lands on the disk, the
//! conflicts, a cancel, a pause, the modes and times, and what is refused. The failures that need a source that misbehaves
//! are in `extract_scripted_tests`.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use pretty_assertions::assert_eq;
use tar::{Builder, EntryType, Header};
use tempfile::TempDir;

use crate::archive::testing::{MOMENT, gzip};
use crate::model::{Answer, ControlAction, JobPhase, OpsEvent, Outcome};

use super::hooks::Hooks;
use super::testing::{
    Ended, Rig, has_no_partial_files, long_ago, modified, pattern, phases, put, read, set_times, task, temp, tree,
};
use super::{ArchiveJob, Request, Task};

/// The modification time of every entry of the test archives.
pub(super) fn moment() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_millis(MOMENT as u64)
}

fn header(kind: EntryType, mode: u32, size: usize) -> Header {
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_size(size as u64);
    header.set_mtime((MOMENT / 1000) as u64);

    header
}

/// A tar built by `fill`.
pub(super) fn tar_by(fill: impl FnOnce(&mut Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    fill(&mut builder);

    builder.into_inner().expect("a tar")
}

/// Adds a file (or, for a path ending in `/`, a folder) with `mode`.
pub(super) fn add(builder: &mut Builder<Vec<u8>>, path: &str, mode: u32, data: &[u8]) {
    let kind = if path.ends_with('/') {
        EntryType::Directory
    } else {
        EntryType::Regular
    };

    builder
        .append_data(&mut header(kind, mode, data.len()), path, data)
        .expect("appended");
}

/// Adds an entry of `kind` that leads to `target`, or is a device or a pipe.
fn add_special(builder: &mut Builder<Vec<u8>>, path: &str, kind: EntryType, target: &str) {
    let mut entry = header(kind, 0o777, 0);
    if !target.is_empty() {
        entry.set_link_name(target).expect("a link name");
    }

    builder.append_data(&mut entry, path, &[][..]).expect("appended");
}

/// Adds a file whose path is written into the header as it is, which the builder would refuse (`../x`).
fn add_raw(builder: &mut Builder<Vec<u8>>, name: &[u8], data: &[u8]) {
    let mut entry = header(EntryType::Regular, 0o644, data.len());
    entry.as_old_mut().name[..name.len()].copy_from_slice(name);
    entry.set_cksum();

    builder.append(&entry, data).expect("appended");
}

/// `a.txt`, and the folder `docs` with `b.txt` and `deep/c.txt`.
pub(super) fn fixture() -> Vec<u8> {
    tar_by(|builder| {
        add(builder, "a.txt", 0o644, b"alpha");
        add(builder, "docs/", 0o755, b"");
        add(builder, "docs/b.txt", 0o644, b"beta");
        add(builder, "docs/deep/", 0o755, b"");
        add(builder, "docs/deep/c.txt", 0o644, b"gamma");
    })
}

/// A folder with the archive `name` of `bytes` in `src`.
pub(super) fn scene(name: &str, bytes: &[u8]) -> TempDir {
    let root = temp();
    put(root.path(), &format!("src/{name}"), bytes);

    root
}

/// The extraction of `entries` of the folder `inner` of the archive `name` in `root/src` into `root/dst`.
pub(super) fn job(root: &Path, name: &str, inner: &str, entries: &[&str], source: ArchiveJob) -> Request {
    task(
        &root.join("src"),
        &[name],
        &root.join("dst"),
        Task::Extract {
            inner: inner.to_owned(),
            entries: entries.iter().map(|entry| (*entry).to_owned()).collect(),
            source,
        },
    )
}

fn extract(root: &Path, name: &str, inner: &str, entries: &[&str]) -> Request {
    job(root, name, inner, entries, ArchiveJob::default())
}

/// Everything in the archive `x.tar` of `bytes`, extracted into a fresh folder.
fn extracted(bytes: &[u8]) -> (TempDir, Ended) {
    let root = scene("x.tar", bytes);
    let ended = Rig::new().run(extract(root.path(), "x.tar", "", &[]));

    (root, ended)
}

pub(super) fn dst(root: &TempDir) -> Vec<String> {
    tree(&root.path().join("dst"))
}

#[test]
fn everything_in_a_tar_and_a_tar_gz_is_extracted_with_its_times_and_counted() {
    for (name, bytes) in [("x.tar", fixture()), ("x.tar.gz", gzip(&fixture()))] {
        let root = scene(name, &bytes);
        let mut rig = Rig::new();

        let ended = rig.run(extract(root.path(), name, "", &[]));

        assert_eq!(ended.outcome, Outcome::Done, "{name}");
        assert_eq!(
            dst(&root),
            ["a.txt", "docs/", "docs/b.txt", "docs/deep/", "docs/deep/c.txt"],
            "{name}"
        );
        assert_eq!(read(&root.path().join("dst/docs/deep/c.txt")), b"gamma");
        assert_eq!(modified(&root.path().join("dst/docs/b.txt")), moment());
        assert_eq!((ended.skipped, ended.errors.len()), (0, 0));
        assert!(phases(&rig).contains(&JobPhase::Extracting));
        assert!(has_no_partial_files(root.path()));

        let last = rig.seen.iter().rev().find_map(|event| match event {
            OpsEvent::Progress {
                done_bytes,
                total_bytes,
                done_files,
                total_files,
                ..
            } => Some((*done_bytes, *total_bytes, *done_files, *total_files)),
            _ => None,
        });
        assert_eq!(last, Some((14, 14, 3, 3)), "the totals are the declared sizes");
    }
}

#[test]
fn chosen_names_come_with_what_is_below_them() {
    let root = scene("x.tar", &fixture());
    let mut rig = Rig::new();

    rig.run(extract(root.path(), "x.tar", "", &["docs"]));

    assert_eq!(dst(&root), ["docs/", "docs/b.txt", "docs/deep/", "docs/deep/c.txt"]);

    let other = scene("x.tar", &fixture());
    rig.run(extract(other.path(), "x.tar", "", &["a.txt"]));

    assert_eq!(dst(&other), ["a.txt"]);
}

#[test]
fn a_folder_of_the_archive_is_extracted_as_the_root_of_the_target() {
    let root = scene("x.tar", &fixture());
    let mut rig = Rig::new();

    rig.run(extract(root.path(), "x.tar", "docs", &[]));
    assert_eq!(dst(&root), ["b.txt", "deep/", "deep/c.txt"]);

    let other = scene("x.tar", &fixture());
    rig.run(extract(other.path(), "x.tar", "docs/deep", &["c.txt"]));
    assert_eq!(dst(&other), ["c.txt"]);
}

#[test]
fn folders_that_no_entry_names_are_made() {
    let bytes = tar_by(|builder| add(builder, "p/q/r.txt", 0o644, b"deep"));
    let root = scene("x.tar", &bytes);

    Rig::new().run(extract(root.path(), "x.tar", "", &[]));

    assert_eq!(dst(&root), ["p/", "p/q/", "p/q/r.txt"]);
}

#[test]
fn a_gzip_file_is_extracted_under_its_name_without_gz() {
    let root = scene("notes.txt.gz", &gzip(b"hello"));

    let ended = Rig::new().run(extract(root.path(), "notes.txt.gz", "", &[]));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(dst(&root), ["notes.txt"]);
    assert_eq!(read(&root.path().join("dst/notes.txt")), b"hello");
}

/// A scene where `dst/a.txt` is "old" and the archive's `a.txt` is "alpha"; the job is asked what to do.
fn clash(answer: Answer, all: bool) -> (TempDir, Ended, crate::model::ConflictInfo) {
    let root = scene("x.tar", &fixture());
    put(root.path(), "dst/a.txt", b"old");
    let mut rig = Rig::new();

    let job = rig.start(extract(root.path(), "x.tar", "", &["a.txt"]));
    let question = rig.answer_next(job, answer, all);

    (root, rig.wait_ended(job), question)
}

#[test]
fn a_taken_name_asks_without_a_compare_and_overwrite_replaces() {
    let (root, ended, question) = clash(Answer::Overwrite, false);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"alpha");
    assert_eq!(question.name, "a.txt");
    assert_eq!(question.suggested, "a (2).txt");
    assert!(!question.comparable);
    assert_eq!((question.source.size, question.source.modified), (5, Some(MOMENT)));
    assert_eq!(question.target.size, 3);
}

#[test]
fn skip_keeps_the_target_and_counts_and_rename_keeps_both() {
    let (root, ended, _) = clash(Answer::Skip, false);
    assert_eq!(
        (read(&root.path().join("dst/a.txt")), ended.skipped),
        (b"old".to_vec(), 1)
    );
    assert_eq!(dst(&root), ["a.txt"]);

    let (root, ended, _) = clash(Answer::Rename, false);
    assert_eq!(ended.skipped, 0);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"old");
    assert_eq!(read(&root.path().join("dst/a (2).txt")), b"alpha");
}

#[test]
fn overwrite_older_replaces_only_a_target_older_than_the_entry_and_asks_once() {
    let root = scene("x.tar", &fixture());
    put(root.path(), "dst/a.txt", b"old");
    put(root.path(), "dst/docs/b.txt", b"new");
    set_times(&root.path().join("dst/a.txt"), long_ago());
    let mut rig = Rig::new();

    let job = rig.start(extract(root.path(), "x.tar", "", &[]));
    rig.answer_next(job, Answer::OverwriteOlder, true);
    let ended = rig.wait_ended(job);

    assert_eq!(read(&root.path().join("dst/a.txt")), b"alpha", "older: replaced");
    assert_eq!(read(&root.path().join("dst/docs/b.txt")), b"new", "newer: kept");
    assert_eq!(ended.skipped, 1);
    assert_eq!(
        rig.seen
            .iter()
            .filter(|event| matches!(event, OpsEvent::Conflict { .. }))
            .count(),
        1
    );
    assert!(phases(&rig).ends_with(&[JobPhase::Waiting, JobPhase::Extracting]));
}

#[test]
fn a_cancel_in_the_middle_of_a_file_removes_it_and_the_folders_that_are_empty() {
    let bytes = tar_by(|builder| {
        add(builder, "d/", 0o755, b"");
        add(builder, "d/big.bin", 0o644, &pattern(64 * 1024 * 1024));
    });
    let root = scene("x.tar", &bytes);
    let hooks = Hooks {
        every_chunk: true,
        ..Hooks::default()
    };
    // The listener cancels at the first progress that shows bytes written, so the cancel lands mid-file.
    let mut rig = Rig::with(hooks, |engine, event| {
        if let OpsEvent::Progress { id, done_bytes, .. } = event
            && *done_bytes > 1
        {
            let _ = engine.control(*id, ControlAction::Cancel);
        }
    });
    let mut request = extract(root.path(), "x.tar", "", &[]);
    request.to = root.path().join("made/by/the/job");

    let ended = rig.run(request);

    assert_eq!(ended.outcome, Outcome::Cancelled);
    assert_eq!(tree(root.path()), ["src/", "src/x.tar"], "nothing is left of the job");
}

#[test]
fn a_paused_extraction_waits_and_goes_on_when_resumed() {
    let root = scene("x.tar", &fixture());
    let mut rig = Rig::paused_in(JobPhase::Extracting);

    let job = rig.start(extract(root.path(), "x.tar", "", &[]));
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    rig.drain();
    assert_eq!(rig.ended_count(job), 0, "it waits");

    rig.engine.control(job, ControlAction::Resume).expect("known job");

    assert_eq!(rig.wait_ended(job).outcome, Outcome::Done);
    assert_eq!(dst(&root).len(), 5);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use pretty_assertions::assert_eq;

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o7777
    }

    #[test]
    fn setuid_setgid_and_sticky_are_stripped_and_a_folder_gets_its_time_after_its_contents() {
        let bytes = tar_by(|builder| {
            add(builder, "d/", 0o3750, b"");
            add(builder, "d/suid", 0o4755, b"x");
            add(builder, "d/sgid", 0o2750, b"x");
            add(builder, "d/sticky", 0o1644, b"x");
        });
        let root = scene("x.tar", &bytes);

        Rig::new().run(extract(root.path(), "x.tar", "", &[]));

        let at = |name: &str| mode(&root.path().join("dst").join(name));
        assert_eq!(
            (at("d"), at("d/suid"), at("d/sgid"), at("d/sticky")),
            (0o750, 0o755, 0o750, 0o644)
        );
        assert_eq!(modified(&root.path().join("dst/d")), moment());
        assert_eq!(modified(&root.path().join("dst/d/suid")), moment());
    }

    #[test]
    fn an_entry_without_a_mode_gets_the_usual_one() {
        let root = scene("notes.txt.gz", &gzip(b"hello"));

        Rig::new().run(extract(root.path(), "notes.txt.gz", "", &[]));

        assert_eq!(mode(&root.path().join("dst/notes.txt")), 0o644);
    }

    #[test]
    fn links_and_special_entries_are_skipped_and_counted_and_never_made() {
        let bytes = tar_by(|builder| {
            add(builder, "ok.txt", 0o644, b"ok");
            add_special(builder, "sym", EntryType::Symlink, "/etc/passwd");
            add_special(builder, "hard", EntryType::Link, "ok.txt");
            add_special(builder, "dev", EntryType::Char, "");
            add_special(builder, "pipe", EntryType::Fifo, "");
        });
        let (root, ended) = extracted(&bytes);

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.skipped, 4);
        assert_eq!(dst(&root), ["ok.txt"]);
    }

    #[test]
    fn nothing_is_written_through_a_link_that_is_in_the_target() {
        let root = scene("x.tar", &fixture());
        put(root.path(), "elsewhere/secret", b"secret");
        put(root.path(), "outside.txt", b"outside");
        fs::create_dir_all(root.path().join("dst")).expect("target");
        symlink(root.path().join("elsewhere"), root.path().join("dst/docs")).expect("link");
        symlink(root.path().join("outside.txt"), root.path().join("dst/a.txt")).expect("link");
        let mut rig = Rig::new();

        let job = rig.start(extract(root.path(), "x.tar", "", &[]));
        rig.answer_next(job, Answer::Overwrite, true);
        let ended = rig.wait_ended(job);

        assert_eq!(
            tree(&root.path().join("elsewhere")),
            ["secret"],
            "nothing went through the folder link"
        );
        assert_eq!(
            read(&root.path().join("outside.txt")),
            b"outside",
            "nor through the file link"
        );
        assert_eq!(
            read(&root.path().join("dst/a.txt")),
            b"alpha",
            "the link itself is replaced"
        );
        let failed: Vec<_> = ended.errors.iter().map(|error| error.path.as_str()).collect();
        assert_eq!(failed, ["docs", "docs/b.txt", "docs/deep", "docs/deep/c.txt"]);
        assert_eq!(
            ended.errors[0].error.message,
            "a file or link has the name of this folder"
        );
    }
}

#[test]
fn names_that_could_leave_the_target_are_refused_one_by_one_and_counted() {
    let long = format!("a/{}", "x".repeat(300));
    let bytes = tar_by(|builder| {
        add(builder, "good.txt", 0o644, b"good");
        for name in [
            &b"../evil.txt"[..],
            b"/abs.txt",
            b"a/../../up.txt",
            b"C:\\x",
            b"C:x",
            b"\\\\host\\x",
            b"back\\slash.txt",
            b"ctrl\x07.txt",
        ] {
            add_raw(builder, name, b"bad");
        }
        add(builder, &long, 0o644, b"bad");
    });
    let (root, ended) = extracted(&bytes);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 9);
    assert_eq!(dst(&root), ["good.txt"]);
    assert_eq!(tree(root.path()), ["dst/", "dst/good.txt", "src/", "src/x.tar"]);
}

#[test]
fn a_file_where_a_folder_is_wanted_fails_what_is_below_it_and_the_rest_goes_on() {
    let root = scene("x.tar", &fixture());
    put(root.path(), "dst/docs", b"a file");
    put(root.path(), "dst/keep.txt", b"kept");

    let ended = Rig::new().run(extract(root.path(), "x.tar", "", &[]));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 4);
    assert_eq!(dst(&root), ["a.txt", "docs", "keep.txt"]);
    assert_eq!(read(&root.path().join("dst/docs")), b"a file");
}

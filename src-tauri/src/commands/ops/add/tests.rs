//! Tests of `ops_add_to_archive`: what is refused before a job exists, the check for a password, and the index kept for
//! the archive going when the job ends.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pairpane_core::archive::Password;
use pairpane_core::model::{OpKind, OpsEvent};
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::archives::fixtures::{
    SEVENZ_PASSWORD, archive_in_folder, encrypted, item, path_text, sevenz_with_encrypted_headers, tar_of, zip_of,
};
use crate::commands::ops::tests::{added_job, received, running, wait_for_end};
use crate::commands::testing::{folder_with, names};

type Events = Arc<Mutex<Vec<OpsEvent>>>;

/// A folder with one file to add, an engine that reports to `events`, and what the app keeps of archives.
struct Bench {
    _files: tempfile::TempDir,
    _out: tempfile::TempDir,
    from: String,
    ops: Ops,
    events: Events,
    archives: Archives,
}

impl Bench {
    fn new() -> Self {
        let (files, from) = folder_with(&[("new.txt", b"new")]);
        let (ops, out, events) = running();

        Self {
            _files: files,
            _out: out,
            from,
            ops,
            events,
            archives: Archives::default(),
        }
    }

    /// Asks to add `new.txt` to the folder `dir` of `archive`.
    fn add(&self, archive: &str, dir: &str) -> CommandResult<JobId> {
        self.add_from(archive, dir, &self.from, &["new.txt"])
    }

    fn add_from(&self, archive: &str, dir: &str, from: &str, chosen: &[&str]) -> CommandResult<JobId> {
        block_on(self.ops.add_to_archive(
            &self.archives,
            archive.to_owned(),
            dir.to_owned(),
            from.to_owned(),
            names(chosen),
        ))
    }

    fn jobs_added(&self) -> usize {
        received(&self.events)
            .iter()
            .filter(|event| matches!(event, OpsEvent::Added { .. }))
            .count()
    }
}

fn sample_tar() -> Vec<u8> {
    tar_of(&[
        item("docs/", b""),
        item("docs/guide.txt", b"guide text"),
        item("readme.txt", b"hello"),
    ])
}

#[test]
fn files_are_added_as_a_job_on_the_archive_for_every_written_format() {
    let zip = zip_of(&[item("a.txt", b"x")]);

    for (name, bytes) in [("pack.zip", zip), ("pack.tar", sample_tar())] {
        let (_folder, archive) = archive_in_folder(name, &bytes);
        let bench = Bench::new();

        let id = bench.add(&archive, "").expect(name);

        wait_for_end(&bench.events);
        let added = added_job(&bench.events, id);
        assert_eq!(added.kind, OpKind::AddToArchive, "{name}");
        assert_eq!(added.label, "new.txt", "{name}");
        assert_eq!(added.from, bench.from, "{name}");
    }
}

#[test]
fn an_archive_that_is_not_written_is_refused_before_any_job() {
    let bench = Bench::new();

    for name in ["pack.rar", "single.gz", "notes.txt", "PACK.RAR"] {
        let path = std::env::temp_dir().join(name);

        let error = bench.add(&path_text(&path), "").expect_err(name);

        assert_eq!(error.code(), IpcErrorCode::Unsupported, "{name}");
        assert_eq!(
            serde_json::to_value(&error).expect("serializable")["message"],
            "This archive can't be changed",
            "{name}"
        );
    }

    assert_eq!(bench.jobs_added(), 0);
}

#[test]
fn an_archive_that_is_cut_short_or_has_refused_names_is_refused() {
    let bench = Bench::new();

    let mut cut = tar_of(&[item("big.bin", &[7; 2000])]);
    cut.truncate(512 + 600);
    let climbing = tar_of(&[item("ok.txt", b"x"), item("../evil.txt", b"y")]);
    let zip_climbing = zip_of(&[item("../evil.txt", b"y")]);

    for (name, bytes) in [
        ("cut.tar", cut),
        ("climbing.tar", climbing),
        ("climbing.zip", zip_climbing),
    ] {
        let (_folder, archive) = archive_in_folder(name, &bytes);

        let error = bench.add(&archive, "").expect_err(name);

        assert_eq!(error.code(), IpcErrorCode::Unsupported, "{name}");
    }

    assert_eq!(bench.jobs_added(), 0);
}

#[test]
fn the_folder_must_be_in_the_archive_or_its_root() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let bench = Bench::new();

    for dir in ["nowhere", "readme.txt"] {
        let error = bench.add(&archive, dir).expect_err(dir);

        assert_eq!(error.code(), IpcErrorCode::NotFound, "{dir}");
    }

    assert_eq!(bench.jobs_added(), 0);
    bench.add(&archive, "docs").expect("a folder of the archive");
    bench.add(&archive, "").expect("the root");
}

#[test]
fn the_path_the_names_and_the_folders_are_checked_for_syntax() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let bench = Bench::new();
    let from = bench.from.as_str();

    for (archive, dir, from, chosen) in [
        ("pack.tar", "", from, vec!["new.txt"]),
        (archive.as_str(), "../x", from, vec!["new.txt"]),
        (archive.as_str(), "docs//x", from, vec!["new.txt"]),
        (archive.as_str(), "", "relative", vec!["new.txt"]),
        (archive.as_str(), "", from, vec![]),
        (archive.as_str(), "", from, vec!["a/b"]),
        (archive.as_str(), "", from, vec![".."]),
    ] {
        let error = bench.add_from(archive, dir, from, &chosen).expect_err("refused");

        assert_eq!(
            error.code(),
            IpcErrorCode::InvalidInput,
            "{archive} {dir} {from} {chosen:?}"
        );
    }

    assert_eq!(bench.jobs_added(), 0);
}

#[test]
fn encrypted_entries_without_a_password_fail_before_a_job_exists() {
    let zip = zip_of(&[encrypted("secret.txt", b"cipher"), item("open.txt", b"plain")]);
    let (_zip_folder, zip_archive) = archive_in_folder("pack.zip", &zip);
    let (_sevenz_folder, sevenz_archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let bench = Bench::new();

    for archive in [&zip_archive, &sevenz_archive] {
        let error = bench.add(archive, "").expect_err("password needed");

        assert_eq!(error.code(), IpcErrorCode::PasswordRequired, "{archive}");
    }

    assert_eq!(bench.jobs_added(), 0);
}

#[test]
fn the_password_given_earlier_goes_into_the_job_with_the_index_and_the_state_of_the_file() {
    let zip = zip_of(&[encrypted("secret.txt", b"cipher")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let archives = Archives::default();
    archives.remember(PathBuf::from(&archive), Password::new("pw".to_owned()));
    let target = Target::parse(archive).expect("valid");

    let source = prepare_rewrite(&archives, &target, "").expect("no password is missing");

    assert_eq!(source.password.as_deref().map(Password::as_str), Some("pw"));
    assert!(source.index.is_some());
    assert!(source.fingerprint.is_some());
}

#[test]
fn a_tar_without_a_kept_index_is_listed_now_because_the_checks_need_it() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let target = Target::parse(archive).expect("valid");

    let source = prepare_rewrite(&Archives::default(), &target, "docs").expect("prepared");

    assert!(source.index.is_some_and(|index| index.has_dir("docs")));
    assert!(source.password.is_none());
}

#[test]
fn a_password_that_the_archive_turns_down_is_forgotten_so_the_page_asks_again() {
    let (_folder, archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let bench = Bench::new();
    bench
        .archives
        .remember(PathBuf::from(&archive), Password::new("not-the-password".to_owned()));

    let error = bench.add(&archive, "").expect_err("turned down");

    assert_eq!(error.code(), IpcErrorCode::PasswordRequired);
    assert!(bench.archives.password(Path::new(&archive)).is_none());
    assert_eq!(bench.jobs_added(), 0);
}

#[test]
fn the_right_password_lets_an_encrypted_7z_be_added_to() {
    let (_folder, archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let bench = Bench::new();
    bench
        .archives
        .remember(PathBuf::from(&archive), Password::new(SEVENZ_PASSWORD.to_owned()));

    bench.add(&archive, "").expect("accepted");

    wait_for_end(&bench.events);
    assert_eq!(bench.jobs_added(), 1);
}

#[test]
fn the_index_kept_for_the_archive_goes_when_the_job_ends() {
    let (_folder, archive) = archive_in_folder("pack.zip", &zip_of(&[item("a.txt", b"x")]));
    let bench = Bench::new();
    let target = Target::parse(archive.clone()).expect("valid");
    // The state of the file the index was made from: the job may replace the file, and then it is no longer this one.
    let before = bench.archives.open(&target).expect("opened");
    bench
        .archives
        .index(&target, &before, bench.archives.stop(), None)
        .expect("listed and kept");
    assert!(bench.archives.cached(&before).is_some());

    bench.add(&archive, "").expect("accepted");
    wait_for_end(&bench.events);

    assert!(
        bench.archives.cached(&before).is_none(),
        "the next open lists the archive again"
    );
}

//! Tests of `ops_extract`: what is refused before a job starts, the check for a password, and a tar extracted for real
//! into a folder through the engine.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pairpane_core::archive::Password;
use pairpane_core::model::{OpKind, OpsEvent, Outcome};
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::archives::fixtures::{
    archive_in_folder, encrypted, item, path_text, sevenz_with_encrypted_headers, tar_of, zip_of,
};
use crate::commands::ops::tests::{collecting_channel, received};
use crate::commands::testing::names;

fn extract_into(
    ops: &Ops,
    archives: &Archives,
    archive: &str,
    dir: &str,
    chosen: &[&str],
    to: &Path,
) -> CommandResult<JobId> {
    block_on(ops.extract(
        archives,
        archive.to_owned(),
        dir.to_owned(),
        names(chosen),
        path_text(to),
    ))
}

fn wait_for_end(events: &std::sync::Arc<std::sync::Mutex<Vec<OpsEvent>>>) -> Outcome {
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
        let ended = received(events).into_iter().find_map(|event| match event {
            OpsEvent::Ended { outcome, .. } => Some(outcome),
            _ => None,
        });
        if let Some(outcome) = ended {
            return outcome;
        }
        assert!(Instant::now() < deadline, "the job ended");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// An engine that reports to `events`, and an empty folder to extract into.
fn running() -> (Ops, tempfile::TempDir, std::sync::Arc<std::sync::Mutex<Vec<OpsEvent>>>) {
    let ops = Ops::default();
    let (channel, events) = collecting_channel();
    ops.subscribe(channel);

    (ops, tempfile::tempdir().expect("target"), events)
}

fn sample() -> Vec<u8> {
    tar_of(&[
        item("docs/", b""),
        item("docs/guide.txt", b"guide text"),
        item("readme.txt", b"hello"),
    ])
}

#[test]
fn a_tar_is_extracted_into_the_folder_as_an_extract_job() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample());
    let (ops, out, events) = running();

    let id = extract_into(&ops, &Archives::default(), &archive, "", &[], out.path()).expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    let seen = received(&events);
    assert!(
        seen.iter()
            .any(|event| matches!(event, OpsEvent::Added { job } if job.id == id && job.kind == OpKind::Extract))
    );
    assert_eq!(std::fs::read(out.path().join("readme.txt")).expect("file"), b"hello");
    assert_eq!(
        std::fs::read(out.path().join("docs").join("guide.txt")).expect("file"),
        b"guide text"
    );
}

#[test]
fn only_the_chosen_names_of_an_inner_folder_are_extracted() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample());
    let (ops, out, events) = running();

    extract_into(&ops, &Archives::default(), &archive, "docs", &["guide.txt"], out.path()).expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert_eq!(
        std::fs::read(out.path().join("guide.txt")).expect("file"),
        b"guide text"
    );
    assert!(!out.path().join("readme.txt").exists());
    assert!(!out.path().join("docs").exists());
}

#[test]
fn encrypted_entries_without_a_password_fail_before_a_job_exists() {
    let zip = zip_of(&[encrypted("secret.txt", b"cipher"), item("open.txt", b"plain")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let (ops, out, events) = running();
    let archives = Archives::default();

    for chosen in [&[][..], &["secret.txt"][..]] {
        let error = extract_into(&ops, &archives, &archive, "", chosen, out.path()).expect_err("password needed");
        assert_eq!(error.code(), IpcErrorCode::PasswordRequired);
    }

    assert_eq!(received(&events).len(), 1, "only the snapshot: no job was added");
}

#[test]
fn the_password_given_earlier_goes_into_the_job() {
    let zip = zip_of(&[encrypted("secret.txt", b"cipher")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let archives = Archives::default();
    let target = Target::parse(archive).expect("valid");
    archives.remember(
        target.path.clone(),
        pairpane_core::archive::Password::new("pw".to_owned()),
    );

    let job = prepare(&archives, &target, "", &[]).expect("no password is missing");

    assert!(job.password.is_some());
    assert!(job.index.is_some());
    assert!(job.fingerprint.is_some());
}

#[test]
fn an_item_that_is_not_in_the_archive_is_not_found_and_a_tar_is_left_to_the_job() {
    let zip = zip_of(&[item("a.txt", b"x")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let archives = Archives::default();
    let target = Target::parse(archive).expect("valid");

    let error = prepare(&archives, &target, "", &names(&["missing.txt"])).expect_err("not there");
    assert_eq!(error.code, IpcErrorCode::NotFound);

    let (_tar_folder, tar) = archive_in_folder("pack.tar", &sample());
    let target = Target::parse(tar).expect("valid");
    let job = prepare(&archives, &target, "", &[]).expect("prepared");
    assert!(
        job.index.is_none(),
        "a compressed or plain tar is read by the job itself"
    );
    assert!(job.fingerprint.is_some());
}

#[test]
fn names_that_are_not_plain_and_a_folder_that_climbs_are_refused() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample());
    let out = tempfile::tempdir().expect("target");
    let (ops, archives) = (Ops::default(), Archives::default());

    for (dir, chosen) in [
        ("", vec![".."]),
        ("", vec!["a/b"]),
        ("", vec![""]),
        ("../x", vec![]),
        ("docs//x", vec![]),
    ] {
        let error = block_on(ops.extract(
            &archives,
            archive.clone(),
            dir.to_owned(),
            chosen.iter().map(|name| (*name).to_owned()).collect(),
            path_text(out.path()),
        ))
        .expect_err(dir);
        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{dir} {chosen:?}");
    }
}

#[test]
fn a_destination_that_is_not_an_absolute_folder_and_duplicate_names_are_handled() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample());
    let (ops, archives) = (Ops::default(), Archives::default());

    let error = block_on(ops.extract(&archives, archive, String::new(), Vec::new(), "relative".to_owned()))
        .expect_err("refused");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);

    assert_eq!(
        checked_entries(names(&["a", "b", "a"])).expect("valid"),
        names(&["a", "b"]),
        "duplicates are dropped"
    );
    assert!(checked_entries(Vec::new()).expect("none means everything").is_empty());
}

#[test]
fn a_remembered_password_that_the_archive_turns_down_is_forgotten_so_the_page_asks_again() {
    let (_folder, archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let (ops, out, _events) = running();
    let archives = Archives::default();
    archives.remember(
        PathBuf::from(&archive),
        pairpane_core::archive::Password::new("not-the-password".to_owned()),
    );

    let error = extract_into(&ops, &archives, &archive, "", &[], out.path()).expect_err("turned down");

    assert_eq!(error.code(), IpcErrorCode::PasswordRequired);
    assert!(archives.password(Path::new(&archive)).is_none());
}

#[test]
fn a_password_the_job_finds_wrong_while_it_decrypts_is_forgotten_when_the_job_ends() {
    let zip = zip_of(&[encrypted("secret.txt", b"not really encrypted data")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let (ops, out, events) = running();
    let archives = Archives::default();
    let kept = PathBuf::from(&archive);
    archives.remember(kept.clone(), Password::new("wrong-one".to_owned()));

    extract_into(&ops, &archives, &archive, "", &[], out.path()).expect("the directory does not turn it down");

    let Outcome::Failed { error } = wait_for_end(&events) else {
        panic!("the job fails on the data");
    };
    assert_eq!(error.code, IpcErrorCode::PasswordRequired);
    assert!(error.message.contains("wrong"), "{}", error.message);
    assert!(
        archives.password(&kept).is_none(),
        "forgotten before the page was told, so it asks again"
    );
}

#[test]
fn a_job_that_ends_for_another_reason_keeps_the_password() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample());
    let (ops, out, events) = running();
    let archives = Archives::default();
    let kept = PathBuf::from(&archive);
    archives.remember(kept.clone(), Password::new("right-one".to_owned()));

    extract_into(&ops, &archives, &archive, "", &[], out.path()).expect("accepted");

    assert_eq!(wait_for_end(&events), Outcome::Done);
    assert!(archives.password(&kept).is_some());
}

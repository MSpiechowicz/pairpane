//! Tests of `ops_pack`: the job it asks the engine for, and what is refused before any job exists.

use std::path::{Path, PathBuf};

use pairpane_core::archive::Password;
use pairpane_core::model::OpKind;

use super::*;
use crate::commands::ops::checked_names;
use crate::commands::ops::tests::{added_job, running, wait_for_end};
use crate::commands::testing::{elsewhere, names, root};

fn options(format: PackFormat) -> PackJob {
    pack_job(format, PackLevel::Normal, None, false).expect("valid")
}

fn archive_named(name: &str) -> String {
    PathBuf::from(elsewhere()).join(name).to_string_lossy().into_owned()
}

fn pack(archive: &str, format: PackFormat) -> CommandResult<Request> {
    pack_request(root(), names(&["a.txt", "docs"]), archive.to_owned(), options(format))
}

#[test]
fn the_items_of_the_folder_are_packed_into_a_new_archive_in_the_archives_folder() {
    let request = pack(&archive_named("out.zip"), PackFormat::Zip).expect("accepted");

    assert_eq!(request.kind, OpKind::Pack);
    assert_eq!(request.from, PathBuf::from(root()));
    assert_eq!(request.names, names(&["a.txt", "docs"]));
    assert_eq!(request.to, PathBuf::from(elsewhere()));
    assert!(!request.verify);
    assert_eq!(
        request.task,
        Task::Pack {
            name: "out.zip".to_owned(),
            options: PackJob {
                format: PackFormat::Zip,
                level: PackLevel::Normal,
                password: None,
                encrypt_names: false,
            },
        }
    );
}

#[test]
fn the_extension_must_be_the_one_of_the_format() {
    for (name, format) in [
        ("out.zip", PackFormat::Zip),
        ("OUT.ZIP", PackFormat::Zip),
        ("out.jar", PackFormat::Zip),
        ("out.tar.zst", PackFormat::TarZst),
        ("out.tzst", PackFormat::TarZst),
        ("out.7z", PackFormat::SevenZ),
    ] {
        pack(&archive_named(name), format).unwrap_or_else(|error| panic!("{name}: {error:?}"));
    }

    for (name, format) in [
        ("out.7z", PackFormat::Zip),
        ("out.tar", PackFormat::TarZst),
        ("out.tar.gz", PackFormat::TarZst),
        ("out.zip", PackFormat::TarZst),
        ("out.zip", PackFormat::SevenZ),
        ("out.rar", PackFormat::SevenZ),
        ("out", PackFormat::Zip),
        ("out.zip.txt", PackFormat::Zip),
    ] {
        let error = pack(&archive_named(name), format).expect_err(name);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{name}");
    }
}

#[test]
fn an_archive_path_that_is_not_an_absolute_plain_file_is_refused() {
    let climbing = PathBuf::from(elsewhere()).join("..").join("out.zip");

    for archive in [
        "out.zip".to_owned(),
        String::new(),
        climbing.to_string_lossy().into_owned(),
        format!("{}{}", archive_named("sub"), std::path::MAIN_SEPARATOR),
    ] {
        let error = pack(&archive, PackFormat::Zip).expect_err(&archive);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{archive}");
    }
}

#[test]
fn the_source_folder_and_the_names_are_checked_like_a_copy_checks_them() {
    let archive = archive_named("out.zip");

    for (from, chosen) in [
        ("relative".to_owned(), names(&["a"])),
        (root(), Vec::new()),
        (root(), names(&[".."])),
    ] {
        let error =
            pack_request(from.clone(), chosen.clone(), archive.clone(), options(PackFormat::Zip)).expect_err("refused");

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{from} {chosen:?}");
    }

    let request = pack_request(root(), names(&["a", "b", "a"]), archive, options(PackFormat::Zip)).expect("valid");
    assert_eq!(request.names, names(&["a", "b"]), "duplicates are dropped");
}

#[test]
fn relative_paths_below_the_folder_are_items_of_a_pack_and_of_nothing_else() {
    let archive = archive_named("out.zip");
    let request = pack_request(
        root(),
        names(&["a/b.txt", "c/d/e.txt", "a/b.txt", "f"]),
        archive.clone(),
        options(PackFormat::Zip),
    )
    .expect("valid");
    assert_eq!(
        request.names,
        names(&["a/b.txt", "c/d/e.txt", "f"]),
        "duplicates are dropped"
    );

    for chosen in ["a/b.txt", "c/d/e.txt"] {
        let error = checked_names(Path::new(&root()), names(&[chosen])).expect_err(chosen);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{chosen}");
    }

    for chosen in [
        names(&["../x"]),
        names(&["a/../x"]),
        names(&["/abs"]),
        names(&["a//b"]),
        names(&["a/./b"]),
        names(&["a/"]),
        names(&["a/b\\c"]),
        names(&["a", "a/b"]),
        names(&["a/b/c", "x", "a/b"]),
        vec![format!("a/{}", "x".repeat(300))],
    ] {
        let error =
            pack_request(root(), chosen.clone(), archive.clone(), options(PackFormat::Zip)).expect_err("refused");

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{chosen:?}");
    }
}

#[test]
fn a_password_is_none_when_empty_and_otherwise_travels_in_the_job() {
    for password in [None, Some(String::new())] {
        let job = pack_job(PackFormat::Zip, PackLevel::Fast, password, false).expect("valid");

        assert!(job.password.is_none());
        assert_eq!(job.level, PackLevel::Fast);
    }

    let job = pack_job(PackFormat::SevenZ, PackLevel::Store, Some("hunter2".to_owned()), false).expect("valid");

    assert_eq!(job.password.as_deref().map(Password::as_str), Some("hunter2"));
}

#[test]
fn a_password_over_the_limit_is_refused_without_showing_it() {
    let long = "x".repeat(1025);

    let error = pack_job(PackFormat::Zip, PackLevel::Normal, Some(long.clone()), false).expect_err("too long");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(!format!("{error:?}").contains(&long));
    assert!(pack_job(PackFormat::Zip, PackLevel::Normal, Some("x".repeat(1024)), false).is_ok());
}

#[test]
fn a_tar_zst_has_no_password() {
    let error = pack_job(PackFormat::TarZst, PackLevel::Normal, Some("secret".to_owned()), false).expect_err("refused");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(pack_job(PackFormat::TarZst, PackLevel::Normal, Some(String::new()), false).is_ok());
}

#[test]
fn names_are_encrypted_only_in_a_7z_with_a_password() {
    let secret = || Some("secret".to_owned());

    assert!(
        pack_job(PackFormat::SevenZ, PackLevel::Normal, secret(), true)
            .expect("valid")
            .encrypt_names
    );

    for (format, password) in [
        (PackFormat::SevenZ, None),
        (PackFormat::SevenZ, Some(String::new())),
        (PackFormat::Zip, secret()),
        (PackFormat::TarZst, None),
    ] {
        let job = pack_job(format, PackLevel::Normal, password, true).expect("not refused");

        assert!(!job.encrypt_names, "{format:?}");
    }
}

#[test]
fn the_engine_runs_the_job_as_a_pack_job_into_the_archives_folder() {
    let (ops, out, events) = running();
    let source = tempfile::tempdir().expect("source");
    std::fs::write(source.path().join("a.txt"), b"alpha").expect("file");
    let archive = out.path().join("out.zip");

    let request = pack_request(
        source.path().to_string_lossy().into_owned(),
        names(&["a.txt"]),
        archive.to_string_lossy().into_owned(),
        options(PackFormat::Zip),
    )
    .expect("accepted");
    let id = ops.engine.submit(request).expect("queued");
    wait_for_end(&events);

    let added = added_job(&events, id);
    assert_eq!(added.kind, OpKind::Pack);
    assert_eq!(added.label, "a.txt");
    assert_eq!(PathBuf::from(&added.from), source.path());
    assert_eq!(
        PathBuf::from(&added.to),
        archive,
        "the queue shows the archive, not its folder"
    );
}

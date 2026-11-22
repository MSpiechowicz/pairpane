//! Tests of what the archive commands share: the path check, the passwords and the error texts.

use pairpane_core::archive::{self, PasswordFault};
use tauri::async_runtime::block_on;

use super::fixtures::{archive_in_folder, item, tar_of, zip_of};
use super::*;
use crate::commands::testing::root;

fn code(error: &IpcError) -> IpcErrorCode {
    error.code
}

#[test]
fn an_archive_path_is_an_absolute_file_with_a_plain_name() {
    let path = Path::new(&root()).join("docs").join("a.zip");

    let target = Target::parse(path.to_string_lossy().into_owned()).expect("valid");

    assert_eq!(target.name, "a.zip");
    assert_eq!(target.folder, Path::new(&root()).join("docs"));
    assert_eq!(target.path, path);
}

#[test]
fn a_path_that_is_not_plain_and_absolute_is_refused() {
    let sep = std::path::MAIN_SEPARATOR;
    let climbing = format!("{}{sep}..{sep}a.zip", root());
    let ends_climbing = format!("{}{sep}..", root());

    for bad in ["a.zip", "docs/a.zip", "", climbing.as_str(), ends_climbing.as_str()] {
        let error = Target::parse(bad.to_owned()).expect_err(bad);
        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad}");
    }
    #[cfg(windows)]
    for bad in [r"\\attacker\x\a.zip", r"\\.\pipe\a.zip", r"\\?\UNC\a\b\c.zip"] {
        assert!(Target::parse(bad.to_owned()).is_err(), "{bad}");
    }
}

#[test]
fn a_password_check_keeps_the_password_for_that_archive_only() {
    let (_folder, path) = archive_in_folder("a.zip", &zip_of(&[item("a.txt", b"hello")]));
    let (_other, other_path) = archive_in_folder("a.zip", &zip_of(&[item("a.txt", b"hello")]));
    let archives = Archives::default();

    let accepted = block_on(archives.remember_if_right(path.clone(), "sesame".to_owned())).expect("checked");

    assert!(accepted);
    let kept = archives.password(Path::new(&path)).expect("kept");
    assert_eq!(kept.as_str(), "sesame");
    assert!(archives.password(Path::new(&other_path)).is_none());
}

#[test]
fn a_password_is_never_shown_by_debug() {
    let (_folder, path) = archive_in_folder("a.zip", &zip_of(&[item("a.txt", b"hello")]));
    let archives = Archives::default();
    block_on(archives.remember_if_right(path.clone(), "hunter2-secret".to_owned())).expect("checked");

    let shown = format!("{:?}", archives.password(Path::new(&path)));

    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn an_empty_or_huge_password_and_a_file_that_is_not_an_archive_are_refused() {
    let (_folder, path) = archive_in_folder("a.zip", &zip_of(&[item("a.txt", b"x")]));
    let (_plain, text) = archive_in_folder("notes.txt", b"just text");
    let archives = Archives::default();

    for password in [String::new(), "x".repeat(MAX_PASSWORD_BYTES + 1)] {
        let error = block_on(archives.remember_if_right(path.clone(), password)).expect_err("refused");
        assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    }

    let error = block_on(archives.remember_if_right(text.clone(), "pw".to_owned())).expect_err("not an archive");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(archives.password(Path::new(&text)).is_none());
}

#[test]
fn a_link_is_not_opened_as_an_archive() {
    #[cfg(unix)]
    {
        let (folder, path) = archive_in_folder("real.tar", &tar_of(&[item("a.txt", b"x")]));
        let link = folder.path().join("link.tar");
        std::os::unix::fs::symlink(&path, &link).expect("link");
        let target = Target::parse(link.to_string_lossy().into_owned()).expect("valid");

        let error = Archives::default().open(&target).err().expect("refused");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

#[test]
fn the_index_of_an_archive_is_kept_until_the_file_changes() {
    let (folder, path) = archive_in_folder("a.tar", &tar_of(&[item("a.txt", b"one")]));
    let target = Target::parse(path).expect("valid");
    let archives = Archives::default();
    let never = AtomicBool::new(false);

    let first = archives
        .index(&target, &archives.open(&target).expect("open"), &never, None)
        .expect("index");
    let again = archives
        .index(&target, &archives.open(&target).expect("open"), &never, None)
        .expect("index");
    assert!(Arc::ptr_eq(&first, &again), "kept");

    std::fs::write(
        folder.path().join("a.tar"),
        tar_of(&[item("a.txt", b"one"), item("b.txt", b"two")]),
    )
    .expect("changed");
    let changed = archives
        .index(&target, &archives.open(&target).expect("open"), &never, None)
        .expect("index");

    assert!(!Arc::ptr_eq(&first, &changed), "built again");
    assert!(changed.find("", "b.txt").is_some());
}

#[test]
fn a_password_fault_reaches_the_page_as_password_required() {
    let needed = archive_error(&archive::password_error(PasswordFault::Needed));
    let wrong = archive_error(&archive::password_error(PasswordFault::Wrong));

    assert_eq!(code(&needed), IpcErrorCode::PasswordRequired);
    assert!(!needed.message.to_lowercase().contains("wrong"));
    assert_eq!(code(&wrong), IpcErrorCode::PasswordRequired);
    assert!(wrong.message.to_lowercase().contains("wrong"));
}

#[test]
fn the_exit_flag_stops_the_reads_that_have_no_cancel_flag_of_their_own() {
    let (_folder, path) = archive_in_folder("a.tar", &tar_of(&[item("a.txt", b"one")]));
    let target = Target::parse(path).expect("valid");
    let archives = Archives::default();
    let opened = archives.open(&target).expect("open");
    assert!(!archives.stop().load(Ordering::Relaxed));
    archives
        .index(&target, &opened, archives.stop(), None)
        .expect("before the exit");

    // Clones share it: the listing thread's copy stops with the one the app raises.
    let clone = archives.clone();
    archives.shut_down();

    assert!(clone.stop().load(Ordering::Relaxed));
    let (_other, other_path) = archive_in_folder("b.tar", &tar_of(&[item("b.txt", b"two")]));
    let other = Target::parse(other_path).expect("valid");
    let error = clone
        .index(&other, &clone.open(&other).expect("open"), clone.stop(), None)
        .expect_err("stopped");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(code(&archive_error(&error)), IpcErrorCode::Io);
}

#[test]
fn a_missing_unrar_and_a_protected_rar_keep_their_own_texts_and_codes() {
    let missing = archive_error(&archive::rar::missing());
    let protected = archive_error(&archive::rar::protected());

    assert_eq!(missing.message, "Install unrar to open RAR archives");
    assert_eq!(code(&missing), IpcErrorCode::Unsupported);
    assert_eq!(code(&protected), IpcErrorCode::Unsupported);
    assert_eq!(protected.message, archive::rar::PASSWORD_TEXT);
}

#[test]
fn an_unrar_that_is_too_old_is_told_with_its_own_text_naming_the_version() {
    let known = archive_error(&archive::rar::too_old(Some("6.21")));
    let unknown = archive_error(&archive::rar::too_old(None));

    assert_eq!(code(&known), IpcErrorCode::Unsupported);
    assert_eq!(
        known.message,
        "The installed unrar 6.21 is not compatible. Update unrar to version 7 or newer to open RAR archives."
    );
    assert_eq!(code(&unknown), IpcErrorCode::Unsupported);
    assert_eq!(
        unknown.message,
        "The installed unrar is not compatible. Update unrar to version 7 or newer to open RAR archives."
    );
}

#[test]
fn an_error_that_is_not_ours_tells_only_its_kind() {
    let error = io::Error::other("/home/someone/secret/path.zip exploded");

    let told = archive_error(&error);

    assert_eq!(told.message, "the archive cannot be read");
    assert!(!told.message.contains("secret"));
}

//! Tests of `archive_view_open`: an entry opened as a session that reads the right bytes and leaves nothing on the disk,
//! what is refused, and the sweep of stale folders.

use pairpane_core::archive::Password;
use pairpane_core::model::ViewKind;
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::archives::fixtures::{
    archive_in_folder, declaring, encrypted, item, sevenz_with_encrypted_headers, tar_of, zip_of,
};

fn open_in(viewer: &Viewer, archives: &Archives, archive: &str, dir: &str, name: &str) -> CommandResult<ViewOpened> {
    block_on(viewer.open_archive_entry(archives, archive.to_owned(), dir.to_owned(), name.to_owned()))
}

/// The temporary folders of the viewer that hold a file called `name`.
fn folders_holding(name: &str) -> Vec<PathBuf> {
    let listing = fs::read_dir(std::env::temp_dir()).expect("temp folder");

    listing
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(PREFIX))
        .map(|entry| entry.path())
        .filter(|folder| folder.join(name).exists())
        .collect()
}

#[test]
fn a_tar_entry_opens_as_a_session_that_reads_its_bytes_and_leaves_no_file() {
    let tar = tar_of(&[
        item("docs/", b""),
        item("docs/pairpane-view-test-one.txt", b"hello from the archive\n"),
    ]);
    let (_folder, archive) = archive_in_folder("pack.tar", &tar);
    let (viewer, archives) = (Viewer::default(), Archives::default());

    let opened = open_in(&viewer, &archives, &archive, "docs", "pairpane-view-test-one.txt").expect("opened");

    assert_eq!(opened.name, "pairpane-view-test-one.txt");
    assert_eq!(opened.size, 23);
    assert_eq!(opened.kind, ViewKind::Text);
    let bytes = block_on(viewer.bytes(opened.session, 0, 64)).expect("read");
    assert_eq!(bytes, b"hello from the archive\n");
    assert!(
        folders_holding("pairpane-view-test-one.txt").is_empty(),
        "nothing is left behind"
    );
}

#[test]
fn a_stored_zip_entry_opens_too() {
    let zip = zip_of(&[item("pairpane-view-test-two.txt", b"zipped")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let (viewer, archives) = (Viewer::default(), Archives::default());

    let opened = open_in(&viewer, &archives, &archive, "", "pairpane-view-test-two.txt").expect("opened");

    assert_eq!(block_on(viewer.bytes(opened.session, 0, 16)).expect("read"), b"zipped");
    assert!(folders_holding("pairpane-view-test-two.txt").is_empty());
}

#[test]
fn an_entry_over_the_cap_is_refused_with_the_text_that_points_to_extraction() {
    let too_big = u32::try_from(MAX_VIEW_ENTRY).expect("fits in 32 bits") + 1;
    let zip = zip_of(&[declaring("pairpane-view-test-big.bin", too_big)]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);

    let error = open_in(
        &Viewer::default(),
        &Archives::default(),
        &archive,
        "",
        "pairpane-view-test-big.bin",
    )
    .expect_err("refused");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(folders_holding("pairpane-view-test-big.bin").is_empty());
}

#[test]
fn a_folder_a_missing_entry_and_an_encrypted_one_without_a_password_are_refused() {
    let zip = zip_of(&[
        item("dir/inner.txt", b"x"),
        encrypted("pairpane-view-test-locked.txt", b"cipher"),
    ]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);
    let (viewer, archives) = (Viewer::default(), Archives::default());

    let folder = open_in(&viewer, &archives, &archive, "", "dir").expect_err("a folder");
    assert_eq!(folder.code(), IpcErrorCode::InvalidInput);

    let missing = open_in(&viewer, &archives, &archive, "", "nothing.txt").expect_err("missing");
    assert_eq!(missing.code(), IpcErrorCode::NotFound);

    let locked = open_in(&viewer, &archives, &archive, "", "pairpane-view-test-locked.txt").expect_err("locked");
    assert_eq!(locked.code(), IpcErrorCode::PasswordRequired);
    assert!(folders_holding("pairpane-view-test-locked.txt").is_empty());
}

#[test]
fn names_that_are_not_plain_are_refused_before_a_thread_is_taken() {
    let (_folder, archive) = archive_in_folder("pack.tar", &tar_of(&[item("a.txt", b"x")]));
    let (viewer, archives) = (Viewer::default(), Archives::default());

    for name in ["", "..", "a/b", "x\0y"] {
        let error = open_in(&viewer, &archives, &archive, "", name).expect_err(name);
        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{name:?}");
    }
    let error = open_in(&viewer, &archives, "relative.tar", "", "a.txt").expect_err("relative");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
}

#[test]
fn the_scratch_folder_is_private_and_goes_away_with_its_file() {
    let scratch = Scratch::create("note.txt").expect("created");
    let folder = scratch.folder.clone();
    drop(scratch.open_file().expect("file"));

    assert!(folder.starts_with(std::env::temp_dir()));
    assert!(folder.file_name().expect("name").to_string_lossy().starts_with(PREFIX));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = |path: &Path| fs::metadata(path).expect("exists").permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&folder.join("note.txt")), 0o600);
    }
    assert!(
        scratch.open_file().is_err(),
        "the file is created new, never opened over another"
    );

    drop(scratch);

    assert!(!folder.exists());
}

#[test]
fn the_sweep_removes_stale_folders_of_the_viewer_and_nothing_else() {
    let temp = tempfile::tempdir().expect("a temporary folder of its own");
    let stale = temp.path().join(format!("{PREFIX}stale"));
    private_folder(&stale).expect("stale folder");
    fs::write(stale.join("left.txt"), "left behind").expect("file");
    let other = temp.path().join("pairpane-keep-me");
    fs::create_dir(&other).expect("another folder");

    sweep_in(temp.path());

    assert!(!stale.exists(), "the stale folder is gone");
    assert!(other.exists(), "other folders stay");
}

#[cfg(unix)]
#[test]
fn the_sweep_never_follows_a_link_named_like_a_viewer_folder() {
    let temp = tempfile::tempdir().expect("a temporary folder of its own");
    let target = tempfile::tempdir().expect("target");
    fs::write(target.path().join("keep.txt"), "x").expect("file");
    let link = temp.path().join(format!("{PREFIX}link"));
    std::os::unix::fs::symlink(target.path(), &link).expect("link");

    sweep_in(temp.path());

    assert!(
        target.path().join("keep.txt").exists(),
        "what the link leads to is untouched"
    );
}

#[test]
fn a_remembered_password_that_the_archive_turns_down_is_forgotten_so_the_page_asks_again() {
    let (_folder, archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let (viewer, archives) = (Viewer::default(), Archives::default());
    archives.remember(PathBuf::from(&archive), Password::new("not-the-password".to_owned()));

    let error = open_in(&viewer, &archives, &archive, "", "a.txt").expect_err("turned down");

    assert_eq!(error.code(), IpcErrorCode::PasswordRequired);
    assert!(archives.password(Path::new(&archive)).is_none());
}

#[test]
fn an_entry_is_viewed_only_where_it_leaves_the_reserve_on_the_temporary_volume() {
    let reserve = MIN_FREE_RESERVE;

    assert!(has_room(None, u64::MAX));
    assert!(has_room(Some(reserve + 10), 10));
    assert!(!has_room(Some(reserve + 9), 10));
    assert!(!has_room(Some(reserve), 1));
}

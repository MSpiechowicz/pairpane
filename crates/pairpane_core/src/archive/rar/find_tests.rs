//! Where `unrar` is looked for: fixed places first, then the absolute entries of `PATH`, and only a file that can be run.

#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::find::locate;

/// A folder holding an `unrar` with the mode given.
fn folder_with(mode: u32) -> (TempDir, PathBuf) {
    let folder = TempDir::new().expect("folder");
    let program = folder.path().join("unrar");
    fs::write(&program, b"#!").expect("file");
    fs::set_permissions(&program, fs::Permissions::from_mode(mode)).expect("mode");

    (folder, program)
}

fn path_of(folders: &[&Path]) -> OsString {
    std::env::join_paths(folders).expect("a path")
}

#[test]
fn a_fixed_place_is_taken_before_path() {
    let (_fixed_folder, fixed) = folder_with(0o755);
    let (path_folder, _) = folder_with(0o755);

    let found = locate(std::slice::from_ref(&fixed), Some(&path_of(&[path_folder.path()])));

    assert_eq!(found, Some(fixed));
}

#[test]
fn path_is_searched_in_order_and_only_in_absolute_folders() {
    let (first, _) = folder_with(0o644);
    let (second, wanted) = folder_with(0o755);

    // A relative entry and an empty one are never looked in, whatever the folder the app runs in holds.
    let value = path_of(&[Path::new("relative/bin"), Path::new(""), first.path(), second.path()]);

    assert_eq!(locate(&[], Some(&value)), Some(wanted));
    assert_eq!(
        locate(&[], Some(&path_of(&[Path::new("relative/bin"), Path::new("")]))),
        None
    );
}

#[test]
fn a_file_that_cannot_be_run_or_is_not_a_file_is_not_taken() {
    let (not_executable, _) = folder_with(0o644);
    let directory = TempDir::new().expect("folder");
    fs::create_dir(directory.path().join("unrar")).expect("a folder named unrar");

    let value = path_of(&[not_executable.path(), directory.path()]);

    assert_eq!(locate(&[], Some(&value)), None);
    assert_eq!(locate(&[directory.path().join("unrar")], None), None);
    assert_eq!(
        locate(&[PathBuf::from("unrar")], None),
        None,
        "a relative candidate is never taken"
    );
}

#[test]
fn nothing_is_found_without_candidates() {
    assert_eq!(locate(&[], None), None);
    assert_eq!(locate(&[PathBuf::from("/nonexistent/pairpane/unrar")], None), None);
}

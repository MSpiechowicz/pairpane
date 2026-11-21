//! Tests of the single-item changes: new folders and files, links and permanent delete.

use std::fs;
use std::io;
use std::path::Path;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::LinkKind;

use super::{MAX_FOLDER_DEPTH, MAX_NAME_UNITS, create_file, create_folders, folder_steps, is_runnable, make_link};

pub(super) fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write");
}

pub(super) fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

fn kind_of<T: std::fmt::Debug>(result: io::Result<T>) -> io::ErrorKind {
    result.expect_err("an error").kind()
}

/// Makes a symbolic link at `link` with the system's own call; `false` when this system does not allow it (Windows
/// without Developer Mode), and the test then has nothing to check.
pub(super) fn symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };

    made.is_ok()
}

#[test]
fn folder_steps_splits_a_path_into_its_parts() {
    assert_eq!(folder_steps("a").expect("one"), ["a"]);
    assert_eq!(folder_steps("a/b/c").expect("three"), ["a", "b", "c"]);
    assert_eq!(folder_steps("New folder/ä b").expect("spaces"), ["New folder", "ä b"]);
}

#[test]
fn folder_steps_refuses_empty_parts_and_names_that_are_not_plain() {
    for bad in [
        "", "/", "/a", "a/", "a//b", ".", "..", "a/./b", "a/../b", "a/\0", "a/b/",
    ] {
        assert_eq!(kind_of(folder_steps(bad)), io::ErrorKind::InvalidInput, "{bad:?}");
    }
}

#[test]
fn folder_steps_takes_a_backslash_as_a_separator_only_on_windows() {
    let result = folder_steps(r"a\b");

    if cfg!(windows) {
        assert_eq!(result.expect("two parts"), ["a", "b"]);
    } else {
        assert_eq!(result.expect("one name"), [r"a\b"]);
    }
}

#[test]
fn folder_steps_limits_the_depth_and_the_length_of_a_part() {
    let deepest = vec!["a"; MAX_FOLDER_DEPTH].join("/");
    let too_deep = vec!["a"; MAX_FOLDER_DEPTH + 1].join("/");
    let long = "a".repeat(MAX_NAME_UNITS);
    let too_long = "a".repeat(MAX_NAME_UNITS + 1);

    assert_eq!(folder_steps(&deepest).expect("fits").len(), MAX_FOLDER_DEPTH);
    assert_eq!(kind_of(folder_steps(&too_deep)), io::ErrorKind::InvalidInput);
    assert_eq!(folder_steps(&long).expect("fits").len(), 1);
    assert_eq!(kind_of(folder_steps(&too_long)), io::ErrorKind::InvalidInput);
}

#[test]
fn the_texts_of_a_refused_folder_path_hold_no_path() {
    for bad in ["/x/y/z", "a//b", ".."] {
        let text = folder_steps(bad).expect_err(bad).to_string();

        assert!(!text.contains(bad), "{text}");
    }
}

#[test]
fn create_folders_makes_nested_folders_and_returns_the_first_part() {
    let temp = TempDir::new().expect("temp dir");

    let first = create_folders(temp.path(), "a/b/c").expect("created");

    assert_eq!(first, "a");
    assert!(temp.path().join("a/b/c").is_dir());
}

#[test]
fn create_folders_makes_one_folder() {
    let temp = TempDir::new().expect("temp dir");

    assert_eq!(create_folders(temp.path(), "new").expect("created"), "new");
    assert!(temp.path().join("new").is_dir());
}

#[test]
fn create_folders_reuses_a_real_folder_on_the_way() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir(temp.path().join("a")).expect("folder");
    write(&temp.path().join("a/keep.txt"), "kept");

    let first = create_folders(temp.path(), "a/b").expect("created");

    assert_eq!(first, "a");
    assert!(temp.path().join("a/b").is_dir());
    assert_eq!(read(&temp.path().join("a/keep.txt")), "kept");
}

#[test]
fn create_folders_refuses_a_last_folder_that_exists() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir_all(temp.path().join("a/b")).expect("folders");

    assert_eq!(kind_of(create_folders(temp.path(), "a")), io::ErrorKind::AlreadyExists);
    assert_eq!(
        kind_of(create_folders(temp.path(), "a/b")),
        io::ErrorKind::AlreadyExists
    );
}

#[test]
fn create_folders_refuses_a_file_at_the_last_name() {
    let temp = TempDir::new().expect("temp dir");
    write(&temp.path().join("a"), "file");

    assert_eq!(kind_of(create_folders(temp.path(), "a")), io::ErrorKind::AlreadyExists);
    assert_eq!(read(&temp.path().join("a")), "file");
}

#[test]
fn create_folders_refuses_a_file_on_the_way_and_keeps_the_folders_made_before() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir(temp.path().join("a")).expect("folder");
    write(&temp.path().join("a/b"), "file");

    let result = create_folders(temp.path(), "a/b/c");

    assert_eq!(kind_of(result), io::ErrorKind::InvalidInput);
    assert_eq!(read(&temp.path().join("a/b")), "file");
}

#[test]
fn create_folders_never_goes_through_a_link_on_the_way() {
    let temp = TempDir::new().expect("temp dir");
    let outside = TempDir::new().expect("outside");
    if !symlink(outside.path(), &temp.path().join("link")) {
        return;
    }

    let result = create_folders(temp.path(), "link/inner");

    assert_eq!(kind_of(result), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(outside.path()).expect("folder").count(), 0);
}

#[test]
fn create_folders_refuses_a_last_name_that_is_a_link() {
    let temp = TempDir::new().expect("temp dir");
    let outside = TempDir::new().expect("outside");
    if !symlink(outside.path(), &temp.path().join("link")) {
        return;
    }

    assert_eq!(
        kind_of(create_folders(temp.path(), "link")),
        io::ErrorKind::AlreadyExists
    );
}

#[test]
fn create_folders_in_a_missing_folder_is_not_found() {
    let temp = TempDir::new().expect("temp dir");

    let result = create_folders(&temp.path().join("gone"), "a/b");

    assert_eq!(kind_of(result), io::ErrorKind::NotFound);
}

#[test]
fn create_file_makes_an_empty_file_and_returns_its_path() {
    let temp = TempDir::new().expect("temp dir");

    let path = create_file(temp.path(), "new.txt").expect("created");

    assert_eq!(path, temp.path().join("new.txt"));
    assert_eq!(fs::metadata(&path).expect("file").len(), 0);
}

#[test]
fn create_file_refuses_what_exists_and_leaves_it_alone() {
    let temp = TempDir::new().expect("temp dir");
    write(&temp.path().join("a.txt"), "kept");
    fs::create_dir(temp.path().join("dir")).expect("folder");

    assert_eq!(kind_of(create_file(temp.path(), "a.txt")), io::ErrorKind::AlreadyExists);
    assert_eq!(kind_of(create_file(temp.path(), "dir")), io::ErrorKind::AlreadyExists);
    assert_eq!(read(&temp.path().join("a.txt")), "kept");
}

#[test]
fn create_file_counts_a_dangling_link_as_taken_and_does_not_create_its_target() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    if !symlink(&target, &temp.path().join("dangling")) {
        return;
    }

    let result = create_file(temp.path(), "dangling");

    assert_eq!(kind_of(result), io::ErrorKind::AlreadyExists);
    assert!(!target.exists());
}

#[test]
fn create_file_refuses_a_name_that_is_not_plain() {
    let temp = TempDir::new().expect("temp dir");

    for bad in ["", ".", "..", "a/b"] {
        assert_eq!(
            kind_of(create_file(temp.path(), bad)),
            io::ErrorKind::InvalidInput,
            "{bad:?}"
        );
    }
}

#[test]
fn programs_and_scripts_are_runnable_whatever_the_case() {
    for name in [
        "a.exe",
        "a.COM",
        "a.Bat",
        "a.cmd",
        "a.ps1",
        "a.psm1",
        "a.vbs",
        "a.vbe",
        "a.js",
        "a.jse",
        "a.wsf",
        "a.wsh",
        "a.msi",
        "a.msc",
        "a.scr",
        "a.pif",
        "a.cpl",
        "a.hta",
        "a.lnk",
        "a.reg",
        "a.jar",
        "a.app",
        "a.command",
        "a.sh",
        "a.py",
        // Names that are only a dot and an extension: `Path::extension` finds none in them.
        ".bat",
        ".EXE",
        ".sh",
        "x.desktop",
        "x.terminal",
        "x.url",
        "x.appref-ms",
        "x.webloc",
        "x.AppImage",
        "x.pkg",
        "archive.tar.SH",
        "my file.Py",
    ] {
        assert!(is_runnable(name), "{name}");
    }
}

#[test]
fn documents_and_extensionless_names_are_not_runnable() {
    for name in [
        "a.txt",
        "a.pdf",
        "a.shx",
        "a.py.txt",
        "sh",
        "py",
        "a.",
        ".",
        "exe",
        "Makefile",
        "readme",
        "notes.txt",
        ".bashrc",
        "",
    ] {
        assert!(!is_runnable(name), "{name:?}");
    }
}

/// Makes the symbolic link `link` to `target` inside `folder`; `false` when Windows refuses links to this user.
fn symlinked(target: &Path, folder: &Path) -> bool {
    let made = make_link(LinkKind::Symbolic, target, folder, "link");
    if cfg!(windows) && made.is_err() {
        return false;
    }
    made.expect("link");

    true
}

#[test]
fn a_symbolic_link_holds_the_target_as_given() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    write(&target, "content");
    let to = TempDir::new().expect("to");

    if !symlinked(&target, to.path()) {
        return;
    }

    assert_eq!(fs::read_link(to.path().join("link")).expect("a link"), target);
    assert_eq!(read(&to.path().join("link")), "content");
}

#[test]
fn a_symbolic_link_to_a_folder_leads_into_it() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("folder");
    fs::create_dir(&target).expect("folder");
    write(&target.join("inner.txt"), "inner");
    let to = TempDir::new().expect("to");

    if !symlinked(&target, to.path()) {
        return;
    }

    assert_eq!(read(&to.path().join("link/inner.txt")), "inner");
}

#[test]
fn a_symbolic_link_to_nothing_is_made_too() {
    let temp = TempDir::new().expect("temp dir");
    let missing = temp.path().join("missing");

    let made = make_link(LinkKind::Symbolic, &missing, temp.path(), "link");
    if cfg!(windows) && made.is_err() {
        return;
    }
    made.expect("link");

    assert_eq!(fs::read_link(temp.path().join("link")).expect("a link"), missing);
}

#[test]
fn a_hard_link_is_the_same_file() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    write(&target, "before");
    let to = TempDir::new().expect("to");

    // The temporary folders can be on different disks (a tmpfs and a disk); then there is nothing to check.
    match make_link(LinkKind::Hard, &target, to.path(), "link") {
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => return,
        result => result.expect("link"),
    }
    write(&target, "after");

    assert_eq!(read(&to.path().join("link")), "after");
}

#[test]
fn a_hard_link_in_the_same_folder_is_made() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    write(&target, "content");

    make_link(LinkKind::Hard, &target, temp.path(), "other.txt").expect("link");

    assert_eq!(read(&temp.path().join("other.txt")), "content");
}

#[test]
fn a_hard_link_to_a_folder_is_refused() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir(temp.path().join("folder")).expect("folder");

    let result = make_link(LinkKind::Hard, &temp.path().join("folder"), temp.path(), "link");

    assert_eq!(kind_of(result), io::ErrorKind::InvalidInput);
    assert!(!temp.path().join("link").exists());
}

#[test]
fn a_hard_link_to_a_link_is_refused() {
    let temp = TempDir::new().expect("temp dir");
    write(&temp.path().join("target.txt"), "content");
    if !symlink(&temp.path().join("target.txt"), &temp.path().join("soft")) {
        return;
    }

    let result = make_link(LinkKind::Hard, &temp.path().join("soft"), temp.path(), "hard");

    assert_eq!(kind_of(result), io::ErrorKind::InvalidInput);
    assert!(fs::symlink_metadata(temp.path().join("hard")).is_err());
}

#[test]
fn a_hard_link_to_a_missing_file_is_not_found() {
    let temp = TempDir::new().expect("temp dir");

    let result = make_link(LinkKind::Hard, &temp.path().join("missing"), temp.path(), "link");

    assert_eq!(kind_of(result), io::ErrorKind::NotFound);
}

#[test]
fn a_link_name_that_is_taken_is_refused_for_both_kinds() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    write(&target, "content");
    write(&temp.path().join("taken"), "mine");

    for kind in [LinkKind::Symbolic, LinkKind::Hard] {
        let result = make_link(kind, &target, temp.path(), "taken");

        assert_eq!(kind_of(result), io::ErrorKind::AlreadyExists, "{kind:?}");
    }
    assert_eq!(read(&temp.path().join("taken")), "mine");
}

#[test]
fn a_link_with_a_name_that_is_not_plain_or_a_relative_target_is_refused() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    write(&target, "content");

    for kind in [LinkKind::Symbolic, LinkKind::Hard] {
        for bad in ["", "..", "x/y"] {
            let result = make_link(kind, &target, temp.path(), bad);

            assert_eq!(kind_of(result), io::ErrorKind::InvalidInput, "{kind:?} {bad:?}");
        }

        let relative = make_link(kind, Path::new("target.txt"), temp.path(), "link");
        assert_eq!(kind_of(relative), io::ErrorKind::InvalidInput, "{kind:?}");
    }
}

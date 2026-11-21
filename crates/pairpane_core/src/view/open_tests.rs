//! Opening for the viewer: what is refused, what is told, and that no text names a path.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::ViewKind;

use super::open;
use super::testing::{put, temp};

fn refusal(folder: &Path, name: &str) -> (ErrorKind, String) {
    let error = open(folder, name).expect_err("refused");

    (error.kind(), error.to_string())
}

#[test]
fn opens_a_regular_file_with_what_is_known_of_it() {
    let folder = temp();
    put(folder.path(), "notes.txt", "zażółć\n".as_bytes());

    let opened = open(folder.path(), "notes.txt").expect("open");

    assert_eq!(opened.name, "notes.txt");
    assert_eq!(opened.size, "zażółć\n".len() as u64);
    assert_eq!(opened.kind, ViewKind::Text);
    assert_eq!(opened.encoding, "UTF-8");
    assert!(opened.modified.is_some_and(|millis| millis > 1_000_000_000_000));
}

#[test]
fn only_text_and_markdown_name_an_encoding() {
    let folder = temp();
    put(folder.path(), "a.md", b"# title\n");
    put(folder.path(), "b.bin", &[0, 1, 2, 3]);

    assert_eq!(open(folder.path(), "a.md").expect("open").encoding, "UTF-8");
    assert_eq!(open(folder.path(), "b.bin").expect("open").encoding, "");
}

#[test]
fn refuses_a_folder_with_a_fixed_text() {
    let folder = temp();
    fs::create_dir(folder.path().join("inner")).expect("folder");

    assert_eq!(
        refusal(folder.path(), "inner"),
        (
            ErrorKind::InvalidInput,
            "the viewer opens files, not folders".to_owned()
        )
    );
}

#[test]
fn a_missing_file_is_not_found() {
    let folder = temp();

    let error = open(folder.path(), "gone.txt").expect_err("missing");

    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn refuses_names_that_are_not_one_plain_name() {
    let folder = temp();
    put(folder.path(), "a.txt", b"a");

    for name in ["", ".", "..", "../a.txt", "sub/a.txt", "a\0.txt"] {
        assert_eq!(
            refusal(folder.path(), name),
            (ErrorKind::InvalidInput, "the name cannot be opened".to_owned()),
            "{name:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn refuses_a_link_to_a_file_and_to_a_folder_and_a_dangling_one() {
    let folder = temp();
    put(folder.path(), "target.txt", b"x");
    fs::create_dir(folder.path().join("dir")).expect("folder");
    std::os::unix::fs::symlink("target.txt", folder.path().join("to-file")).expect("link");
    std::os::unix::fs::symlink("dir", folder.path().join("to-dir")).expect("link");
    std::os::unix::fs::symlink("nowhere", folder.path().join("dangling")).expect("link");

    for name in ["to-file", "to-dir", "dangling"] {
        assert_eq!(
            refusal(folder.path(), name),
            (
                ErrorKind::InvalidInput,
                "the viewer opens regular files, not links".to_owned()
            ),
            "{name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn refuses_a_pipe_without_waiting_for_a_writer() {
    let folder = temp();
    let status = std::process::Command::new("mkfifo")
        .arg(folder.path().join("pipe"))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());

    // A pipe without a writer would block an ordinary open for ever; this returns.
    assert_eq!(
        refusal(folder.path(), "pipe"),
        (
            ErrorKind::InvalidInput,
            "the viewer opens regular files only".to_owned()
        )
    );
}

#[cfg(unix)]
#[test]
fn a_folder_that_is_a_link_is_followed_but_a_file_in_it_is_not_a_link() {
    let real = temp();
    put(real.path(), "a.txt", b"inside");
    let outer = temp();
    std::os::unix::fs::symlink(real.path(), outer.path().join("shown")).expect("link");

    let opened = open(&outer.path().join("shown"), "a.txt").expect("open");

    assert_eq!(opened.file.read_at(0, 10).expect("read"), b"inside");
}

#[test]
fn no_refusal_names_the_folder_or_the_file() {
    let folder = temp();
    fs::create_dir(folder.path().join("secret-folder")).expect("folder");
    let secret = folder.path().to_string_lossy().into_owned();

    for name in ["secret-folder", "secret-missing.txt", "..", "a/b"] {
        let text = open(folder.path(), name).expect_err("refused").to_string();

        assert!(!text.contains(&secret), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }
}

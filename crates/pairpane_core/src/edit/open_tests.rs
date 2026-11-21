//! Opening for the editor: what comes back for each kind of file, what is refused, and that no text names a path.

use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{EditOpened, LineEnding};

use super::testing::{document, put, temp};
use super::{MAX_EDIT, check_regular, open};

fn refusal(folder: &Path, name: &str) -> (ErrorKind, String) {
    let error = open(folder, name, None).expect_err("refused");

    (error.kind(), error.to_string())
}

#[test]
fn a_text_file_opens_with_everything_needed_to_save_it_the_same() {
    let folder = temp();
    put(folder.path(), "a.txt", "zażółć\r\nline\r\n".as_bytes());

    let document = document(folder.path(), "a.txt");

    assert_eq!(document.name, "a.txt");
    assert_eq!(document.text, "zażółć\nline\n");
    assert_eq!(document.encoding, "UTF-8");
    assert!(!document.bom);
    assert_eq!(document.line_ending, LineEnding::Crlf);
    assert!(!document.mixed_endings);
    assert_eq!(document.size, "zażółć\r\nline\r\n".len() as u64);
    assert!(!document.revision.is_empty());
}

#[test]
fn a_mark_and_a_mix_of_line_breaks_are_told() {
    let folder = temp();
    put(
        folder.path(),
        "a.txt",
        &[&[0xEF, 0xBB, 0xBF][..], b"a\nb\r\nc\n"].concat(),
    );

    let document = document(folder.path(), "a.txt");

    assert!(document.bom);
    assert_eq!(document.text, "a\nb\nc\n");
    assert_eq!(document.line_ending, LineEnding::Lf);
    assert!(document.mixed_endings);
}

#[test]
fn a_file_with_no_break_is_lf_and_an_empty_file_is_text() {
    let folder = temp();
    put(folder.path(), "one.txt", b"one line");
    put(folder.path(), "empty.txt", b"");

    assert_eq!(document(folder.path(), "one.txt").line_ending, LineEnding::Lf);
    assert_eq!(document(folder.path(), "empty.txt").text, "");
}

#[test]
fn the_same_file_has_the_same_revision_until_it_changes() {
    let folder = temp();
    put(folder.path(), "a.txt", b"one");

    let first = document(folder.path(), "a.txt").revision;
    assert_eq!(document(folder.path(), "a.txt").revision, first);

    put(folder.path(), "a.txt", b"one two");
    assert_ne!(document(folder.path(), "a.txt").revision, first);
}

#[test]
fn a_file_with_a_nul_byte_is_binary_and_utf16_is_text() {
    let folder = temp();
    put(folder.path(), "image.bin", b"ab\0cd");
    put(folder.path(), "wide.txt", &[0xFF, 0xFE, b'h', 0, b'i', 0]);

    assert_eq!(
        open(folder.path(), "image.bin", None).expect("open"),
        EditOpened::Binary
    );
    assert_eq!(document(folder.path(), "wide.txt").text, "hi");
    assert_eq!(document(folder.path(), "wide.txt").encoding, "UTF-16LE");
}

#[test]
fn bytes_that_are_not_valid_open_nothing_and_another_encoding_can_be_asked_for() {
    let folder = temp();
    // Valid UTF-8 text with one byte cut out of a character at the end.
    let mut bytes = "zażółć gęślą jaźń ".repeat(20).into_bytes();
    bytes.push(0xC5);
    put(folder.path(), "a.txt", &bytes);

    let asked = open(folder.path(), "a.txt", Some("UTF-8")).expect("open");
    assert_eq!(
        asked,
        EditOpened::Undecodable {
            encoding: "UTF-8".to_owned()
        }
    );

    let as_latin2 = open(folder.path(), "a.txt", Some("windows-1250")).expect("open");
    let EditOpened::Text { document } = as_latin2 else {
        panic!("not text");
    };
    assert_eq!(document.encoding, "windows-1250");
    assert!(!document.text.contains('\u{FFFD}'));
}

#[test]
fn a_label_that_is_not_known_is_refused_before_the_file_is_read() {
    let folder = temp();

    let error = open(folder.path(), "gone.txt", Some("klingon")).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the encoding is not known");
}

#[test]
fn a_file_over_the_cap_is_told_with_its_size_and_one_at_the_cap_opens() {
    let folder = temp();
    let big = File::create(folder.path().join("big.txt")).expect("file");
    big.set_len(MAX_EDIT as u64 + 1).expect("sparse");
    let at_cap = File::create(folder.path().join("cap.txt")).expect("file");
    at_cap.set_len(MAX_EDIT as u64).expect("sparse");

    assert_eq!(
        open(folder.path(), "big.txt", None).expect("open"),
        EditOpened::TooLarge {
            size: MAX_EDIT as u64 + 1
        }
    );
    // A sparse file reads as zeros: at the cap it is read, and binary.
    assert_eq!(open(folder.path(), "cap.txt", None).expect("open"), EditOpened::Binary);
}

#[test]
fn a_folder_is_refused_with_a_fixed_text() {
    let folder = temp();
    fs::create_dir(folder.path().join("inner")).expect("folder");

    assert_eq!(
        refusal(folder.path(), "inner"),
        (
            ErrorKind::InvalidInput,
            "the editor opens files, not folders".to_owned()
        )
    );
}

#[test]
fn a_missing_file_is_not_found_and_a_bad_name_is_refused() {
    let folder = temp();

    assert_eq!(
        open(folder.path(), "gone.txt", None).expect_err("missing").kind(),
        ErrorKind::NotFound
    );

    for name in ["", ".", "..", "../a.txt", "sub/a.txt", "a\0.txt"] {
        let (kind, text) = refusal(folder.path(), name);

        assert_eq!(kind, ErrorKind::InvalidInput, "{name:?}");
        assert_eq!(text, "the name cannot be opened", "{name:?}");
    }
}

#[cfg(unix)]
#[test]
fn links_are_refused_and_so_is_a_pipe_without_waiting_for_a_writer() {
    use std::os::unix::fs::symlink;

    use crate::edit::testing::fifo;

    let folder = temp();
    put(folder.path(), "target.txt", b"x");
    symlink("target.txt", folder.path().join("link")).expect("link");
    symlink("nowhere", folder.path().join("dangling")).expect("link");
    fifo(folder.path(), "pipe");

    for name in ["link", "dangling"] {
        assert_eq!(
            refusal(folder.path(), name),
            (
                ErrorKind::InvalidInput,
                "the editor opens regular files, not links".to_owned()
            ),
            "{name}"
        );
    }
    assert_eq!(
        refusal(folder.path(), "pipe"),
        (
            ErrorKind::InvalidInput,
            "the editor opens regular files only".to_owned()
        )
    );
}

#[cfg(unix)]
#[test]
fn a_file_the_user_cannot_write_opens_read_only() {
    use std::os::unix::fs::PermissionsExt;

    use super::testing::is_root;

    if is_root() {
        return;
    }

    let folder = temp();
    put(folder.path(), "a.txt", b"fixed");
    put(folder.path(), "b.txt", b"free");
    fs::set_permissions(folder.path().join("a.txt"), fs::Permissions::from_mode(0o444)).expect("chmod");

    assert!(document(folder.path(), "a.txt").read_only);
    assert!(!document(folder.path(), "b.txt").read_only);
}

#[test]
fn no_refusal_names_the_folder_or_the_file() {
    let folder = temp();
    fs::create_dir(folder.path().join("secret-folder")).expect("folder");

    for name in ["secret-folder", "secret-missing.txt", "..", "a/b"] {
        let text =
            open(folder.path(), name, None).map_or_else(|error| error.to_string(), |opened| format!("{opened:?}"));

        assert!(!text.contains("secret"), "{text}");
    }
}

#[test]
fn an_external_editor_is_given_files_only() {
    let folder = temp();
    put(folder.path(), "a.txt", b"x");
    fs::create_dir(folder.path().join("inner")).expect("folder");

    check_regular(&folder.path().join("a.txt")).expect("a file");

    let error = check_regular(&folder.path().join("inner")).expect_err("refused");
    assert_eq!(error.to_string(), "the editor opens files, not folders");
    let gone = check_regular(&folder.path().join("gone")).expect_err("missing");
    assert_eq!(gone.kind(), ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn a_link_to_a_file_is_followed_for_an_external_editor_and_a_link_to_a_folder_is_not() {
    let folder = temp();
    put(folder.path(), "a.txt", b"x");
    fs::create_dir(folder.path().join("inner")).expect("folder");
    std::os::unix::fs::symlink("a.txt", folder.path().join("to-file")).expect("link");
    std::os::unix::fs::symlink("inner", folder.path().join("to-dir")).expect("link");

    check_regular(&folder.path().join("to-file")).expect("a link to a file");
    assert!(check_regular(&folder.path().join("to-dir")).is_err());
}

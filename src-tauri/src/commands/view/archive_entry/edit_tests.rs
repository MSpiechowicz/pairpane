//! Tests of `archive_edit_open`: an entry of an archive read as text for a comparison, read-only, and what is told when it
//! is not text, too big or not there.

use pairpane_core::model::{EditOpened, IpcErrorCode};
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::archives::fixtures::{archive_in_folder, item, tar_of, zip_of};

fn read(archive: &str, dir: &str, name: &str, most: u64) -> CommandResult<EditOpened> {
    block_on(edit_entry(
        &Archives::default(),
        (archive.to_owned(), dir.to_owned(), name.to_owned()),
        None,
        most,
    ))
}

#[test]
fn a_text_entry_is_read_as_text_that_cannot_be_saved() {
    let tar = tar_of(&[item("docs/", b""), item("docs/notes.txt", b"first\r\nsecond\r\n")]);
    let (_folder, archive) = archive_in_folder("pack.tar", &tar);

    let opened = read(&archive, "docs", "notes.txt", 1024).expect("read");

    let EditOpened::Text { document } = opened else {
        panic!("text: {opened:?}");
    };
    assert_eq!(document.text, "first\nsecond\n");
    assert!(document.read_only);
    assert_eq!(document.name, "notes.txt");
}

#[test]
fn bytes_too_many_bytes_and_a_missing_or_folder_entry_are_told_apart() {
    let zip = zip_of(&[
        item("bin.dat", b"a\0b\0c\0\0\0"),
        item("big.txt", &[b'x'; 64]),
        item("dir/", b""),
    ]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);

    assert_eq!(read(&archive, "", "bin.dat", 1024).expect("read"), EditOpened::Binary);
    assert!(matches!(
        read(&archive, "", "big.txt", 63).expect("read"),
        EditOpened::TooLarge { .. }
    ));
    assert_eq!(
        read(&archive, "", "gone.txt", 1024).expect_err("gone").code(),
        IpcErrorCode::NotFound
    );
    assert!(read(&archive, "", "dir", 1024).is_err());
    assert_eq!(
        read(&archive, "", "../x", 1024).expect_err("not plain").code(),
        IpcErrorCode::InvalidInput
    );
}

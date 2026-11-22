//! The tar.zst writer: what its headers say (checked with the `tar` crate's reader), and files that change under it.

use std::io::{self, Cursor, Read};
use std::sync::atomic::AtomicBool;

use crate::archive::testing::{MOMENT, idle, noise};
use crate::model::{ArchiveFormat, PackFormat, PackLevel};

use super::testing::{options, pack, read_back, sample};
use super::{Entry, TarWriter};

#[test]
fn the_headers_hold_no_owner_and_no_names_of_users() {
    let items = sample();
    let bytes = pack(PackFormat::TarZst, &options(PackLevel::Normal, None), &items).expect("packed");
    let tar = zstd::decode_all(bytes.as_slice()).expect("a Zstandard frame");
    let mut archive = tar::Archive::new(tar.as_slice());
    let mut seen = Vec::new();

    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("an entry");
        let header = entry.header().clone();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).expect("content");

        assert_eq!((header.uid().expect("uid"), header.gid().expect("gid")), (0, 0));
        assert!(header.username_bytes().is_none_or(<[u8]>::is_empty));
        assert!(header.groupname_bytes().is_none_or(<[u8]>::is_empty));
        assert_eq!(header.mtime().expect("mtime"), (MOMENT / 1000) as u64);

        seen.push((
            entry.path().expect("a path").to_string_lossy().into_owned(),
            header.entry_type().is_dir(),
            header.mode().expect("mode"),
            content,
        ));
    }

    let wanted: Vec<_> = items
        .iter()
        .map(|item| {
            let dir = item.content.is_none();

            (
                if dir {
                    format!("{}/", item.name)
                } else {
                    item.name.clone()
                },
                dir,
                item.mode & 0o777,
                item.content.clone().unwrap_or_default(),
            )
        })
        .collect();

    // The long name (over 100 bytes) is in the GNU extension and reads back whole.
    assert!(items.iter().any(|item| item.name.len() > 200));
    assert_eq!(seen, wanted);
}

#[test]
fn a_file_that_ends_before_its_declared_size_fails_the_pack() {
    let cancel = idle();
    let mut writer = TarWriter::new(Cursor::new(Vec::new()), PackLevel::Fast, &cancel).expect("a writer");
    let content = noise(40);

    let error = writer
        .add(Entry::file("short.bin", 0o644, None, 100, content.as_slice()))
        .expect_err("fails");

    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn a_file_that_grew_is_cut_at_its_declared_size() {
    let cancel = AtomicBool::new(false);
    let mut writer = TarWriter::new(Cursor::new(Vec::new()), PackLevel::Fast, &cancel).expect("a writer");
    let content = noise(1000);

    writer
        .add(Entry::file("grew.bin", 0o644, Some(MOMENT), 600, content.as_slice()))
        .expect("added");
    writer
        .add(Entry::file("after.txt", 0o644, Some(MOMENT), 5, &b"after"[..]))
        .expect("added");
    let bytes = writer.finish().expect("finished").into_inner();

    let seen = read_back(&bytes, ArchiveFormat::TarZst, None).expect("read back");

    assert_eq!(seen[0].content, content[..600]);
    assert_eq!(seen[1].content, b"after");
}

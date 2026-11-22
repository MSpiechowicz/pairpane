//! A gzip file as an archive of one entry: its name, size and time, and the bytes it inflates to.

use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use flate2::{Compression, GzBuilder};
use pretty_assertions::assert_eq;

use crate::model::ArchiveFormat;

use super::entries::{EntryType, Place};
use super::index::ArchiveIndex;
use super::{EntrySource, Input, build_index, open_source};

fn gz(name: Option<&str>, mtime: u32, content: &[u8]) -> Vec<u8> {
    let mut builder = GzBuilder::new().mtime(mtime);

    if let Some(name) = name {
        builder = builder.filename(name);
    }

    let mut encoder = builder.write(Vec::new(), Compression::fast());
    encoder.write_all(content).expect("compress");

    encoder.finish().expect("finish")
}

fn index(bytes: &Vec<u8>, path: Option<&str>) -> io::Result<ArchiveIndex> {
    let input = Input {
        file: bytes,
        path: path.map(Path::new),
    };

    build_index(ArchiveFormat::Gz, &input, &AtomicBool::new(false), None)
}

fn only_name(bytes: &Vec<u8>, path: Option<&str>) -> String {
    let index = index(bytes, path).expect("an index");

    assert_eq!(index.entries().len(), 1);

    index.entries()[0].path.clone()
}

#[test]
fn the_entry_has_the_name_the_header_holds_the_size_of_the_trailer_and_the_time() {
    let bytes = gz(Some("report.txt"), 1_700_000_000, &[b'x'; 1234]);

    let index = index(&bytes, Some("/data/whatever.gz")).expect("an index");

    let entry = &index.entries()[0];
    assert_eq!(
        (entry.path.as_str(), entry.size, entry.kind),
        ("report.txt", 1234, EntryType::File)
    );
    assert_eq!(entry.modified, Some(1_700_000_000_000));
    assert_eq!(entry.place, Place::Gz);
    assert!(!entry.encrypted && entry.writable());
    assert_eq!(index.format(), ArchiveFormat::Gz);
    assert!(!index.truncated());
}

#[test]
fn without_a_usable_header_name_the_archive_name_without_gz_is_used() {
    let bytes = gz(None, 0, b"data");

    assert_eq!(only_name(&bytes, Some("/data/notes.txt.gz")), "notes.txt");
    assert_eq!(only_name(&bytes, Some("/data/NOTES.TXT.GZ")), "NOTES.TXT");
    assert_eq!(only_name(&bytes, Some("/data/plain")), "plain");
    assert_eq!(only_name(&bytes, None), "file");
    assert_eq!(
        index(&bytes, Some("/d/a.gz")).expect("an index").entries()[0].modified,
        None
    );

    for unsafe_name in ["../evil", "/etc/passwd", "a/b", "a\\b", "esc\u{1b}x", "x\ny", "..", ""] {
        let bytes = gz(Some(unsafe_name), 0, b"data");

        assert_eq!(only_name(&bytes, Some("/data/safe.gz")), "safe", "{unsafe_name:?}");
    }
}

#[test]
fn what_is_not_gzip_or_is_cut_in_its_header_is_unreadable() {
    let good = gz(Some("a.txt"), 0, b"hello");

    for bytes in [
        Vec::new(),
        b"plain text that is no gzip file at all".to_vec(),
        good[..6].to_vec(),
        // A reserved flag bit.
        [&good[..3], &[0xE0], &good[4..]].concat(),
    ] {
        let error = index(&bytes, None).expect_err("unreadable");

        assert_eq!(error.to_string(), "the archive cannot be read", "{}", bytes.len());
    }
}

#[test]
fn the_source_inflates_the_entry_and_stops_a_bomb() {
    let bytes = gz(Some("a.txt"), 0, &vec![7; 100_000]);
    let index = index(&bytes, None).expect("an index");
    let input = Input {
        file: &bytes,
        path: None,
    };
    let cancel = AtomicBool::new(false);

    let mut inflated = Vec::new();
    open_source(&index, &input, &cancel, None)
        .expect("a source")
        .extract(&[0], &mut |_, data| {
            data.read_to_end(&mut inflated)?;

            Ok(super::Flow::Continue)
        })
        .expect("extracted");
    assert_eq!(inflated, vec![7; 100_000]);

    // The same file read with a limit below its inflated size fails at the limit, not silently short.
    let error = super::gz::GzSource::new(&index, &bytes, &cancel, 5_000)
        .extract(&[0], &mut |_, data| {
            data.read_to_end(&mut Vec::new()).map(|_| super::Flow::Continue)
        })
        .expect_err("too much data");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

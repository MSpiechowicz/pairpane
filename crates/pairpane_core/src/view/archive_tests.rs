//! Listing zip, tar and tar.gz files: names, sizes, times, the caps, and archives that are broken.

use std::io::{ErrorKind, Write};

use flate2::Compression;
use flate2::write::GzEncoder;
use pretty_assertions::assert_eq;

use crate::model::{ArchiveEntry, ArchiveFormat, ArchiveListing};

use super::testing::{file_of, idle, opened};
use super::{MAX_ARCHIVE_ENTRIES, ViewFile, archive};

/// 2021-03-04 05:06:08 as a zip stores it (MS-DOS date and time).
const DOS_DATE: u16 = 21092;
const DOS_TIME: u16 = 10436;
/// The same moment in milliseconds since the epoch, read as UTC.
pub(super) const MOMENT: i64 = 1_614_834_368_000;

/// A zip of stored (not compressed) entries, written by hand: `(name, content)`; a name ending in `/` is a folder.
pub(super) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut directory = Vec::new();

    for (name, content) in entries {
        let offset = bytes.len() as u32;
        let crc = crc32fast::hash(content);
        let size = content.len() as u32;

        bytes.extend_from_slice(b"PK\x03\x04");
        for field in [20u16, 0, 0, DOS_TIME, DOS_DATE] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        for field in [crc, size, size] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(content);

        directory.extend_from_slice(b"PK\x01\x02");
        for field in [20u16, 20, 0, 0, DOS_TIME, DOS_DATE] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        for field in [crc, size, size] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        for field in [0u16, 0, 0, 0] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }

    let start = bytes.len() as u32;
    bytes.extend_from_slice(&directory);
    bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&start.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    bytes
}

/// A tar of `(path, content)`, a path ending in `/` being a folder, all modified at `MOMENT`.
pub(super) fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());

    for (path, content) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mtime((MOMENT / 1000) as u64);
        header.set_mode(0o644);
        header.set_entry_type(if path.ends_with('/') {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        });
        builder.append_data(&mut header, path, *content).expect("append");
    }

    builder.into_inner().expect("a tar")
}

pub(super) fn many_names(count: usize) -> Vec<String> {
    (0..count).map(|number| format!("f{number}")).collect()
}

pub(super) fn empty_files(names: &[String]) -> Vec<(&str, &[u8])> {
    names.iter().map(|name| (name.as_str(), b"".as_slice())).collect()
}

pub(super) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).expect("compress");

    encoder.finish().expect("finish")
}

pub(super) fn list(bytes: &[u8], format: ArchiveFormat) -> ArchiveListing {
    let (_folder, file) = file_of(bytes);

    archive(&file, format, &idle()).expect("a listing")
}

pub(super) fn failure(file: &ViewFile, format: ArchiveFormat) -> (ErrorKind, String) {
    let error = archive(file, format, &idle()).expect_err("unreadable");

    (error.kind(), error.to_string())
}

pub(super) fn entry(name: &str, size: u64, dir: bool, modified: Option<i64>) -> ArchiveEntry {
    ArchiveEntry {
        name: name.to_owned(),
        size,
        dir,
        modified,
    }
}

#[test]
fn lists_a_zip_from_its_central_directory() {
    let bytes = zip_of(&[("docs/", b""), ("docs/readme.txt", b"hello"), ("zażółć.bin", &[7; 300])]);

    let listing = list(&bytes, ArchiveFormat::Zip);

    assert_eq!(listing.format, ArchiveFormat::Zip);
    assert!(!listing.truncated);
    assert_eq!(
        listing.entries,
        [
            entry("docs/", 0, true, Some(MOMENT)),
            entry("docs/readme.txt", 5, false, Some(MOMENT)),
            // Without the UTF-8 flag, a name that is valid UTF-8 is still taken as UTF-8.
            entry("zażółć.bin", 300, false, Some(MOMENT)),
        ]
    );
}

#[test]
fn an_empty_zip_has_no_entries() {
    let listing = list(&zip_of(&[]), ArchiveFormat::Zip);

    assert_eq!(listing.entries, []);
    assert!(!listing.truncated);
}

#[test]
fn a_zip_with_more_entries_than_the_cap_is_cut() {
    // 65,535 is the count that a zip without a zip64 record can hold: it is a count, not a sign to look further.
    for count in [MAX_ARCHIVE_ENTRIES + 5, usize::from(u16::MAX)] {
        let names = many_names(count);

        let listing = list(&zip_of(&empty_files(&names)), ArchiveFormat::Zip);

        assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES, "{count}");
        assert!(listing.truncated, "{count}");
        assert_eq!(listing.entries[0].name, "f0");
        assert_eq!(
            listing.entries[MAX_ARCHIVE_ENTRIES - 1].name,
            format!("f{}", MAX_ARCHIVE_ENTRIES - 1)
        );
    }
}

#[test]
fn a_zip_with_comment_and_leading_bytes_is_listed() {
    let mut bytes = b"leading bytes before the archive\n".to_vec();
    bytes.extend_from_slice(&zip_of(&[("a.txt", b"x")]));

    let listing = list(&bytes, ArchiveFormat::Zip);

    assert_eq!(
        listing.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a.txt"]
    );
}

#[test]
fn a_broken_zip_is_an_error_with_a_fixed_text() {
    let unreadable = (ErrorKind::InvalidData, "the archive cannot be read".to_owned());
    let good = zip_of(&[("a.txt", b"hello"), ("b.txt", b"world")]);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not a zip", b"this is not a zip file at all".to_vec()),
        ("empty", Vec::new()),
        ("only a signature", b"PK\x03\x04".to_vec()),
        ("cut in half", good[..good.len() / 2].to_vec()),
        ("directory cut off", good[..good.len() - 30].to_vec()),
        ("zeros", vec![0; 5000]),
    ];

    for (what, bytes) in cases {
        let (_folder, file) = file_of(&bytes);

        assert_eq!(failure(&file, ArchiveFormat::Zip), unreadable, "{what}");
    }
}

#[test]
fn a_zip_with_a_garbled_directory_is_an_error_or_a_listing_never_a_panic() {
    let good = zip_of(&[("a.txt", b"hello"), ("b.txt", b"world"), ("dir/", b"")]);

    for index in 0..good.len() {
        let mut bytes = good.clone();
        bytes[index] ^= 0xFF;
        let (_folder, file) = file_of(&bytes);

        let _ = archive(&file, ArchiveFormat::Zip, &idle());
    }
}

#[test]
fn a_zip_that_declares_a_huge_directory_or_too_many_entries_is_refused() {
    let mut huge = zip_of(&[("a.txt", b"x")]);
    let at = huge.len() - 22;
    huge[at + 12..at + 16].copy_from_slice(&(200 * 1024 * 1024u32).to_le_bytes());
    let (_folder, file) = file_of(&huge);
    assert_eq!(
        failure(&file, ArchiveFormat::Zip),
        (
            ErrorKind::InvalidData,
            "the archive's directory is too large to list".to_owned()
        )
    );

    let mut many = zip_of(&[("a.txt", b"x")]);
    let at = many.len() - 22;
    many[at + 10..at + 12].copy_from_slice(&60_000u16.to_le_bytes());
    let (_folder, file) = file_of(&many);
    assert_eq!(
        failure(&file, ArchiveFormat::Zip),
        (
            ErrorKind::InvalidData,
            "the archive declares too many entries to list".to_owned()
        )
    );
}

/// The zip64 end record, its locator and a zip32 end record whose fields say "see the zip64 record", then `tail`,
/// put in place of the end record of `plain`. The zip64 record says `entries` entries.
pub(super) fn zip64_then(plain: &[u8], entries: u64, tail: &[u8]) -> Vec<u8> {
    let eocd = plain.len() - 22;
    let field = |at: usize| u64::from(u32::from_le_bytes(plain[at..at + 4].try_into().expect("four bytes")));
    let (size, start) = (field(eocd + 12), field(eocd + 16));

    let mut bytes = plain[..eocd].to_vec();
    let record_at = bytes.len() as u64;
    bytes.extend_from_slice(b"PK\x06\x06");
    bytes.extend_from_slice(&44u64.to_le_bytes());
    bytes.extend_from_slice(&[45, 0, 45, 0]);
    bytes.extend_from_slice(&[0; 8]);
    for value in [entries, entries, size, start] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(b"PK\x06\x07");
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&record_at.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&plain[eocd..eocd + 10]);
    bytes.extend_from_slice(&[0xFF; 10]);
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(tail);

    bytes
}

#[test]
fn a_zip64_end_record_is_followed() {
    // The 16-bit count says "look in the zip64 record"; the record says 1 entry and the real directory size.
    let (_folder, file) = file_of(&zip64_then(&zip_of(&[("a.txt", b"x")]), 1, b""));

    let listing = archive(&file, ArchiveFormat::Zip, &idle()).expect("a listing");

    assert_eq!(listing.entries, [entry("a.txt", 1, false, Some(MOMENT))]);
}

#[test]
fn lists_a_tar() {
    let bytes = tar_of(&[
        ("src/", b""),
        ("src/main.rs", b"fn main() {}\n"),
        ("a long.txt", &[1; 1000]),
    ]);

    let listing = list(&bytes, ArchiveFormat::Tar);

    assert_eq!(listing.format, ArchiveFormat::Tar);
    assert!(!listing.truncated);
    assert_eq!(
        listing.entries,
        [
            entry("src/", 0, true, Some(MOMENT)),
            entry("src/main.rs", 13, false, Some(MOMENT)),
            entry("a long.txt", 1000, false, Some(MOMENT)),
        ]
    );
}

#[test]
fn lists_a_tar_gz() {
    let bytes = gzip(&tar_of(&[("a.txt", b"hello"), ("b/", b""), ("b/c.bin", &[0; 4000])]));

    let listing = list(&bytes, ArchiveFormat::TarGz);

    assert_eq!(listing.format, ArchiveFormat::TarGz);
    assert!(!listing.truncated);
    assert_eq!(
        listing.entries,
        [
            entry("a.txt", 5, false, Some(MOMENT)),
            entry("b/", 0, true, Some(MOMENT)),
            entry("b/c.bin", 4000, false, Some(MOMENT)),
        ]
    );
}

#[test]
fn a_tar_with_more_entries_than_the_cap_is_cut() {
    let names = many_names(MAX_ARCHIVE_ENTRIES + 3);
    let bytes = tar_of(&empty_files(&names));

    for (bytes, format) in [
        (bytes.clone(), ArchiveFormat::Tar),
        (gzip(&bytes), ArchiveFormat::TarGz),
    ] {
        let listing = list(&bytes, format);

        assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES, "{format:?}");
        assert!(listing.truncated, "{format:?}");
    }
}

#[test]
fn a_long_entry_name_is_cut_at_4096_bytes_on_a_character() {
    let name = "é".repeat(3000);
    let bytes = tar_of(&[(name.as_str(), b"x")]);

    let listing = list(&bytes, ArchiveFormat::Tar);

    assert_eq!(listing.entries[0].name.len(), 4096);
    assert!(listing.entries[0].name.chars().all(|c| c == 'é'));
}

#[test]
fn a_broken_tar_is_an_error() {
    let unreadable = (ErrorKind::InvalidData, "the archive cannot be read".to_owned());

    for (what, bytes) in [
        ("letters", vec![b'A'; 2000]),
        ("ascii", b"hello, this is not a tar".repeat(40)),
    ] {
        let (_folder, file) = file_of(&bytes);

        assert_eq!(failure(&file, ArchiveFormat::Tar), unreadable, "{what}");
    }
}

#[test]
fn a_broken_gzip_is_an_error() {
    let unreadable = (ErrorKind::InvalidData, "the archive cannot be read".to_owned());

    for (what, bytes) in [
        ("not gzip", b"plain text, no magic".repeat(30)),
        ("only the magic", vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3]),
    ] {
        let (_folder, file) = file_of(&bytes);

        assert_eq!(failure(&file, ArchiveFormat::TarGz), unreadable, "{what}");
    }

    // A tar that breaks after its first entry lists what it has.
    let mut cut = tar_of(&[("a.txt", b"hello"), ("b.txt", b"world")]);
    cut.truncate(512 * 2 + 100);
    let listing = list(&cut, ArchiveFormat::Tar);
    assert_eq!(
        listing.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a.txt"]
    );
    assert!(listing.truncated);
}

#[test]
fn the_kind_found_on_opening_matches_what_is_listed() {
    let (_folder, opened) = opened("pack.tar.gz", &gzip(&tar_of(&[("a.txt", b"hello")])));

    let listing = archive(&opened.file, ArchiveFormat::TarGz, &idle()).expect("a listing");

    assert_eq!(
        opened.kind,
        crate::model::ViewKind::Archive {
            format: ArchiveFormat::TarGz
        }
    );
    assert_eq!(listing.entries.len(), 1);
}

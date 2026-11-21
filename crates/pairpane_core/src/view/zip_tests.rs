//! Listing a zip with the reader of our own: names, sizes and times from the extra data, data before the zip, the
//! limits against a crafted end record (nothing big is read or allocated), and a few thousand damaged zips.

use std::fs::{self, File};
use std::io::{ErrorKind, Seek, SeekFrom, Write};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::model::ArchiveFormat;

use super::archive_tests::{MOMENT, entry, failure, list, zip_of, zip64_then};
use super::testing::{idle, put, temp};
use super::{ViewFile, archive, open};

const GIB: u64 = 1024 * 1024 * 1024;
const DOS_DATE: u16 = 21092;
const DOS_TIME: u16 = 10436;

/// A record of a central directory with a name, general purpose `flags`, a 32-bit `size`, `extra` data and the DOS
/// `date`.
fn record(name: &[u8], flags: u16, size: u32, extra: &[u8], date: u16) -> Vec<u8> {
    let mut bytes = b"PK\x01\x02".to_vec();

    for field in [20u16, 20, flags, 0, DOS_TIME, date] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&[0; 14]);
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(extra);

    bytes
}

/// A zip that is only a central directory of `records` and its end record, which declares `entries` entries.
fn directory_of(records: &[Vec<u8>], entries: u16) -> Vec<u8> {
    let directory = records.concat();
    let mut bytes = directory.clone();

    bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    bytes.extend_from_slice(&entries.to_le_bytes());
    bytes.extend_from_slice(&entries.to_le_bytes());
    bytes.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    bytes
}

/// An extra data field `id` holding `data`.
fn field(id: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = id.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
    bytes.extend_from_slice(data);

    bytes
}

fn one(record: Vec<u8>) -> crate::model::ArchiveEntry {
    let mut listing = list(&directory_of(&[record], 1), ArchiveFormat::Zip);

    assert_eq!(listing.entries.len(), 1);

    listing.entries.remove(0)
}

fn refused(bytes: &[u8]) -> (ErrorKind, String) {
    let folder = temp();
    put(folder.path(), "z.zip", bytes);
    let file = open(folder.path(), "z.zip").expect("open").file;

    failure(&file, ArchiveFormat::Zip)
}

fn unreadable() -> (ErrorKind, String) {
    (ErrorKind::InvalidData, "the archive cannot be read".to_owned())
}

#[test]
fn a_name_is_utf8_with_the_flag_and_cp437_without_it_unless_it_is_valid_utf8() {
    let cases: [(&[u8], u16, &str); 5] = [
        (b"\x82\x80.txt", 0, "éÇ.txt"),
        (b"\xB3\xC4", 0, "│─"),
        ("zażółć".as_bytes(), 0, "zażółć"),
        ("zażółć".as_bytes(), 1 << 11, "zażółć"),
        (b"a\xFFb", 1 << 11, "a\u{FFFD}b"),
    ];

    for (name, flags, text) in cases {
        assert_eq!(one(record(name, flags, 1, &[], DOS_DATE)).name, text, "{name:?}");
    }
}

#[test]
fn a_name_never_shows_a_control_or_bidirectional_character() {
    let name = "a\nb\u{202E}c\u{2067}d\x1b\u{85}e".as_bytes();

    let shown = one(record(name, 1 << 11, 1, &[], DOS_DATE));

    assert_eq!(shown.name, "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}\u{FFFD}e");
}

#[test]
fn a_long_folder_name_is_cut_but_still_a_folder() {
    let name = format!("{}/", "d".repeat(5000));

    let shown = one(record(name.as_bytes(), 1 << 11, 0, &[], DOS_DATE));

    assert_eq!(shown.name, "d".repeat(4096));
    assert!(shown.dir);
}

#[test]
fn the_size_and_time_come_from_the_extra_data_when_it_has_them() {
    let mut extra = field(0x9999, &[1, 2, 3]);
    extra.extend(field(0x0001, &5_000_000_000u64.to_le_bytes()));
    let mut stamp = vec![1];
    stamp.extend_from_slice(&1_700_000_000i32.to_le_bytes());
    extra.extend(field(0x5455, &stamp));

    let shown = one(record(b"big.bin", 0, u32::MAX, &extra, DOS_DATE));

    assert_eq!(shown, entry("big.bin", 5_000_000_000, false, Some(1_700_000_000_000)));
}

#[test]
fn a_damaged_extra_field_is_ignored() {
    // The field says 200 bytes and there are 2: the 32-bit size and the DOS time stay.
    let shown = one(record(b"a", 0, u32::MAX, &[1, 0, 200, 0, 7, 7], DOS_DATE));

    assert_eq!(shown, entry("a", u64::from(u32::MAX), false, Some(MOMENT)));
}

#[test]
fn a_time_that_is_no_moment_is_left_out() {
    // Date 0 (month 0), month 13 and the 30th of February.
    for date in [0, (13 << 5) | 1, (41 << 9) | (2 << 5) | 30] {
        assert_eq!(one(record(b"a", 0, 1, &[], date)).modified, None, "{date}");
    }
}

#[test]
fn a_zip_with_a_comment_is_listed() {
    let mut bytes = zip_of(&[("a.txt", b"x")]);
    let end = bytes.len() - 2;
    bytes[end..].copy_from_slice(&1000u16.to_le_bytes());
    bytes.extend_from_slice(&[b'c'; 1000]);

    assert_eq!(list(&bytes, ArchiveFormat::Zip).entries.len(), 1);
}

#[test]
fn data_before_a_zip_or_a_zip64_zip_does_not_matter() {
    let plain = zip_of(&[("a.txt", b"x"), ("b/", b"")]);
    let wide = zip64_then(&plain, 2, b"");

    for (what, zip) in [("zip", plain), ("zip64", wide)] {
        let mut bytes = b"MZ self-extracting program, 33 b".to_vec();
        bytes.extend_from_slice(&zip);

        let names: Vec<_> = list(&bytes, ArchiveFormat::Zip)
            .entries
            .into_iter()
            .map(|entry| (entry.name, entry.dir))
            .collect();

        assert_eq!(names, [("a.txt".to_owned(), false), ("b/".to_owned(), true)], "{what}");
    }
}

#[test]
fn an_entry_count_that_the_directory_does_not_hold_is_an_error_not_a_guess() {
    // Three declared, one there: the rest of the 138 bytes is zeros.
    let mut bytes = record(b"a", 0, 1, &[], DOS_DATE);
    bytes.resize(46 * 3, 0);
    let end = directory_of(&[], 3);
    bytes.extend_from_slice(&end);
    let at = bytes.len() - 22;
    bytes[at + 12..at + 16].copy_from_slice(&(46u32 * 3).to_le_bytes());

    assert_eq!(refused(&bytes), unreadable());

    // Declared 1, two there: one is listed.
    let two = directory_of(
        &[record(b"a", 0, 1, &[], DOS_DATE), record(b"b", 0, 1, &[], DOS_DATE)],
        2,
    );
    let mut bytes = two.clone();
    let at = bytes.len() - 22;
    bytes[at + 10..at + 12].copy_from_slice(&1u16.to_le_bytes());

    assert_eq!(list(&bytes, ArchiveFormat::Zip).entries.len(), 1);
}

/// A file of 1 GiB that is a hole but for its last 98 bytes: a zip64 end record that declares `entries`, `size` and
/// `offset`, its locator and an end record that sends the reader to them (all three fields full).
fn sparse_zip64(entries: u64, size: u64, offset: u64) -> (TempDir, ViewFile) {
    let folder = temp();
    let path = folder.path().join("sparse.zip");
    let mut zip = File::create(&path).expect("create");
    zip.seek(SeekFrom::Start(GIB)).expect("seek");

    let mut tail = b"PK\x06\x06".to_vec();
    tail.extend_from_slice(&44u64.to_le_bytes());
    tail.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for value in [entries, entries, size, offset] {
        tail.extend_from_slice(&value.to_le_bytes());
    }
    tail.extend_from_slice(b"PK\x06\x07\0\0\0\0");
    tail.extend_from_slice(&GIB.to_le_bytes());
    tail.extend_from_slice(&1u32.to_le_bytes());
    tail.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    tail.extend_from_slice(&[0xFF; 12]);
    tail.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    tail.extend_from_slice(&[0, 0]);
    zip.write_all(&tail).expect("tail");
    drop(zip);

    let file = open(folder.path(), "sparse.zip").expect("open").file;

    (folder, file)
}

#[test]
fn a_sparse_gigabyte_zip_with_a_crafted_zip64_record_is_refused_unread() {
    let refusal = |text: &str| (ErrorKind::InvalidData, text.to_owned());
    let too_many = refusal("the archive declares too many entries to list");

    let cases = [
        (
            "a trillion entries",
            sparse_zip64(1 << 40, 1 << 20, 0),
            too_many.clone(),
        ),
        ("more entries than bytes", sparse_zip64(100_000, 46, 0), too_many),
        (
            "a directory of gigabytes",
            sparse_zip64(5, 3 * GIB, 0),
            refusal("the archive's directory is too large to list"),
        ),
        ("an offset past the end", sparse_zip64(10, 460, GIB * 4), unreadable()),
        ("a hole instead of records", sparse_zip64(10, 460, 0), unreadable()),
    ];

    for (what, (_folder, file), expected) in cases {
        let size = file.size().expect("size");
        assert!(size > GIB, "{what}");

        assert_eq!(failure(&file, ArchiveFormat::Zip), expected, "{what}");
    }

    // The same file with sizes that are true is an empty directory.
    let (_folder, file) = sparse_zip64(0, 0, 0);
    assert_eq!(archive(&file, ArchiveFormat::Zip, &idle()).expect("empty").entries, []);
}

#[test]
fn a_sparse_zip_with_a_crafted_end_record_is_refused_too() {
    // A plain end record at the end of a hole: the directory it declares would be 4 GiB.
    let folder = temp();
    let mut zip = File::create(folder.path().join("sparse.zip")).expect("create");
    zip.seek(SeekFrom::Start(GIB)).expect("seek");
    let mut end = b"PK\x05\x06\0\0\0\0".to_vec();
    end.extend_from_slice(&[0xFF; 12]);
    end.extend_from_slice(&[0, 0]);
    zip.write_all(&end).expect("end");
    drop(zip);
    let file = open(folder.path(), "sparse.zip").expect("open").file;

    assert_eq!(
        failure(&file, ArchiveFormat::Zip),
        (
            ErrorKind::InvalidData,
            "the archive's directory is too large to list".to_owned()
        )
    );
}

/// A small, deterministic stream of numbers.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;

        self.0
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
}

#[test]
fn thousands_of_damaged_zips_are_listed_or_refused_never_a_panic() {
    let plain = zip_of(&[("a.txt", b"hello"), ("dir/", b""), ("zażółć.bin", &[7; 50])]);
    let mut stamp = vec![1];
    stamp.extend_from_slice(&1_700_000_000i32.to_le_bytes());
    let mut extra = field(0x0001, &5u64.to_le_bytes());
    extra.extend(field(0x5455, &stamp));
    let seeds = [
        plain.clone(),
        zip64_then(&plain, 3, b""),
        [b"leading bytes".as_slice(), &plain].concat(),
        directory_of(&[record(b"\x82x", 0, u32::MAX, &extra, DOS_DATE)], 1),
    ];

    let folder = temp();
    let mut dice = Dice(0x9E37_79B9_7F4A_7C15);

    for round in 0..4000 {
        let mut bytes = seeds[round % seeds.len()].clone();

        for _ in 0..=dice.below(3) {
            // Half of the damage is to the end records, where the numbers that matter are.
            let from = if dice.below(2) == 0 {
                bytes.len().saturating_sub(100)
            } else {
                0
            };
            let at = from + dice.below(bytes.len() - from);
            bytes[at] = dice.next() as u8;
        }
        if dice.below(8) == 0 {
            bytes.truncate(dice.below(bytes.len()));
        }

        fs::write(folder.path().join("z.zip"), &bytes).expect("write");
        let file = open(folder.path(), "z.zip").expect("open").file;

        let _ = archive(&file, ArchiveFormat::Zip, &idle());
    }
}

//! Reading the data of zip entries through [`open_source`]: every method, the checks on what an entry holds, the places an
//! entry may be, and what happens to one entry that cannot be read.

use std::io;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use crate::archive::testing::{Member, zip_members, zip_of};
use crate::archive::{Flow, Input, open_source};

use super::testing::{BZIP2, DEFLATE, LZMA, STORED, ZSTD, failure, index_of, member, read_entries, read_only};

/// Content that compresses, and is not all the same.
fn content() -> Vec<u8> {
    (0..50_000_u32)
        .map(|number| (number % 251) as u8 ^ (number / 1000) as u8)
        .collect()
}

#[test]
fn every_method_gives_back_the_content_and_a_folder_gives_nothing() {
    let data = content();

    for method in [STORED, DEFLATE, BZIP2, ZSTD, LZMA] {
        let bytes = zip_members(&[
            member("a.bin", &data, method),
            Member::stored("dir/", b""),
            member("dir/b.txt", b"short", method),
            member("empty", b"", method),
        ]);
        let got = read_entries(&bytes, None, &[]).expect("a source");

        assert_eq!(got.len(), 4, "method {method}");
        assert_eq!(got[0].1.as_ref().expect("a.bin"), &data, "method {method}");
        assert_eq!(got[1].1.as_ref().expect("dir"), b"", "method {method}");
        assert_eq!(got[2].1.as_ref().expect("b.txt"), b"short", "method {method}");
        assert_eq!(got[3].1.as_ref().expect("empty"), b"", "method {method}");
    }
}

#[test]
fn only_the_wanted_entries_are_read_in_the_order_asked_and_a_stop_ends_it() {
    let bytes = zip_of(&[("a", b"1"), ("b", b"22"), ("c", b"333")]);

    let got = read_entries(&bytes, None, &[2, 0]).expect("a source");
    let names: Vec<_> = got.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["c", "a"]);

    let index = index_of(&bytes).expect("an index");
    let mut seen = Vec::new();
    open_source(
        &index,
        &Input {
            file: &bytes,
            path: None,
        },
        &AtomicBool::new(false),
        None,
    )
    .expect("a source")
    .extract(&[0, 1, 2], &mut |entry, _| {
        seen.push(entry.path.clone());

        Ok(Flow::Stop)
    })
    .expect("extracted");
    assert_eq!(seen, ["a"]);
}

#[test]
fn a_raised_cancel_flag_stops_the_source() {
    let bytes = zip_of(&[("a", b"1")]);
    let index = index_of(&bytes).expect("an index");
    let cancel = AtomicBool::new(true);

    let error = open_source(
        &index,
        &Input {
            file: &bytes,
            path: None,
        },
        &cancel,
        None,
    )
    .expect("a source")
    .extract(&[0], &mut |_, _| Ok(Flow::Continue))
    .expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn a_checksum_that_does_not_match_fails_the_entry_at_its_end() {
    for method in [STORED, DEFLATE] {
        let mut bad = member("a", b"some content", method);
        bad.crc ^= 1;
        let error = failure(&zip_members(&[bad]), None);

        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "method {method}");
        assert_eq!(error.to_string(), "the checksum of an entry does not match its data");
    }
}

#[test]
fn data_beyond_the_declared_size_fails_the_entry_and_less_data_does_too() {
    for method in [STORED, DEFLATE] {
        let mut more = member("a", b"twelve bytes", method);
        more.size = 5;
        let error = failure(&zip_members(&[more]), None);
        assert_eq!(
            error.to_string(),
            "an entry holds more data than it declares",
            "method {method}"
        );

        let mut less = member("a", b"twelve bytes", method);
        less.size = 40;
        let error = failure(&zip_members(&[less]), None);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "method {method}");
    }
}

#[test]
fn a_method_that_is_not_handled_fails_that_entry_only() {
    let mut deflate64 = Member::stored("old", b"data");
    deflate64.method = 9;
    let bytes = zip_members(&[Member::stored("before", b"1"), deflate64, Member::stored("after", b"2")]);

    let got = read_entries(&bytes, None, &[]).expect("a source");

    assert_eq!(got[0].1.as_ref().expect("before"), b"1");
    assert_eq!(
        got[1].1.as_ref().expect_err("deflate64").kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(got[2].1.as_ref().expect("after"), b"2");
}

#[test]
fn entries_that_overlap_make_the_source_refuse_the_archive() {
    let first = member("a", &content(), DEFLATE);
    let inside = first.data.len() as u32 / 2;

    // The second entry starts where the first does, and inside the first one's data.
    for start in [0, 40, 30 + 1 + inside] {
        let mut second = Member::stored("b", b"second");
        second.offset = Some(start);
        let bytes = zip_members(&[member("a", &content(), DEFLATE), second]);
        let error = read_entries(&bytes, None, &[]).expect_err("overlap");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "start {start}");
        assert_eq!(error.to_string(), "the entries of the archive overlap");
    }

    // Entries that follow each other do not overlap.
    let bytes = zip_members(&[first, Member::stored("b", b"second")]);
    assert_eq!(read_entries(&bytes, None, &[]).expect("fine").len(), 2);
}

#[test]
fn a_local_header_that_pushes_the_data_into_the_next_entry_fails_that_entry() {
    let mut bytes = zip_of(&[("a", b"first one"), ("b", b"second")]);
    // The local header of "a" claims an extra field of 2,000 bytes.
    bytes[28..30].copy_from_slice(&2000_u16.to_le_bytes());

    let got = read_entries(&bytes, None, &[]).expect("a source");

    assert_eq!(
        got[0].1.as_ref().expect_err("overlaps").kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(got[1].1.as_ref().expect("b"), b"second");
}

#[test]
fn data_that_runs_past_the_file_or_a_missing_local_header_fails_the_entry() {
    let mut bytes = zip_of(&[("a", b"first one")]);
    // The central directory says the data is 100,000 bytes.
    let record = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x01\x02")
        .expect("a record");
    bytes[record + 20..record + 24].copy_from_slice(&100_000_u32.to_le_bytes());
    assert_eq!(failure(&bytes, None).kind(), io::ErrorKind::InvalidData);

    let mut moved = Member::stored("a", b"first one");
    moved.offset = Some(10_000);
    assert_eq!(failure(&zip_members(&[moved]), None).kind(), io::ErrorKind::InvalidData);

    // Not a local header where the directory says it is.
    let mut broken = zip_of(&[("a", b"first one")]);
    broken[0] = b'X';
    assert_eq!(
        read_only(&broken, None).expect_err("no header").to_string(),
        "the archive cannot be read"
    );
}

#[test]
fn the_local_header_gives_the_name_and_extra_lengths_not_the_directory() {
    // The local header of "a" carries an extra field that the directory's record lacks.
    let mut with_extra = Member::stored("a", b"content");
    with_extra.extra = vec![0xFE, 0xCA, 4, 0, 1, 2, 3, 4];
    let mut bytes = zip_members(&[with_extra]);
    let record = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x01\x02")
        .expect("a record");
    // Take the extra field out of the directory's record: its length 0, the name moves up.
    bytes[record + 30..record + 32].copy_from_slice(&0_u16.to_le_bytes());
    let name = record + 46;
    bytes.drain(name + 1..name + 9);
    // ... and the end record's size of the directory shrinks with it.
    let end = bytes.len() - 22;
    let size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().expect("four bytes")) - 8;
    bytes[end + 12..end + 16].copy_from_slice(&size.to_le_bytes());

    assert_eq!(read_only(&bytes, None).expect("read"), b"content");
}

//! The walkers of the zip directory and of tar headers: the full record each entry gives, zip64 fields in their order,
//! the kinds, modes and links, and a visitor that reads, skips, stops or fails.

use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use super::entries::{EntryType, Flow, Place, Record};
use super::tar::walk;
use super::testing::{Central, MOMENT, directory_of, field, tar_of, zip_of};
use super::zip::{AesExtra, ZipPlace, directory};

fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

fn zip_records(bytes: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    let bytes = bytes.to_vec();

    directory::walk(&bytes, &idle(), |record| {
        records.push(record);

        Flow::Continue
    })
    .expect("a directory");

    records
}

fn place(record: &Record) -> &ZipPlace {
    let Place::Zip(place) = &record.place else {
        panic!("a zip place");
    };

    place
}

fn one_record(central: &Central<'_>) -> Record {
    zip_records(&directory_of(&[central.bytes()], 1)).remove(0)
}

#[test]
fn a_zip_record_has_everything_the_data_reader_needs() {
    let zip = zip_of(&[("a.txt", b"hello"), ("dir/", b""), ("b.bin", &[7; 300])]);

    let records = zip_records(&zip);

    assert_eq!(
        records.iter().map(|record| record.name.as_str()).collect::<Vec<_>>(),
        ["a.txt", "dir/", "b.bin"]
    );
    assert_eq!(
        records.iter().map(|record| record.size).collect::<Vec<_>>(),
        [5, 0, 300]
    );
    assert_eq!(records[0].modified, Some(MOMENT));

    let first = place(&records[0]);
    assert_eq!(
        (first.method, first.flags, first.compressed, first.offset),
        (0, 0, 5, 0)
    );
    assert_eq!(first.crc, crc32fast::hash(b"hello"));
    assert_eq!(first.made_by, 20);
    assert_eq!(first.dos_time, 10436);

    // Each local header follows the data of the one before: 30 bytes, the name, the data.
    assert_eq!(place(&records[1]).offset, 30 + 5 + 5);
    assert_eq!(place(&records[2]).offset, 30 + 5 + 5 + 30 + 4);
    assert_eq!(place(&records[2]).compressed, 300);
}

#[test]
fn data_before_the_zip_moves_the_offsets_of_the_local_headers() {
    let zip = zip_of(&[("a.txt", b"hello"), ("b.txt", b"world")]);
    let lead = b"MZ self-extracting program, 33 b".to_vec();
    let plain = zip_records(&zip);

    let moved = zip_records(&[lead.clone(), zip].concat());

    for (before, after) in plain.iter().zip(&moved) {
        assert_eq!(place(after).offset, place(before).offset + lead.len() as u64);
    }
}

#[test]
fn a_zip_record_says_what_the_entry_is_and_its_permissions() {
    let unix = |external: u32| Central {
        made_by: 3 << 8 | 20,
        external: external << 16,
        ..Central::new(b"entry")
    };

    let cases = [
        (unix(0o100_755), EntryType::File, Some(0o755)),
        (unix(0o040_755), EntryType::Dir, Some(0o755)),
        (unix(0o120_777), EntryType::Symlink, Some(0o777)),
        (unix(0o010_644), EntryType::Special, Some(0o644)),
        (unix(0o020_644), EntryType::Special, Some(0o644)),
        (unix(0o140_644), EntryType::Special, Some(0o644)),
        // The setuid bit is stored as it is; stripping it is the extractor's job.
        (unix(0o104_755), EntryType::File, Some(0o4755)),
        (unix(0), EntryType::File, None),
        // A name that ends in a slash is a folder whatever the attributes say.
        (
            Central {
                made_by: 3 << 8,
                external: 0o100_644 << 16,
                ..Central::new(b"d/")
            },
            EntryType::Dir,
            Some(0o644),
        ),
        // A DOS host has the folder bit in the low byte and no permissions.
        (
            Central {
                external: 0x10,
                ..Central::new(b"dosdir")
            },
            EntryType::Dir,
            None,
        ),
        (
            Central {
                external: 0x20,
                ..Central::new(b"dosfile")
            },
            EntryType::File,
            None,
        ),
    ];

    for (central, kind, mode) in cases {
        let record = one_record(&central);

        assert_eq!((record.kind, record.mode), (kind, mode), "{:?}", record.name);
    }
}

#[test]
fn encryption_and_the_aes_field_are_read_from_the_directory() {
    let plain = one_record(&Central::new(b"a"));
    assert!(!plain.encrypted && place(&plain).aes.is_none());

    let crypto = one_record(&Central {
        flags: 1,
        ..Central::new(b"a")
    });
    assert!(crypto.encrypted && place(&crypto).aes.is_none());

    // Version 2 (AE-2), the vendor `AE`, AES-256, deflate underneath.
    let aes_data = [2, 0, b'A', b'E', 3, 8, 0];
    let aes = one_record(&Central {
        method: 99,
        flags: 1,
        extra: &field(0x9901, &aes_data),
        ..Central::new(b"a")
    });

    assert!(aes.encrypted);
    assert_eq!(place(&aes).method, 99);
    assert_eq!(
        place(&aes).aes,
        Some(AesExtra {
            version: 2,
            strength: 3,
            method: 8
        })
    );
}

#[test]
fn zip64_values_are_taken_in_order_and_only_for_the_fields_that_are_full() {
    const FULL: u32 = u32::MAX;
    let wide = |values: &[u64]| {
        field(
            0x0001,
            &values.iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<u8>>(),
        )
    };

    // Only the offset is full: the field holds just the offset, and the sizes stay as they are.
    let extra = wide(&[7_000_000_000]);
    let only_offset = one_record(&Central {
        size: 100,
        compressed: 60,
        offset: FULL,
        extra: &extra,
        ..Central::new(b"a")
    });
    assert_eq!(only_offset.size, 100);
    assert_eq!(
        (place(&only_offset).compressed, place(&only_offset).offset),
        (60, 7_000_000_000)
    );

    // Only the compressed size is full.
    let extra = wide(&[6_000_000_000]);
    let only_compressed = one_record(&Central {
        size: 100,
        compressed: FULL,
        offset: 12,
        extra: &extra,
        ..Central::new(b"a")
    });
    assert_eq!(only_compressed.size, 100);
    assert_eq!(
        (place(&only_compressed).compressed, place(&only_compressed).offset),
        (6_000_000_000, 12)
    );

    // The size and the offset are full, the compressed size is not.
    let extra = wide(&[5_000_000_000, 7_000_000_000]);
    let split = one_record(&Central {
        size: FULL,
        compressed: 60,
        offset: FULL,
        extra: &extra,
        ..Central::new(b"a")
    });
    assert_eq!(split.size, 5_000_000_000);
    assert_eq!((place(&split).compressed, place(&split).offset), (60, 7_000_000_000));

    // All three, in the order the format says.
    let extra = wide(&[5_000_000_000, 6_000_000_000, 7_000_000_000]);
    let all = one_record(&Central {
        size: FULL,
        compressed: FULL,
        offset: FULL,
        extra: &extra,
        ..Central::new(b"a")
    });
    assert_eq!(all.size, 5_000_000_000);
    assert_eq!(
        (place(&all).compressed, place(&all).offset),
        (6_000_000_000, 7_000_000_000)
    );
}

#[test]
fn a_zip_walk_stops_when_the_visitor_says_so() {
    let zip = zip_of(&[("a", b"1"), ("b", b"2"), ("c", b"3")]);
    let mut seen = 0;

    let walked = directory::walk(&zip, &idle(), |_| {
        seen += 1;

        if seen == 2 { Flow::Stop } else { Flow::Continue }
    })
    .expect("a walk");

    assert_eq!(seen, 2);
    assert!(walked.stopped && !walked.cut);
}

fn tar_with(add: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    add(&mut builder);

    builder.into_inner().expect("a tar")
}

fn header(path: &str, kind: tar::EntryType, size: u64, mode: u32) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_path(path).expect("a path");
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(mode);
    header.set_mtime((MOMENT / 1000) as u64);

    header
}

fn tar_records(bytes: &[u8]) -> Vec<Record> {
    let mut stream = io::BufReader::new(io::Cursor::new(bytes.to_vec()));
    let mut records = Vec::new();

    walk(&mut Plain(&mut stream), None, &idle(), |record, _| {
        records.push(record);

        Ok(Flow::Continue)
    })
    .expect("a walk");

    records
}

/// A seekable stream as a tar stream.
struct Plain<'a, R>(&'a mut io::BufReader<R>);

impl<R: Read> Read for Plain<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl<R: Read> super::tar::Stream for Plain<'_, R> {
    fn skip(&mut self, count: u64) -> io::Result<()> {
        io::copy(&mut self.0.by_ref().take(count), &mut io::sink()).and_then(|copied| {
            if copied == count {
                Ok(())
            } else {
                Err(super::error::unreadable())
            }
        })
    }
}

#[test]
fn a_tar_record_has_the_kind_the_mode_and_the_link() {
    use tar::EntryType as T;

    let bytes = tar_with(|builder| {
        builder
            .append_data(&mut header("file", T::Regular, 3, 0o4755), "file", &b"abc"[..])
            .expect("file");
        builder
            .append_data(&mut header("dir/", T::Directory, 0, 0o755), "dir/", io::empty())
            .expect("dir");

        builder
            .append_link(&mut header("sym", T::Symlink, 0, 0o777), "sym", "target/file")
            .expect("symlink");
        builder
            .append_link(&mut header("hard", T::Link, 0, 0o644), "hard", "file")
            .expect("hardlink");

        for (name, kind) in [("fifo", T::Fifo), ("char", T::Char), ("block", T::Block)] {
            builder
                .append_data(&mut header(name, kind, 0, 0o600), name, io::empty())
                .expect(name);
        }
    });

    let records = tar_records(&bytes);

    let summary: Vec<_> = records
        .iter()
        .map(|record| (record.name.as_str(), record.kind, record.mode))
        .collect();
    assert_eq!(
        summary,
        [
            ("file", EntryType::File, Some(0o4755)),
            ("dir/", EntryType::Dir, Some(0o755)),
            ("sym", EntryType::Symlink, Some(0o777)),
            ("hard", EntryType::Hardlink, Some(0o644)),
            ("fifo", EntryType::Special, Some(0o600)),
            ("char", EntryType::Special, Some(0o600)),
            ("block", EntryType::Special, Some(0o600)),
        ]
    );
    assert_eq!(records[2].link.as_deref(), Some("target/file"));
    assert_eq!(records[3].link.as_deref(), Some("file"));
    assert_eq!(records[0].link, None);
    assert!(
        records
            .iter()
            .all(|record| record.place == Place::Walk && !record.encrypted)
    );
}

#[test]
fn a_sparse_entry_is_special_and_a_pax_linkpath_wins_over_the_header() {
    use tar::EntryType as T;

    let bytes = tar_with(|builder| {
        let mut sparse = header("gnu-sparse", T::GNUSparse, 0, 0o644);
        sparse.as_gnu_mut().expect("a gnu header").set_real_size(1_000_000);
        builder
            .append_data(&mut sparse, "gnu-sparse", io::empty())
            .expect("sparse");

        builder
            .append_pax_extensions([("GNU.sparse.major", &b"1"[..]), ("GNU.sparse.name", &b"x"[..])])
            .expect("pax");
        builder
            .append_data(
                &mut header("pax-sparse", T::Regular, 4, 0o644),
                "pax-sparse",
                &b"abcd"[..],
            )
            .expect("pax sparse");

        builder
            .append_pax_extensions([("linkpath", &b"from/pax"[..])])
            .expect("pax");
        builder
            .append_link(&mut header("sym", T::Symlink, 0, 0o777), "sym", "from/header")
            .expect("symlink");
    });

    let records = tar_records(&bytes);

    assert_eq!(
        records.iter().map(|record| record.kind).collect::<Vec<_>>(),
        [EntryType::Special, EntryType::Special, EntryType::Symlink]
    );
    assert_eq!(records[2].link.as_deref(), Some("from/pax"));
}

#[test]
fn the_visitor_reads_the_data_of_an_entry_or_leaves_it_to_be_stepped_over() {
    let bytes = tar_of(&[("a.txt", b"hello"), ("big.bin", &[9; 3000]), ("c.txt", b"world")]);
    let mut stream = io::BufReader::new(io::Cursor::new(bytes));
    let mut read = Vec::new();

    let walked = walk(&mut Plain(&mut stream), None, &idle(), |record, data| {
        let mut content = Vec::new();

        // The big one is not read, and not read to its end for the first, to show that both are stepped over.
        if record.name == "a.txt" {
            data.take(2).read_to_end(&mut content)?;
        } else if record.name != "big.bin" {
            data.read_to_end(&mut content)?;
        }
        read.push((record.name, content));

        Ok(Flow::Continue)
    })
    .expect("a walk");

    assert!(!walked.stopped && !walked.cut);
    assert_eq!(
        read,
        [
            ("a.txt".to_owned(), b"he".to_vec()),
            ("big.bin".to_owned(), Vec::new()),
            ("c.txt".to_owned(), b"world".to_vec()),
        ]
    );
}

#[test]
fn a_tar_walk_stops_on_request_and_ends_with_the_error_of_the_visitor() {
    let bytes = tar_of(&[("a", b"1"), ("b", b"2"), ("c", b"3")]);

    let mut stream = io::BufReader::new(io::Cursor::new(bytes.clone()));
    let mut names = Vec::new();
    let walked = walk(&mut Plain(&mut stream), None, &idle(), |record, _| {
        names.push(record.name);

        Ok(if names.len() == 2 { Flow::Stop } else { Flow::Continue })
    })
    .expect("a walk");
    assert_eq!(names, ["a", "b"]);
    assert!(walked.stopped);

    let mut stream = io::BufReader::new(io::Cursor::new(bytes));
    let error = walk(&mut Plain(&mut stream), None, &idle(), |_, _| {
        Err(io::Error::other("disk full"))
    })
    .expect_err("failed");
    assert_eq!(error.to_string(), "disk full");
}

#[test]
fn data_the_archive_does_not_hold_is_an_error_for_the_reader() {
    let mut bytes = tar_of(&[("a", &[1; 2000])]);
    bytes.truncate(512 + 700);
    let mut stream = io::BufReader::new(io::Cursor::new(bytes));
    let mut failed = None;

    let walked = walk(&mut Plain(&mut stream), None, &idle(), |_, data| {
        failed = data.read_to_end(&mut Vec::new()).err().map(|error| error.kind());

        Ok(Flow::Continue)
    });

    assert_eq!(failed, Some(io::ErrorKind::InvalidData));
    assert!(walked.is_ok());
}

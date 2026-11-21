//! The limits of listing an archive: a tar header that declares gigabytes of name, a zip with a decoy end record, a
//! zip of exactly 65,535 entries, and cancelling a long listing.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::Compression;
use flate2::write::GzEncoder;
use pretty_assertions::assert_eq;
use tar::EntryType;

use crate::model::ArchiveFormat;

use super::archive::Capped;
use super::archive_tests::{MOMENT, empty_files, entry, failure, gzip, list, many_names, tar_of, zip_of, zip64_then};
use super::tar_list::{self, Stream};
use super::testing::{file_of, idle, temp};
use super::{MAX_ARCHIVE_ENTRIES, archive, open};

const MIB: u64 = 1024 * 1024;

/// A tar header block of `kind` for an entry of `size` bytes.
fn header_of(kind: EntryType, size: u64) -> Vec<u8> {
    let mut header = tar::Header::new_gnu();
    header.set_path("entry").expect("a path");
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mtime((MOMENT / 1000) as u64);
    header.set_cksum();

    header.as_bytes().to_vec()
}

/// `bytes` padded with zeros to whole 512-byte blocks.
fn padded(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.resize(bytes.len().next_multiple_of(512), 0);

    bytes
}

/// One PAX record, `key=value`, with its length in front.
fn record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    while (length.to_string().len() + body.len()) != length {
        length += 1;
    }

    format!("{length}{body}").into_bytes()
}

/// A tar with an entry (kind `kind`, declaring `size` bytes of data) in front of a file `short` of 2 bytes. Only
/// `head` of the declared data is written; `gap` stands for the rest of it, when `Some`.
fn describing(kind: EntryType, size: u64, head: &[u8], gap: Option<u64>) -> Vec<Part> {
    let mut parts = vec![Part::Bytes(header_of(kind, size)), Part::Bytes(head.to_vec())];
    parts.extend(gap.map(Part::Gap));
    parts.push(Part::Bytes(tar_of(&[("short", b"hi")])));

    parts
}

enum Part {
    Bytes(Vec<u8>),
    /// Bytes that exist only in name: reading one is a failure, stepping over them is not.
    Gap(u64),
}

/// A tar stream made of bytes and gaps.
struct Scripted {
    parts: VecDeque<Part>,
}

impl Read for Scripted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(front) = self.parts.front_mut() else {
            return Ok(0);
        };
        let Part::Bytes(bytes) = front else {
            return Err(io::Error::other("a gap was read"));
        };

        let count = bytes.len().min(buffer.len());
        buffer[..count].copy_from_slice(&bytes[..count]);
        bytes.drain(..count);
        if bytes.is_empty() {
            self.parts.pop_front();
        }

        Ok(count)
    }
}

impl Stream for Scripted {
    fn skip(&mut self, mut count: u64) -> io::Result<()> {
        while count > 0 {
            match self.parts.front_mut() {
                None => return Err(io::Error::other("skipped past the end")),
                Some(Part::Bytes(bytes)) => {
                    let step = bytes.len().min(usize::try_from(count).unwrap_or(usize::MAX));
                    bytes.drain(..step);
                    count -= step as u64;
                    if bytes.is_empty() {
                        self.parts.pop_front();
                    }
                }
                Some(Part::Gap(left)) => {
                    let step = (*left).min(count);
                    *left -= step;
                    count -= step;
                    if *left == 0 {
                        self.parts.pop_front();
                    }
                }
            }
        }

        Ok(())
    }
}

fn walk(parts: Vec<Part>) -> Vec<(String, u64)> {
    let mut stream = Scripted { parts: parts.into() };
    let (entries, truncated) = tar_list::list(&mut stream, None, &idle()).expect("a listing");

    assert!(!truncated);

    entries.into_iter().map(|entry| (entry.name, entry.size)).collect()
}

#[test]
fn a_huge_long_name_is_cut_and_stepped_over_never_read() {
    let size = 256 * MIB;
    // 8000 bytes of name, then the zeros of the rest: they are a gap that fails when read.
    let parts = describing(EntryType::GNULongName, size, &[b'n'; 8000], Some(size - 8000));

    assert_eq!(walk(parts), [("n".repeat(4096), 2)]);
}

#[test]
fn a_huge_pax_entry_and_long_link_are_not_read_whole_either() {
    let size = 1024 * MIB;
    let mut records = record("path", &"p".repeat(100));
    records.resize(64 * 1024, 0);
    let parts = describing(EntryType::XHeader, size, &records, Some(size - records.len() as u64));
    assert_eq!(walk(parts), [("p".repeat(100), 2)]);

    let parts = describing(EntryType::GNULongLink, size, &[], Some(size));
    assert_eq!(walk(parts), [("short".to_owned(), 2)]);

    let parts = describing(EntryType::XGlobalHeader, size, &[], Some(size));
    assert_eq!(walk(parts), [("short".to_owned(), 2)]);
}

#[test]
fn a_pax_path_and_size_describe_the_next_entry() {
    let path = format!("{}file", "deep/".repeat(60));
    let mut records = record("path", &path);
    records.extend(record("size", "5"));

    let mut tar = header_of(EntryType::XHeader, records.len() as u64);
    tar.extend(padded(records));
    // The header says 0 bytes, the PAX record says 5: they are in the next block.
    tar.extend(header_of(EntryType::Regular, 0));
    tar.extend(padded(b"12345".to_vec()));
    tar.extend(tar_of(&[("after", b"")]));

    let listing = list(&tar, ArchiveFormat::Tar);

    assert_eq!(listing.entries[0], entry(&path, 5, false, Some(MOMENT)));
    assert_eq!(listing.entries[1], entry("after", 0, false, Some(MOMENT)));
    assert_eq!(listing.entries.len(), 2);
}

#[test]
fn a_sparse_entry_with_extension_blocks_is_followed() {
    let mut header = tar::Header::new_gnu();
    header.set_path("holes.bin").expect("a path");
    header.set_entry_type(EntryType::GNUSparse);
    header.set_size(0);
    header.set_mtime((MOMENT / 1000) as u64);
    let gnu = header.as_gnu_mut().expect("a gnu header");
    gnu.set_real_size(1_000_000);
    gnu.set_is_extended(true);
    header.set_cksum();

    let mut tar = header.as_bytes().to_vec();
    tar.extend([0; 512]);
    tar.extend(tar_of(&[("after", b"")]));

    let listing = list(&tar, ArchiveFormat::Tar);

    assert_eq!(listing.entries[0], entry("holes.bin", 1_000_000, false, Some(MOMENT)));
    assert_eq!(listing.entries[1].name, "after");
    assert_eq!(listing.entries.len(), 2);
}

#[test]
fn exactly_the_cap_of_entries_is_not_a_cut() {
    let names = many_names(MAX_ARCHIVE_ENTRIES);

    let listing = list(&tar_of(&empty_files(&names)), ArchiveFormat::Tar);

    assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES);
    assert!(!listing.truncated);
}

/// A gzip of a tar whose first entry is a GNU long name that declares `size` bytes (all but the name zeros, written
/// in chunks), in front of a file of 2 bytes.
fn gz_with_long_name(size: u64) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&header_of(EntryType::GNULongName, size)).expect("header");
    gz.write_all(&[b'n'; 8000]).expect("name");

    let zeros = vec![0; MIB as usize];
    let mut left = size - 8000;
    while left > 0 {
        let step = left.min(MIB) as usize;
        gz.write_all(&zeros[..step]).expect("zeros");
        left -= step as u64;
    }
    gz.write_all(&tar_of(&[("short", b"hi")])).expect("entry");

    gz.finish().expect("finish")
}

#[test]
fn a_gz_tar_with_a_huge_long_name_lists_with_the_name_cut() {
    let (_folder, file) = file_of(&gz_with_long_name(16 * MIB));
    let listing = archive(&file, ArchiveFormat::TarGz, &idle()).expect("a listing");

    assert_eq!(listing.entries, [entry(&"n".repeat(4096), 2, false, Some(MOMENT))]);
    assert!(!listing.truncated);
}

#[test]
fn a_plain_tar_with_a_huge_long_name_is_stepped_over_with_a_seek() {
    // 1 GiB of declared name, of which the file holds 8000 bytes; the entry after it is where the data would end.
    let size = 1024 * MIB;
    let folder = temp();
    let mut tar = fs::File::create(folder.path().join("big.tar")).expect("create");
    tar.write_all(&header_of(EntryType::GNULongName, size)).expect("header");
    tar.write_all(&[b'n'; 8000]).expect("name");
    tar.seek(SeekFrom::Start(512 + size)).expect("seek");
    tar.write_all(&tar_of(&[("short", b"hi")])).expect("entry");
    drop(tar);
    let file = open(folder.path(), "big.tar").expect("open").file;

    let listing = archive(&file, ArchiveFormat::Tar, &idle()).expect("a listing");

    assert_eq!(listing.entries, [entry(&"n".repeat(4096), 2, false, Some(MOMENT))]);
}

#[test]
fn a_raised_flag_stops_every_listing() {
    let cancel = AtomicBool::new(true);
    let zip = zip_of(&[("a", b"x")]);
    let tar = tar_of(&[("a", b"x")]);

    for (bytes, format) in [
        (zip, ArchiveFormat::Zip),
        (tar.clone(), ArchiveFormat::Tar),
        (gzip(&tar), ArchiveFormat::TarGz),
    ] {
        let (_folder, file) = file_of(&bytes);

        let error = archive(&file, format, &cancel).expect_err("cancelled");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{format:?}");
        assert_eq!(error.to_string(), "the read was cancelled");
    }
}

/// Raises `cancel` at its `after`-th read.
struct Raiser<'a, R> {
    inner: R,
    reads: u32,
    after: u32,
    cancel: &'a AtomicBool,
}

impl<R: Read> Read for Raiser<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads == self.after {
            self.cancel.store(true, Ordering::Relaxed);
        }

        self.inner.read(buffer)
    }
}

#[test]
fn a_flag_raised_while_stepping_over_a_huge_entry_stops_the_listing() {
    // A long name that declares 1 GiB, followed by endless zeros: only the flag ends the walk.
    let cancel = AtomicBool::new(false);
    let reached = Cell::new(false);
    let endless = io::Cursor::new(header_of(EntryType::GNULongName, 1024 * MIB)).chain(io::repeat(0));
    let mut stream = Capped {
        inner: Raiser {
            inner: endless,
            reads: 0,
            after: 50,
            cancel: &cancel,
        },
        left: u64::MAX,
        reached: &reached,
        cancel: &cancel,
    };

    let error = tar_list::list(&mut stream, Some(&reached), &cancel).expect_err("cancelled");

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(stream.inner.reads < 100);
}

#[test]
fn an_earlier_end_record_behind_a_decoy_is_not_followed_past_the_checked_directory() {
    // The last end record says 1 entry in 46 bytes (which passes the size checks) at an offset past the file: it is the
    // one that counts, and the zip64 record before it, which says 150,000 entries, is never looked at.
    let names = many_names(150_000);
    let mut decoy = b"PK\x05\x06\0\0\0\0".to_vec();
    decoy.extend_from_slice(&[1, 0, 1, 0]);
    decoy.extend_from_slice(&46u32.to_le_bytes());
    decoy.extend_from_slice(&0xFFFF_FF00u32.to_le_bytes());
    decoy.extend_from_slice(&[0, 0]);
    let bytes = zip64_then(&zip_of(&empty_files(&names)), 150_000, &decoy);
    let (_folder, file) = file_of(&bytes);

    assert_eq!(
        failure(&file, ArchiveFormat::Zip),
        (io::ErrorKind::InvalidData, "the archive cannot be read".to_owned())
    );
}

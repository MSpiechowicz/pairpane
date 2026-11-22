//! Listing a tar: what a cut archive lists, which name wins, the checksum variants, folders without a folder type, and
//! a read that a signal interrupted.

use std::cell::Cell;
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;
use tar::EntryType;

use crate::model::ArchiveFormat;

use crate::archive::tar::{Capped, Stream};

use super::archive_tests::{MOMENT, entry, failure, gzip, list, tar_of};
use super::testing::file_of;

/// A header block for `path`, of `kind` and `size`, with its checksum; `edit` may change the header before that.
fn block(path: &str, kind: EntryType, size: u64, edit: impl FnOnce(&mut tar::Header)) -> Vec<u8> {
    let mut header = tar::Header::new_gnu();
    header.set_path(path).expect("a path");
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mtime((MOMENT / 1000) as u64);
    edit(&mut header);
    header.set_cksum();

    header.as_bytes().to_vec()
}

/// `data` padded with zeros to whole blocks.
fn padded(data: &[u8]) -> Vec<u8> {
    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);

    bytes
}

/// A PAX record `key=value` with its length in front.
fn pax(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    while length.to_string().len() + body.len() != length {
        length += 1;
    }

    format!("{length}{body}").into_bytes()
}

/// A tar of a descriptive entry (`kind`, holding `data`) followed by the file `real`.
fn described(kind: EntryType, data: &[u8]) -> Vec<u8> {
    let mut tar = block("description", kind, data.len() as u64, |_| {});
    tar.extend(padded(data));
    tar.extend(block("real", EntryType::Regular, 3, |_| {}));
    tar.extend(padded(b"abc"));
    tar.extend([0; 1024]);

    tar
}

#[test]
fn an_entry_whose_data_runs_past_the_end_of_the_file_is_listed_and_the_list_is_cut() {
    let whole = tar_of(&[("a.txt", &[1; 1000]), ("b.txt", &[2; 1000])]);

    // Cut in the data of the last entry, in a plain tar and in a gzip of one.
    let cut = &whole[..512 + 1024 + 512 + 100];
    for (bytes, format) in [(cut.to_vec(), ArchiveFormat::Tar), (gzip(cut), ArchiveFormat::TarGz)] {
        let listing = list(&bytes, format);

        assert_eq!(
            listing.entries,
            [
                entry("a.txt", 1000, false, Some(MOMENT)),
                entry("b.txt", 1000, false, Some(MOMENT)),
            ],
            "{format:?}"
        );
        assert!(listing.truncated, "{format:?}");
    }

    // A tar that ends right after the data of its last entry (no end blocks) is whole.
    let listing = list(&whole[..512 + 1024 + 512 + 1024], ArchiveFormat::Tar);
    assert_eq!(listing.entries.len(), 2);
    assert!(!listing.truncated);
}

#[test]
fn a_tar_of_one_cut_entry_is_listed_not_refused() {
    let whole = tar_of(&[("only.bin", &[1; 5000])]);

    let listing = list(&whole[..512 + 700], ArchiveFormat::Tar);

    assert_eq!(listing.entries, [entry("only.bin", 5000, false, Some(MOMENT))]);
    assert!(listing.truncated);
}

#[test]
fn an_empty_or_nul_only_long_name_leaves_the_name_of_the_header() {
    for data in [&b""[..], &[0; 5]] {
        let listing = list(&described(EntryType::GNULongName, data), ArchiveFormat::Tar);

        assert_eq!(listing.entries, [entry("real", 3, false, Some(MOMENT))]);
    }
}

#[test]
fn a_pax_path_wins_over_a_gnu_long_name() {
    let mut tar = block("description", EntryType::GNULongName, 9, |_| {});
    tar.extend(padded(b"gnu-name\0"));
    let records = pax("path", "pax-name");
    tar.extend(block("pax", EntryType::XHeader, records.len() as u64, |_| {}));
    tar.extend(padded(&records));
    tar.extend(block("header-name", EntryType::Regular, 0, |_| {}));
    tar.extend([0; 1024]);

    let listing = list(&tar, ArchiveFormat::Tar);

    assert_eq!(listing.entries, [entry("pax-name", 0, false, Some(MOMENT))]);
}

#[test]
fn a_checksum_summed_as_signed_bytes_is_accepted_and_a_wrong_one_is_not() {
    // The bytes of "é" are over 127, so the two sums differ.
    let mut header = block("é.txt", EntryType::Regular, 0, |_| {});
    let mut spaced = header.clone();
    spaced[148..156].fill(b' ');
    let unsigned: i64 = spaced.iter().map(|byte| i64::from(*byte)).sum();
    let signed: i64 = spaced.iter().map(|byte| i64::from(*byte as i8)).sum();
    assert_ne!(unsigned, signed);

    let mut tar = |sum: i64| {
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut bytes = header.clone();
        bytes.extend([0; 1024]);
        bytes
    };

    assert_eq!(list(&tar(signed), ArchiveFormat::Tar).entries[0].name, "é.txt");
    assert_eq!(list(&tar(unsigned), ArchiveFormat::Tar).entries[0].name, "é.txt");

    let wrong = tar(unsigned + 1);
    let (_folder, file) = file_of(&wrong);
    assert_eq!(
        failure(&file, ArchiveFormat::Tar),
        (io::ErrorKind::InvalidData, "the archive cannot be read".to_owned())
    );
}

#[test]
fn a_regular_entry_named_like_a_folder_is_a_folder() {
    // Type '0' and the old type NUL are both a regular file for the reader.
    for kind_byte in [b'0', 0] {
        let mut tar = block("folder/", EntryType::Regular, 0, |header| {
            header.as_mut_bytes()[156] = kind_byte
        });
        tar.extend(block("file", EntryType::Regular, 0, |_| {}));
        tar.extend([0; 1024]);

        let listing = list(&tar, ArchiveFormat::Tar);

        assert_eq!(
            listing.entries,
            [
                entry("folder/", 0, true, Some(MOMENT)),
                entry("file", 0, false, Some(MOMENT)),
            ],
            "{kind_byte}"
        );
    }
}

/// Fails its first read with `Interrupted`, as a signal does, then gives `left` zeros.
struct Signalled {
    interrupted: bool,
    left: usize,
}

impl Read for Signalled {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }

        let count = buffer.len().min(self.left);
        buffer[..count].fill(0);
        self.left -= count;

        Ok(count)
    }
}

#[test]
fn skipping_inflated_data_goes_on_after_a_signal_but_not_after_a_cancel() {
    let reached = Cell::new(false);
    let down = AtomicBool::new(false);
    let mut capped = Capped {
        inner: Signalled {
            interrupted: false,
            left: 100_000,
        },
        left: u64::MAX,
        reached: &reached,
        cancel: &down,
    };

    capped.skip(70_000).expect("skipped");

    let raised = AtomicBool::new(true);
    let mut capped = Capped {
        inner: Signalled {
            interrupted: false,
            left: 100_000,
        },
        left: u64::MAX,
        reached: &reached,
        cancel: &raised,
    };

    assert_eq!(
        capped.skip(70_000).expect_err("cancelled").kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn a_tar_name_never_shows_a_control_or_bidirectional_character() {
    let mut tar = block("a\nb\u{202E}c\u{200F}d\u{61C}e\u{9F}", EntryType::Regular, 0, |_| {});
    tar.extend([0; 1024]);

    let listing = list(&tar, ArchiveFormat::Tar);

    assert_eq!(listing.entries[0].name, "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}e\u{FFFD}");
}

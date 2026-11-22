//! The reading of `unrar vt`: the output captured from `unrar 7.30` for a hand-made archive (see `testing.rs`), cut where
//! the blocks repeat, and the cases that the capture does not hold.

use pretty_assertions::assert_eq;

use super::{Listing, parse, parse_within};
use crate::archive::entries::{EntryType, Place, Record};
use crate::archive::error::password_fault;
use crate::archive::names::Unsafe;
use crate::archive::{Builder, Flow};
use crate::model::ArchiveFormat;

/// 2023-11-14 22:13:20 UTC in milliseconds.
const MOMENT_MS: i64 = 1_700_000_000_000;

/// `unrar vt -p- <archive>` of an archive with a folder, a file with the setuid bit, a symbolic link, a hard link and a file
/// whose name holds a line break.
const CAPTURED: &str = "\nUNRAR 7.30 beta 2 freeware      Copyright (c) 1993-2026 Alexander Roshal\n\nArchive: t.rar\nDetails: RAR 5\n\n        Name: d\n        Type: Directory\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: drwxr-xr-x\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0\n\n        Name: d/a.txt\n        Type: File\n        Size: 8\n Packed size: 8\n       Ratio: 100%\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: -rw-r--r--\n       CRC32: 5B6D4005\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0 -md=128k\n\n        Name: b.txt\n        Type: File\n        Size: 4\n Packed size: 4\n       Ratio: 100%\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: -rwsr-xr-x\n       CRC32: 06289FA3\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0 -md=128k\n\n        Name: ln\n        Type: Unix symbolic link\n      Target: b.txt\n        Size: 0\n Packed size: 0\n       Ratio: 0%\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: lrwxrwxrwx\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0 -md=128k\n\n        Name: hard\n        Type: Hard link\n      Target: b.txt\n        Size: 0\n Packed size: 0\n       Ratio: 0%\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: lrwxrwxrwx\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0 -md=128k\n\n        Name: new\nline.txt\n        Type: File\n        Size: 3\n Packed size: 3\n       Ratio: 100%\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: -rw-r--r--\n       CRC32: 9495F28D\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0 -md=128k\n\n        Name: ../evil.txt\n        Type: File\n        Size: 5\n Packed size: 5\n       Ratio: 100%\n       mtime: 2023-11-14 22:13:20,000000000\n  Attributes: -rw-r--r--\n       CRC32: B73FCD7A\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m0 -md=128k\n\n";

fn listing(text: &str) -> Listing {
    parse(text.as_bytes()).expect("a listing")
}

/// `(name, kind, size)` of each record.
fn summary(listing: &Listing) -> Vec<(&str, EntryType, u64)> {
    listing
        .records
        .iter()
        .map(|record| (record.name.as_str(), record.kind, record.size))
        .collect()
}

#[test]
fn the_entries_of_a_captured_listing_are_read_with_their_fields() {
    let listing = listing(CAPTURED);

    assert_eq!(
        summary(&listing),
        [
            ("d", EntryType::Dir, 0),
            ("d/a.txt", EntryType::File, 8),
            ("b.txt", EntryType::File, 4),
            ("ln", EntryType::Symlink, 0),
            ("hard", EntryType::Hardlink, 0),
            ("new\nline.txt", EntryType::File, 3),
            ("../evil.txt", EntryType::File, 5),
        ]
    );
    assert!(!listing.cut && !listing.volume);

    let file = &listing.records[1];
    assert_eq!(file.modified, Some(MOMENT_MS));
    assert_eq!(file.mode, Some(0o644));
    assert_eq!(file.place, Place::Walk);
    assert!(!file.encrypted);

    assert_eq!(
        listing.records[2].mode,
        Some(0o4755),
        "the setuid bit is kept for the caller to strip"
    );
    assert_eq!(listing.records[3].link.as_deref(), Some("b.txt"));
    assert_eq!(listing.records[4].link.as_deref(), Some("b.txt"));
}

#[test]
fn a_name_with_a_line_break_or_a_climb_is_refused_by_the_index_and_the_rest_is_listed() {
    let mut builder = Builder::new(ArchiveFormat::Rar);

    for record in listing(CAPTURED).records {
        assert_eq!(builder.push(record), Flow::Continue);
    }
    let index = builder.finish(false);

    let refused: Vec<_> = index
        .entries()
        .iter()
        .filter_map(|entry| entry.refused.map(|why| (entry.path.as_str(), why)))
        .collect();
    assert_eq!(
        refused,
        [("new\nline.txt", Unsafe::Control), ("../evil.txt", Unsafe::Parent)]
    );
    assert!(index.find("d", "a.txt").is_some());
    assert!(index.find("", "evil.txt").is_none());
}

#[test]
fn a_name_that_runs_over_lines_is_one_name_whatever_is_in_its_lines() {
    let text =
        "Archive: t.rar\nDetails: RAR 5\n\n        Name: a\n        Size: 9\nb\n        Type: File\n        Size: 3\n";

    let listing = listing(text);

    assert_eq!(summary(&listing), [("a\n        Size: 9\nb", EntryType::File, 3)]);
}

#[test]
fn encrypted_content_and_other_systems_attributes_are_read() {
    let text = "Archive: t.rar\nDetails: RAR 4, solid\n\n        Name: secret.txt\n        Type: File\n        Size: 5\n       mtime: 2024-06-05 20:38:10,000000000\n  Attributes: ..A....\n     Host OS: Windows\n Compression: RAR 3.0(v29) -m3 -md=4096k\n       Flags: encrypted, solid\n\n        Name: odd\n        Type: File\n  Attributes: lrwxrwxrwx\n\n        Name: junction\n        Type: NTFS junction point\n\n        Name: plain\n        Type: File\n        Flags: solid\n";

    let listing = listing(text);
    let records = &listing.records;

    assert!(records[0].encrypted);
    assert_eq!(
        records[0].mode, None,
        "attributes of another system hold no permission bits"
    );
    assert_eq!(records[0].modified, Some(1_717_619_890_000));
    assert_eq!(
        records[1].kind,
        EntryType::Symlink,
        "RAR 3 stores a link as a file with link attributes"
    );
    assert_eq!(records[2].kind, EntryType::Special);
    assert!(!records[3].encrypted);
}

#[test]
fn a_volume_of_a_set_is_marked() {
    assert!(listing("Archive: t.rar\nDetails: RAR 5, volume 1\n\n").volume);
}

#[test]
fn what_is_not_a_listing_of_an_archive_is_refused() {
    for text in [
        "",
        "\nUNRAR 7.30\n\ng.rar is not RAR archive\n",
        "Archive: x\n\n        Name: a\n        Type: File\n",
    ] {
        let error = parse(text.as_bytes()).expect_err(text);

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{text}");
        assert_eq!(password_fault(&error), None);
    }
}

#[test]
fn an_output_over_a_cap_is_cut_and_never_half_an_entry() {
    let entries = parse_within(CAPTURED.as_bytes(), 1 << 20, 3).expect("a listing");
    assert!(entries.cut);
    assert_eq!(entries.records.len(), 3);

    // Cut in the middle of the fourth entry: what is there of it is dropped.
    let at = CAPTURED.find("Name: ln").expect("fourth entry") + 30;
    let bytes = parse_within(CAPTURED.as_bytes(), at as u64, 100).expect("a listing");
    assert!(bytes.cut);
    assert_eq!(bytes.records.len(), 3);
    assert!(bytes.records.iter().all(|record: &Record| record.name != "ln"));
}

#[test]
fn a_line_longer_than_a_name_can_be_is_cut_and_the_next_lines_are_read() {
    let long = "x".repeat(100_000);
    let text = format!(
        "Archive: t.rar\nDetails: RAR 5\n\n        Name: {long}\n        Type: File\n        Size: 3\n\n        Name: ok\n        Type: File\n"
    );

    let listing = listing(&text);

    assert_eq!(listing.records.len(), 2);
    assert!(listing.records[0].name.len() <= crate::archive::MAX_NAME_BYTES + 64);
    assert_eq!(listing.records[1].name, "ok");

    let mut builder = Builder::new(ArchiveFormat::Rar);
    for record in listing.records {
        builder.push(record);
    }
    assert_eq!(builder.finish(false).entries()[0].refused, Some(Unsafe::TooLong));
}

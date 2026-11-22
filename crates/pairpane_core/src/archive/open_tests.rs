//! Indexing and extracting through the two doors of the module: end to end for a plain tar, a tar.gz, xz, zstd and bzip2, and a stored zip,
//! what a listing reads, what a source yields, and the password marker.

use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use crate::model::ArchiveFormat;

use super::entries::{Declared, Flow};
use super::error::{PasswordFault, password_error, password_fault};
use super::file::ReadAt;
use super::testing::{
    Counting, assert_stop_and_failure, bzip2, extract, gzip, index_of, project, root, tar_of, xz, zip_of, zstd,
};
use super::{Input, Password, build_index, open_source};

#[test]
fn a_plain_tar_a_compressed_tar_and_a_stored_zip_are_indexed_end_to_end() {
    let tar = tar_of(&project());

    for (bytes, format) in [
        (tar.clone(), ArchiveFormat::Tar),
        (gzip(&tar), ArchiveFormat::TarGz),
        (xz(&tar), ArchiveFormat::TarXz),
        (zstd(&tar), ArchiveFormat::TarZst),
        (bzip2(&tar), ArchiveFormat::TarBz2),
        (zip_of(&project()), ArchiveFormat::Zip),
    ] {
        let index = index_of(&bytes, format).expect("an index");

        assert_eq!(index.format(), format);
        assert!(!index.truncated(), "{format:?}");
        assert_eq!(
            root(&index),
            [("src".to_owned(), true), ("README".to_owned(), false)],
            "{format:?}"
        );
        let src: Vec<_> = index
            .list("src")
            .expect("src")
            .iter()
            .map(|child| child.name.to_owned())
            .collect();
        assert_eq!(src, ["main.rs", "lib"], "{format:?}");
        assert_eq!(
            index
                .find("src/lib", "util.rs")
                .and_then(|child| child.entry)
                .map(|entry| entry.size),
            Some(17)
        );
    }
}

#[test]
fn a_zip_listing_reads_the_end_records_and_the_directory_and_nothing_else() {
    let big = vec![0xAB; 1024 * 1024];
    let zip = Counting::new(zip_of(&[
        ("one.bin", &big),
        ("two.bin", &big),
        ("three.bin", &big),
        ("dir/", b""),
    ]));
    let size = zip.size().expect("size");
    assert!(size > 3 * 1024 * 1024);

    let index = index_of(&zip, ArchiveFormat::Zip).expect("an index");

    assert_eq!(index.entries().len(), 4);
    // The tail that holds the end record (64 KiB at most) and the directory: a few hundred bytes more.
    assert!(zip.read() < 70_000, "read {} of {size}", zip.read());
}

#[test]
fn moving_between_the_folders_of_an_index_reads_nothing() {
    let file = Counting::new(tar_of(&project()));
    let index = index_of(&file, ArchiveFormat::Tar).expect("an index");
    let after_listing = file.read();

    for dir in ["", "src", "src/lib", "src", ""] {
        assert!(index.list(dir).is_some());
    }

    assert_eq!(file.read(), after_listing);
}

#[test]
fn a_tar_extraction_yields_the_wanted_entries_in_order_and_stops_after_the_last() {
    let big = vec![5; 2 * 1024 * 1024];
    let tar = Counting::new(tar_of(&[
        ("a.txt", b"first"),
        ("big.bin", &big),
        ("c.txt", b"third"),
        ("d.txt", &big),
    ]));
    let index = index_of(&tar, ArchiveFormat::Tar).expect("an index");
    let before = tar.read();

    let got = extract(&index, &tar, &[0, 2]).expect("extracted");

    assert_eq!(
        got,
        [
            ("a.txt".to_owned(), b"first".to_vec()),
            ("c.txt".to_owned(), b"third".to_vec())
        ]
    );
    // The two big entries are stepped over with seeks, never read.
    assert!(tar.read() - before < 100_000, "read {}", tar.read() - before);
}

#[test]
fn a_compressed_tar_extraction_decodes_through_to_the_wanted_entries() {
    let tar = tar_of(&project());

    for (bytes, format) in [
        (gzip(&tar), ArchiveFormat::TarGz),
        (xz(&tar), ArchiveFormat::TarXz),
        (zstd(&tar), ArchiveFormat::TarZst),
        (bzip2(&tar), ArchiveFormat::TarBz2),
    ] {
        let index = index_of(&bytes, format).expect("an index");
        let selection = index.select("src", &["lib".to_owned()]).expect("selected");

        let got = extract(&index, &bytes, &selection.ordinals()).expect("extracted");

        assert_eq!(
            got,
            [("src/lib/util.rs".to_owned(), b"pub fn util() {}\n".to_vec())],
            "{format:?}"
        );
    }
}

#[test]
fn a_visitor_that_stops_ends_the_extraction_and_one_that_fails_ends_it_with_its_error() {
    let tar = tar_of(&[("a", b"1"), ("b", b"2"), ("c", b"3")]);

    assert_stop_and_failure(&index_of(&tar, ArchiveFormat::Tar).expect("an index"), &tar);
}

#[test]
fn an_archive_that_changed_since_it_was_indexed_is_an_error_not_a_guess() {
    let index = index_of(&tar_of(&[("a", b"1"), ("b", b"2"), ("c", b"3")]), ArchiveFormat::Tar).expect("an index");
    let shorter = tar_of(&[("a", b"1")]);

    let error = extract(&index, &shorter, &[0, 2]).expect_err("changed");

    assert_eq!(error.to_string(), "the archive cannot be read");
}

#[test]
fn a_raised_cancel_flag_stops_indexing_and_extracting() {
    let tar = tar_of(&project());
    let raised = AtomicBool::new(true);

    for (bytes, format) in [
        (tar.clone(), ArchiveFormat::Tar),
        (gzip(&tar), ArchiveFormat::TarGz),
        (zip_of(&project()), ArchiveFormat::Zip),
    ] {
        let error = build_index(
            format,
            &Input {
                file: &bytes,
                path: None,
            },
            &raised,
            None,
        )
        .expect_err("cancelled");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{format:?}");
    }

    let index = index_of(&tar, ArchiveFormat::Tar).expect("an index");
    let error = open_source(&index, &Input { file: &tar, path: None }, &raised, None)
        .expect("a source")
        .extract(&[0], &mut |_, _| Ok(Flow::Continue))
        .expect_err("cancelled");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn a_cut_tar_gz_gives_an_index_marked_cut() {
    let whole = tar_of(&[("a.txt", &[1; 1000]), ("b.txt", &[2; 1000])]);
    let cut = &whole[..512 + 1024 + 512 + 100];

    let index = index_of(&gzip(cut), ArchiveFormat::TarGz).expect("an index");

    assert_eq!(index.entries().len(), 2);
    assert!(index.truncated());
}

#[test]
fn an_entry_reader_keeps_to_the_size_the_entry_declares() {
    let read = |content: &[u8], declared: u64| {
        let mut out = Vec::new();

        Declared::new(content, declared).read_to_end(&mut out).map(|_| out)
    };

    assert_eq!(read(b"hello", 5).expect("exact"), b"hello");
    assert_eq!(read(b"", 0).expect("empty"), b"");
    assert_eq!(
        read(b"hello", 4).expect_err("one byte too many").kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        read(b"hello", 9).expect_err("too short").kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        read(b"hello", 9).expect_err("too short").to_string(),
        "the archive cannot be read"
    );

    let mut at_most = Vec::new();
    Declared::at_most(&b"hello"[..], 9)
        .read_to_end(&mut at_most)
        .expect("fewer is fine");
    assert_eq!(at_most, b"hello");
}

#[test]
fn a_password_never_shows_and_its_marker_is_told_from_other_errors() {
    let password = Password::new("hunter2".to_owned());

    assert_eq!(format!("{password:?}"), "Password(..)");
    assert_eq!(password.as_bytes(), b"hunter2");

    let needed = password_error(PasswordFault::Needed);
    let wrong = password_error(PasswordFault::Wrong);
    assert_eq!(password_fault(&needed), Some(PasswordFault::Needed));
    assert_eq!(password_fault(&wrong), Some(PasswordFault::Wrong));
    assert_eq!(needed.to_string(), "needs a password");
    assert_eq!(wrong.to_string(), "the password is wrong");
    assert_eq!(password_fault(&io::Error::other("something else")), None);
    assert_eq!(password_fault(&io::Error::from(io::ErrorKind::PermissionDenied)), None);
}

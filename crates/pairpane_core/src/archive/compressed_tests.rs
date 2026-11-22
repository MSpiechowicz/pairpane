//! The tars inside XZ, Zstandard and bzip2 (indexed and extracted like the others in `open_tests`): read through
//! concatenated streams, limited by the scan cap, and refused when damaged or cancelled.

use std::io;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use crate::model::ArchiveFormat;

use super::entries::Flow;
use super::testing::{Counting, bzip2, extract, idle, index_of, project, tar_of, xz, zstd};
use super::{Input, build_index, records};

type Encode = fn(&[u8]) -> Vec<u8>;

const COMPRESSED: [(ArchiveFormat, Encode); 3] = [
    (ArchiveFormat::TarXz, xz),
    (ArchiveFormat::TarZst, zstd),
    (ArchiveFormat::TarBz2, bzip2),
];

#[test]
fn a_stream_in_several_pieces_is_read_as_one() {
    let tar = tar_of(&[("a.txt", &[1; 1000]), ("b.txt", &[2; 1000])]);
    // Cut on a block boundary, inside the first entry's data, so the tar is split between the two streams.
    let (head, tail) = tar.split_at(1024);

    for (format, encode) in COMPRESSED {
        let bytes = [encode(head), encode(tail)].concat();
        let index = index_of(&bytes, format).expect("an index");

        assert_eq!(index.entries().len(), 2, "{format:?}");
        assert!(!index.truncated(), "{format:?}");
        assert_eq!(
            extract(&index, &bytes, &[0, 1]).expect("extracted"),
            [("a.txt".to_owned(), vec![1; 1000]), ("b.txt".to_owned(), vec![2; 1000])],
            "{format:?}"
        );
    }
}

#[test]
fn a_tar_longer_than_the_scan_cap_is_cut_and_listing_writes_nothing() {
    let tar = tar_of(&[("a.txt", &[1; 1000]), ("b.txt", &[2; 1000]), ("c.txt", &[3; 1000])]);

    for (format, encode) in COMPRESSED {
        let file = Counting::new(encode(&tar));
        let mut names = Vec::new();

        let cut = records(
            format,
            &Input {
                file: &file,
                path: None,
            },
            &idle(),
            2100,
            None,
            &mut |record| {
                names.push(record.name);

                Flow::Continue
            },
        )
        .expect("listed");

        assert!(cut, "{format:?}");
        assert_eq!(names, ["a.txt", "b.txt"], "{format:?}");
    }
}

#[test]
fn a_damaged_compressed_tar_is_an_error_or_a_cut_listing_never_a_panic() {
    let tar = tar_of(&[("a.txt", &[1; 5000]), ("b.txt", &[2; 5000])]);

    for (format, encode) in COMPRESSED {
        let whole = encode(&tar);
        let cut = whole[..whole.len() * 3 / 4].to_vec();
        let garbage = vec![0xA7; 600];

        match index_of(&cut, format) {
            Ok(index) => assert!(index.truncated(), "{format:?}"),
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{format:?}"),
        }

        let error = index_of(&garbage, format).expect_err("garbage");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{format:?}");
        assert_eq!(error.to_string(), "the archive cannot be read", "{format:?}");
    }
}

#[test]
fn a_raised_cancel_flag_stops_a_compressed_tar() {
    let raised = AtomicBool::new(true);

    for (format, encode) in COMPRESSED {
        let bytes = encode(&tar_of(&project()));
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
}

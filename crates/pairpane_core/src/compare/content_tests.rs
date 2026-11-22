//! Comparing by content: what the bytes decide, what is skipped, and the stop between chunks.

use std::sync::atomic::Ordering;
use std::time::Duration;

use pretty_assertions::assert_eq;

use crate::model::{CompareEvent, CompareOptions, CompareStatus};

use super::Limits;
use super::testing::{compare, compare_with, folders, normal, plain, put, temp};

use CompareStatus::{Differs, Equal, LeftNewer};

fn by_content() -> CompareOptions {
    CompareOptions {
        content: true,
        ..plain()
    }
}

/// The status of `x.txt` as the two sides hold it (bytes and time of each), judged by metadata and by content.
fn both_ways(left: (&[u8], u64), right: (&[u8], u64)) -> [(String, CompareStatus); 2] {
    let (one, other) = (temp(), temp());
    put(one.path(), "x.txt", left.0, left.1);
    put(other.path(), "x.txt", right.0, right.1);

    [plain(), by_content()].map(|options| compare(one.path(), other.path(), options).statuses().remove(0))
}

fn status(status: CompareStatus) -> (String, CompareStatus) {
    ("x.txt".to_owned(), status)
}

#[test]
fn files_of_one_size_and_time_that_hold_other_bytes_differ_only_when_content_is_compared() {
    let [by_metadata, by_bytes] = both_ways((b"abcdef", 10_000), (b"abcdeX", 10_000));

    assert_eq!(by_metadata, status(Equal));
    assert_eq!(by_bytes, status(Differs));
}

#[test]
fn equal_bytes_are_the_same_file_whatever_the_times_say() {
    let [by_metadata, by_bytes] = both_ways((b"abcdef", 90_000), (b"abcdef", 10_000));

    assert_eq!(by_metadata, status(LeftNewer));
    assert_eq!(by_bytes, status(Equal));
}

#[test]
fn different_bytes_leave_the_later_file_newer() {
    let [_, by_bytes] = both_ways((b"abcdef", 90_000), (b"abcdeX", 10_000));

    assert_eq!(by_bytes, status(LeftNewer));
}

#[test]
fn files_longer_than_a_chunk_are_compared_to_their_last_byte() {
    let (left, right) = (temp(), temp());
    let mut one = vec![7_u8; 300 * 1024];
    let two = one.clone();
    put(left.path(), "big.bin", &one, 10_000);
    put(right.path(), "big.bin", &two, 10_000);
    *one.last_mut().expect("a byte") = 8;
    put(left.path(), "tail.bin", &one, 10_000);
    put(right.path(), "tail.bin", &two, 10_000);

    let compared = compare(left.path(), right.path(), by_content());

    assert_eq!(
        compared.statuses(),
        [("big.bin".to_owned(), Equal), ("tail.bin".to_owned(), Differs)]
    );
    // Every byte of both pairs was read.
    assert_eq!(compared.bytes(), Some(4 * 300 * 1024));
}

#[test]
fn a_stop_between_chunks_leaves_the_file_without_a_row() {
    let (left, right) = (temp(), temp());
    let bytes = vec![1_u8; 2 * 1024 * 1024];
    put(left.path(), "big.bin", &bytes, 10_000);
    put(right.path(), "big.bin", &bytes, 10_000);

    // Progress goes out after each chunk, and the first one that holds bytes raises the flag.
    let compared = compare_with(
        folders(left.path(), right.path()),
        by_content(),
        Limits {
            tick: Duration::ZERO,
            ..normal()
        },
        |event, cancel| {
            if matches!(event, CompareEvent::Progress { bytes, .. } if *bytes > 0) {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    )
    .expect("a comparison");

    assert_eq!(compared.done(), (0, 0, false, true));
    let read = compared.bytes();
    assert!(read.is_some_and(|bytes| bytes < 4 * 1024 * 1024), "{read:?}");
}

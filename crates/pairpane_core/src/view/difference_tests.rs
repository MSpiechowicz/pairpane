//! Finding where two files differ: both directions, chunk borders, different sizes, the budget and cancelling.

use std::cell::Cell;
use std::io::ErrorKind;

use pretty_assertions::assert_eq;

use crate::model::ByteDifference::{self, Found, Searching};

use super::testing::file_of;
use super::{DIFF_BUDGET, DIFF_CHUNK, ViewFile, next_difference};

const NONE: ByteDifference = ByteDifference::None;

fn next(left: &ViewFile, right: &ViewFile, from: u64, forward: bool) -> ByteDifference {
    next_difference(left, right, from, forward, DIFF_BUDGET, &|| false).expect("a scan")
}

/// Two files that are the same, `size` bytes long, but for the bytes at `changes` of the second.
fn pair(size: usize, changes: &[usize]) -> (tempfile::TempDir, tempfile::TempDir, ViewFile, ViewFile) {
    let bytes: Vec<u8> = (0..size).map(|at| (at % 251) as u8).collect();
    let mut other = bytes.clone();
    for at in changes {
        other[*at] ^= 0xFF;
    }

    let (first, left) = file_of(&bytes);
    let (second, right) = file_of(&other);

    (first, second, left, right)
}

#[test]
fn identical_files_have_no_difference_either_way() {
    let (_a, _b, left, right) = pair(3 * DIFF_CHUNK + 5, &[]);

    assert_eq!(next(&left, &right, 0, true), NONE);
    assert_eq!(next(&left, &right, u64::MAX, false), NONE);
}

#[test]
fn finds_a_difference_at_the_start_in_the_middle_and_across_a_chunk_border() {
    let border = DIFF_CHUNK;
    let (_a, _b, left, right) = pair(3 * DIFF_CHUNK, &[0, 100_000, border - 1, border, 2 * border + 7]);

    let mut forward = Vec::new();
    let mut from = 0;
    while let Found { offset } = next(&left, &right, from, true) {
        forward.push(offset);
        from = offset + 1;
    }

    let all = [0, 100_000, border as u64 - 1, border as u64, 2 * border as u64 + 7];
    assert_eq!(forward, all);

    let mut backward = Vec::new();
    let mut from = 3 * DIFF_CHUNK as u64;
    while let Found { offset } = next(&left, &right, from, false) {
        backward.push(offset);
        from = offset;
    }

    backward.reverse();
    assert_eq!(backward, all);
}

#[test]
fn forward_starts_at_from_and_backward_stops_before_it() {
    let (_a, _b, left, right) = pair(1000, &[10, 20]);

    assert_eq!(next(&left, &right, 10, true), Found { offset: 10 });
    assert_eq!(next(&left, &right, 11, true), Found { offset: 20 });
    assert_eq!(next(&left, &right, 21, true), NONE);
    assert_eq!(next(&left, &right, 20, false), Found { offset: 10 });
    assert_eq!(next(&left, &right, 10, false), NONE);
    assert_eq!(next(&left, &right, 21, false), Found { offset: 20 });
}

#[test]
fn the_bytes_only_the_longer_file_has_differ() {
    let (_a, left) = file_of(b"abcdef");
    let (_b, right) = file_of(b"abcd");

    assert_eq!(next(&left, &right, 0, true), Found { offset: 4 });
    assert_eq!(next(&left, &right, 5, true), Found { offset: 5 });
    assert_eq!(next(&left, &right, 6, true), NONE);
    assert_eq!(next(&left, &right, 6, false), Found { offset: 5 });
    assert_eq!(next(&left, &right, 4, false), NONE);
    assert_eq!(next(&right, &left, 0, true), Found { offset: 4 });

    let (_c, other) = file_of(b"abXd");
    assert_eq!(next(&other, &left, 0, true), Found { offset: 2 });
    assert_eq!(next(&other, &left, 4, false), Found { offset: 2 });
}

#[test]
fn empty_files_and_offsets_past_both_ends() {
    let (_a, empty) = file_of(b"");
    let (_b, other) = file_of(b"");
    let (_c, some) = file_of(b"xyz");

    assert_eq!(next(&empty, &other, 0, true), NONE);
    assert_eq!(next(&empty, &other, 0, false), NONE);
    assert_eq!(next(&empty, &some, 0, true), Found { offset: 0 });
    assert_eq!(next(&empty, &some, u64::MAX, false), Found { offset: 2 });
    assert_eq!(next(&some, &some, 99, true), NONE);
    assert_eq!(next(&some, &some, 99, false), NONE);
    assert_eq!(next(&empty, &some, 3, true), NONE);
}

#[test]
fn a_spent_budget_answers_searching_and_asking_again_finds_it() {
    let (_a, _b, left, right) = pair(4 * DIFF_CHUNK, &[3 * DIFF_CHUNK + 9]);
    let budget = 2 * DIFF_CHUNK as u64;
    let ask = |from, forward| next_difference(&left, &right, from, forward, budget, &|| false).expect("a scan");

    let Searching { from } = ask(0, true) else {
        panic!("the budget runs out first");
    };
    assert_eq!(from, budget);
    assert_eq!(
        ask(from, true),
        Found {
            offset: 3 * DIFF_CHUNK as u64 + 9
        }
    );

    let Searching { from } = ask(3 * DIFF_CHUNK as u64, false) else {
        panic!("the budget runs out first");
    };
    assert_eq!(from, DIFF_CHUNK as u64);
    assert_eq!(ask(from, false), NONE);
    assert_eq!(
        ask(4 * DIFF_CHUNK as u64, false),
        Found {
            offset: 3 * DIFF_CHUNK as u64 + 9
        }
    );
}

#[test]
fn a_cancelled_scan_stops_between_chunks_with_interrupted() {
    let (_a, _b, left, right) = pair(4 * DIFF_CHUNK, &[]);
    let looked = Cell::new(0);
    let cancel = || {
        looked.set(looked.get() + 1);

        looked.get() > 1
    };

    let error = next_difference(&left, &right, 0, true, DIFF_BUDGET, &cancel).expect_err("stopped");

    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert_eq!(looked.get(), 2, "one chunk was read, then the flag was seen");

    let error = next_difference(&left, &right, u64::MAX, false, DIFF_BUDGET, &|| true).expect_err("stopped");
    assert_eq!(error.kind(), ErrorKind::Interrupted);
}

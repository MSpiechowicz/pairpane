//! The rule that decides a pair: the tolerance of the times, the sizes, the content, folders and links.

use pretty_assertions::assert_eq;

use crate::model::{CompareFacts, CompareStatus, EntryKind};

use super::{TOLERANCE_MS, status};

fn file(size: u64, modified: Option<i64>) -> CompareFacts {
    CompareFacts {
        kind: EntryKind::File,
        link: false,
        size,
        modified,
        files: 0,
        bytes: 0,
    }
}

fn of_kind(kind: EntryKind, link: bool) -> CompareFacts {
    CompareFacts {
        kind,
        link,
        ..file(0, None)
    }
}

/// The status of a left file at `1_000_000 + apart` against a right one at `1_000_000`, both of 5 bytes.
fn apart(apart: i64) -> CompareStatus {
    status(&file(5, Some(1_000_000 + apart)), &file(5, Some(1_000_000)), None)
}

#[test]
fn times_within_the_tolerance_are_the_same_and_beyond_it_the_later_one_is_newer() {
    assert_eq!(TOLERANCE_MS, 2_000);

    assert_eq!(apart(0), CompareStatus::Equal);
    assert_eq!(apart(1_999), CompareStatus::Equal);
    assert_eq!(apart(2_000), CompareStatus::Equal);
    assert_eq!(apart(2_001), CompareStatus::LeftNewer);
    assert_eq!(apart(-1_999), CompareStatus::Equal);
    assert_eq!(apart(-2_000), CompareStatus::Equal);
    assert_eq!(apart(-2_001), CompareStatus::RightNewer);
}

#[test]
fn the_later_file_is_newer_whatever_the_sizes() {
    let newer = status(&file(9, Some(10_000)), &file(5, Some(1_000)), None);
    let older = status(&file(5, Some(1_000)), &file(9, Some(10_000)), None);

    assert_eq!(newer, CompareStatus::LeftNewer);
    assert_eq!(older, CompareStatus::RightNewer);
}

#[test]
fn files_of_one_time_and_different_sizes_differ() {
    assert_eq!(
        status(&file(5, Some(1_000)), &file(6, Some(1_000)), None),
        CompareStatus::Differs
    );
    // Within the tolerance counts as one time.
    assert_eq!(
        status(&file(5, Some(1_000)), &file(6, Some(2_500)), None),
        CompareStatus::Differs
    );
}

#[test]
fn a_file_with_no_time_is_judged_by_its_size() {
    assert_eq!(
        status(&file(5, None), &file(5, Some(1_000)), None),
        CompareStatus::Equal
    );
    assert_eq!(status(&file(5, None), &file(6, None), None), CompareStatus::Differs);
}

#[test]
fn what_the_bytes_say_settles_a_pair_of_one_size() {
    // Equal bytes are the same file, whatever the times say.
    assert_eq!(
        status(&file(5, Some(10_000)), &file(5, Some(1_000)), Some(true)),
        CompareStatus::Equal
    );
    // Different bytes at one time differ; at two times the later one is newer.
    assert_eq!(
        status(&file(5, Some(1_000)), &file(5, Some(1_000)), Some(false)),
        CompareStatus::Differs
    );
    assert_eq!(
        status(&file(5, Some(10_000)), &file(5, Some(1_000)), Some(false)),
        CompareStatus::LeftNewer
    );
}

#[test]
fn folders_are_compared_by_presence_and_other_kinds_by_what_they_are() {
    let folder = of_kind(EntryKind::Dir, false);
    let link = of_kind(EntryKind::Other, true);
    let special = of_kind(EntryKind::Other, false);

    assert_eq!(status(&folder, &folder, None), CompareStatus::Equal);
    assert_eq!(status(&special, &special, None), CompareStatus::Equal);
    assert_eq!(status(&folder, &file(0, None), None), CompareStatus::Differs);
    assert_eq!(status(&link, &file(0, None), None), CompareStatus::Differs);
    assert_eq!(status(&link, &special, None), CompareStatus::Differs);
}

#[test]
fn two_links_are_the_same_only_when_they_lead_to_the_same_place() {
    let link = of_kind(EntryKind::Other, true);

    assert_eq!(status(&link, &link, Some(true)), CompareStatus::Equal);
    assert_eq!(status(&link, &link, Some(false)), CompareStatus::Differs);
    // A target that could not be read is no proof of a match.
    assert_eq!(status(&link, &link, None), CompareStatus::Differs);
}

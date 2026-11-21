//! The `Range` header.

use pretty_assertions::assert_eq;

use super::range::{MAX_HEADER, Requested, parse};

const CAP: u64 = 100;

fn bytes(start: u64, end: u64) -> Requested {
    Requested::Bytes { start, end }
}

#[test]
fn the_forms_of_a_single_range() {
    let table = [
        ("bytes=0-9", 1000, bytes(0, 9)),
        ("bytes=10-19", 1000, bytes(10, 19)),
        ("bytes=990-", 1000, bytes(990, 999)),
        ("bytes=0-", 50, bytes(0, 49)),
        ("bytes=-10", 1000, bytes(990, 999)),
        ("bytes=-5000", 80, bytes(0, 79)),
        ("bytes=5-5", 1000, bytes(5, 5)),
        ("bytes=0-99999", 1000, bytes(0, 99)),
        ("bytes=950-99999", 1000, bytes(950, 999)),
        ("BYTES=1-2", 1000, bytes(1, 2)),
        ("bytes = 1-2", 1000, bytes(1, 2)),
        (" bytes=1-2 ", 1000, bytes(1, 2)),
    ];

    for (header, size, expected) in table {
        assert_eq!(parse(Some(header), size, CAP), expected, "{header}");
    }
}

#[test]
fn a_range_is_clamped_to_the_cap() {
    assert_eq!(parse(Some("bytes=0-"), 10_000, CAP), bytes(0, 99));
    assert_eq!(parse(Some("bytes=500-"), 10_000, CAP), bytes(500, 599));
    assert_eq!(parse(Some("bytes=-1000"), 10_000, CAP), bytes(9000, 9099));
    assert_eq!(parse(Some("bytes=0-10"), 10_000, 1), bytes(0, 0));
    assert_eq!(parse(Some("bytes=0-10"), 10_000, 0), bytes(0, 0));
}

#[test]
fn a_range_past_the_end_cannot_be_satisfied() {
    let table = [
        ("bytes=1000-", 1000),
        ("bytes=1000-1005", 1000),
        ("bytes=5000-", 1000),
        ("bytes=-0", 1000),
        ("bytes=0-", 0),
        ("bytes=-5", 0),
        ("bytes=0-0", 0),
    ];

    for (header, size) in table {
        assert_eq!(
            parse(Some(header), size, CAP),
            Requested::Unsatisfiable,
            "{header} of {size}"
        );
    }
}

#[test]
fn what_is_not_one_good_bytes_range_is_ignored() {
    let long = format!("bytes=0-{}", "9".repeat(MAX_HEADER));
    let table = [
        "",
        "bytes",
        "bytes=",
        "bytes=-",
        "bytes=abc",
        "bytes=1-2,5-6",
        "bytes=1-2, 5-6",
        "items=1-2",
        "lines=1-2",
        "bytes=5-2",
        "bytes=a-2",
        "bytes=1-b",
        "bytes=+1-2",
        "bytes=1.5-2",
        "bytes=0x1-2",
        "bytes=99999999999999999999999-",
        "bytes=1-99999999999999999999999",
        long.as_str(),
    ];

    for header in table {
        assert_eq!(parse(Some(header), 1000, CAP), Requested::Whole, "{header}");
    }

    assert_eq!(parse(None, 1000, CAP), Requested::Whole);
}

#[test]
fn big_numbers_do_not_overflow() {
    let max = u64::MAX.to_string();

    assert_eq!(
        parse(Some(&format!("bytes={max}-")), 1000, CAP),
        Requested::Unsatisfiable
    );
    assert_eq!(parse(Some(&format!("bytes=0-{max}")), 1000, CAP), bytes(0, 99));
    assert_eq!(parse(Some(&format!("bytes=-{max}")), 1000, CAP), bytes(0, 99));
    assert_eq!(
        parse(Some("bytes=18446744073709551614-"), u64::MAX, u64::MAX),
        bytes(u64::MAX - 1, u64::MAX - 1)
    );
    assert_eq!(parse(Some("bytes=0-"), u64::MAX, u64::MAX), bytes(0, u64::MAX - 1));
}

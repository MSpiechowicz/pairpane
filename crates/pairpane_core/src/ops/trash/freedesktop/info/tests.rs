use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use jiff::civil::date;
use jiff::tz::TimeZone;
use pretty_assertions::assert_eq;

use super::{date_of, deletion_date, format_date, parse_path, render};

#[test]
fn a_record_has_the_header_the_encoded_path_and_the_local_date() {
    let text = render(Path::new("/home/me/a b/c.txt"), "2026-04-01T12:30:00");

    assert_eq!(
        text,
        "[Trash Info]\nPath=/home/me/a%20b/c.txt\nDeletionDate=2026-04-01T12:30:00\n"
    );
}

#[test]
fn a_path_is_encoded_byte_by_byte_and_decodes_to_the_same_path() {
    let odd = PathBuf::from(OsString::from_vec(b"/x/\xff\xfe odd".to_vec()));
    let paths = [
        PathBuf::from("/plain/Path-1_2.3~/x"),
        PathBuf::from("/new\nline/100%/back\\slash/#hash/?q=1&r"),
        PathBuf::from("/zażółć/日本語"),
        PathBuf::from("relative/under/top"),
        odd,
    ];

    for path in paths {
        let text = render(&path, "2026-04-01T12:30:00");

        assert_eq!(text.lines().count(), 3, "{path:?}: one line each");
        assert!(text.is_ascii(), "{path:?}");
        assert_eq!(parse_path(&text), Some(path.clone()), "{path:?}");
    }

    let text = render(Path::new("/a=b"), "2026-04-01T12:30:00");
    assert!(text.contains("Path=/a%3Db\n"), "{text}");
}

#[test]
fn the_date_is_local_time_to_the_second_with_no_zone() {
    let zone = TimeZone::fixed(jiff::tz::offset(2));
    let time = date(2026, 4, 1).at(7, 5, 9, 0).to_zoned_datetime(zone).expect("a time");

    assert_eq!(format_date(&time), "2026-04-01T07:05:09");
    assert_eq!(
        format_date(
            &date(1999, 12, 31)
                .at(23, 59, 59, 0)
                .to_zoned_datetime(TimeZone::UTC)
                .expect("a time")
        ),
        "1999-12-31T23:59:59"
    );
}

#[test]
fn the_date_of_now_has_the_shape_of_the_specification() {
    let now = deletion_date().expect("a date");
    let bytes = now.as_bytes();

    assert_eq!(bytes.len(), 19, "{now}");
    for (at, separator) in [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')] {
        assert_eq!(bytes[at], separator, "{now}");
    }
    assert!(
        now.chars()
            .enumerate()
            .all(|(at, c)| [4, 7, 10, 13, 16].contains(&at) || c.is_ascii_digit()),
        "{now}"
    );
}

#[test]
fn text_that_is_not_a_record_has_no_path() {
    for text in [
        "",
        "Path=/x\n",
        "[Other]\nPath=/x\n",
        "[Trash Info]\nDeletionDate=2026-04-01T12:30:00\n",
        "[Trash Info]\nPath\n",
    ] {
        assert_eq!(parse_path(text), None, "{text:?}");
    }
}

#[test]
fn the_first_path_counts_and_other_lines_are_ignored() {
    let text = "[Trash Info]\nExtra=1\nPath=/first\nPath=/second\nDeletionDate=never\n";

    assert_eq!(parse_path(text), Some(PathBuf::from("/first")));
    // A line is split at the first `=`: a name can hold more.
    assert_eq!(parse_path("[Trash Info]\nPath=/a=b\n"), Some(PathBuf::from("/a=b")));
    // A `%` that is not followed by two hex digits stays.
    assert_eq!(
        parse_path("[Trash Info]\nPath=/100%/%zz\n"),
        Some(PathBuf::from("/100%/%zz"))
    );
}

#[test]
fn a_clock_outside_the_years_a_date_holds_is_refused_and_does_not_panic() {
    // About 31 million years from 1970: past the last year a date can hold, and still a time the system can name.
    let far = std::time::Duration::from_secs(1_000_000_000_000_000);
    let times = [
        std::time::SystemTime::UNIX_EPOCH.checked_add(far),
        std::time::SystemTime::UNIX_EPOCH.checked_sub(far),
    ];

    for time in times.into_iter().flatten() {
        let error = date_of(time).expect_err("out of range");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), "the system clock is out of range");
    }
}

#[test]
fn a_time_in_range_is_written_as_a_date() {
    let date = date_of(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400 * 365)).expect("date");

    // The local zone moves it by at most a day: 1970-12-31 or 1971-01-01, or the day around.
    assert!(date.starts_with("1971-") || date.starts_with("1970-"), "{date}");
}

//! The checks on entry names: the unsafe ones are refused whatever the system, the plain ones come out as one
//! normalized path.

use pretty_assertions::assert_eq;

use super::names::{Unsafe, is_hostile, normalize};
use crate::ops::{MAX_FOLDER_DEPTH, MAX_NAME_UNITS};

#[test]
fn a_plain_name_is_kept_and_odd_separators_are_tidied() {
    let cases = [
        ("a.txt", "a.txt"),
        ("dir/sub/file.bin", "dir/sub/file.bin"),
        ("dir/", "dir"),
        ("./a/b", "a/b"),
        ("a//b/./c/", "a/b/c"),
        ("zażółć/gęślą.txt", "zażółć/gęślą.txt"),
        ("..hidden/x", "..hidden/x"),
    ];

    for (raw, tidy) in cases {
        assert_eq!(normalize(raw), Ok(Some(tidy.to_owned())), "{raw:?}");
    }
}

#[test]
fn a_colon_is_a_name_on_unix_and_refused_on_windows() {
    // On Windows a colon makes a stream or a drive, so no file can be named with one.
    if cfg!(windows) {
        assert_eq!(normalize("ab:c/d"), Err(Unsafe::Reserved));
    } else {
        assert_eq!(normalize("ab:c/d"), Ok(Some("ab:c/d".to_owned())));
    }
}

#[test]
fn a_name_with_no_part_is_the_archive_root() {
    for raw in ["", ".", "./", "./."] {
        assert_eq!(normalize(raw), Ok(None), "{raw:?}");
    }
}

#[test]
fn a_name_that_climbs_or_starts_at_a_root_is_refused() {
    let cases = [
        ("../x", Unsafe::Parent),
        ("a/../../x", Unsafe::Parent),
        ("a/b/../../../x", Unsafe::Parent),
        // Inside the folder it resolves to, but no archive has a reason to spell it so.
        ("a/../b", Unsafe::Parent),
        ("..", Unsafe::Parent),
        ("x/..", Unsafe::Parent),
        ("/abs", Unsafe::Absolute),
        ("/", Unsafe::Absolute),
        ("//host/share/x", Unsafe::Absolute),
        ("C:\\x", Unsafe::Backslash),
        ("C:x", Unsafe::Drive),
        ("c:/x", Unsafe::Drive),
        ("\\\\host\\x", Unsafe::Backslash),
        ("..\\x", Unsafe::Backslash),
        ("a\\..\\..\\x", Unsafe::Backslash),
        ("a\\b", Unsafe::Backslash),
    ];

    for (raw, why) in cases {
        assert_eq!(normalize(raw), Err(why), "{raw:?}");
    }
}

#[test]
fn a_control_or_bidirectional_character_refuses_the_name() {
    let cases = [
        "a\0b",
        "nul\0.txt",
        "a\nb",
        "a\tb",
        "a\x1bb",
        "a\x7fb",
        "a\u{85}b",
        "a\u{202E}fdp.exe",
        "a\u{2067}b",
        "a\u{200F}b",
        "a\u{61C}b",
    ];

    for raw in cases {
        assert_eq!(normalize(raw), Err(Unsafe::Control), "{raw:?}");
    }

    assert!(!is_hostile('é') && !is_hostile('\u{FFFD}') && !is_hostile(' '));
}

#[test]
fn a_part_over_255_units_or_a_path_over_the_depth_is_refused() {
    let longest = "n".repeat(MAX_NAME_UNITS);
    assert_eq!(normalize(&longest), Ok(Some(longest.clone())));
    assert_eq!(normalize(&format!("{longest}x")), Err(Unsafe::TooLong));
    // Units are bytes on Unix and UTF-16 units on Windows, so 255 two-byte characters fit only on Windows.
    let wide = format!("a/{}", "é".repeat(MAX_NAME_UNITS));
    if cfg!(windows) {
        assert!(normalize(&wide).is_ok());
    } else {
        assert_eq!(normalize(&wide), Err(Unsafe::TooLong));
    }
    assert_eq!(
        normalize(&format!("a/{}", "é".repeat(MAX_NAME_UNITS + 1))),
        Err(Unsafe::TooLong)
    );

    let deepest = vec!["d"; MAX_FOLDER_DEPTH].join("/");
    assert_eq!(normalize(&deepest), Ok(Some(deepest.clone())));
    assert_eq!(normalize(&format!("{deepest}/x")), Err(Unsafe::TooDeep));

    assert_eq!(normalize(&"a/".repeat(3000)), Err(Unsafe::TooLong));
}

/// Windows reads these as devices or streams, so there they are refused; elsewhere they are files like any other.
#[test]
fn windows_device_names_are_refused_on_windows_only() {
    let names = [
        "NUL",
        "con.txt",
        "a/COM1",
        "aux.tar.gz",
        "LPT9",
        "a/prn ",
        "file.",
        "name ",
    ];

    for raw in names {
        let expected = if cfg!(windows) {
            Err(Unsafe::Reserved)
        } else {
            Ok(Some(raw.trim_end_matches('/').to_owned()))
        };

        assert_eq!(normalize(raw), expected, "{raw:?}");
    }
}

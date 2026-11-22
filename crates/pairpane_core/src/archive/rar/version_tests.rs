//! The version banner of `unrar`, as it is read to refuse an `unrar` older than 7.

use pretty_assertions::assert_eq;

use super::version::parse_major;
use super::{TOO_OLD, too_old};

#[test]
fn the_major_version_is_read_from_the_banner_of_each_release() {
    for (banner, major) in [
        (
            "\nUNRAR 7.30 beta 2 freeware      Copyright (c) 1993-2026 Alexander Roshal\n\nUsage: ...",
            7,
        ),
        ("UNRAR 7.00 freeware      Copyright (c) 1993-2024 Alexander Roshal", 7),
        (
            "\r\nUNRAR 6.24 freeware      Copyright (c) 1993-2023 Alexander Roshal\r\n",
            6,
        ),
        ("UNRAR 5.61 beta 1 freeware", 5),
        ("RAR 7.10   Copyright (c) 1993-2025 Alexander Roshal", 7),
        ("UNRAR 10.0 freeware", 10),
    ] {
        assert_eq!(parse_major(banner), Some(major), "{banner}");
    }
}

#[test]
fn a_text_that_is_no_banner_has_no_version() {
    for text in [
        "",
        "usage: unrar",
        "UNRAR",
        "UNRAR freeware 7",
        "UNRAR x.y",
        "UNRAR 7",
        "7.30 UNRAR",
    ] {
        assert_eq!(parse_major(text), None, "{text:?}");
    }
}

#[test]
fn an_old_unrar_is_refused_with_a_fixed_text_a_page_shows_as_it_is() {
    let error = too_old();

    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(error.to_string(), TOO_OLD);
    assert_eq!(TOO_OLD, "unrar 7 or newer is needed to open RAR archives");
    assert!(crate::view::is_refusal(&error));
}

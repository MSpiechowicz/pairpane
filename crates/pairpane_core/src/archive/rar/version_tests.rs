//! The version banner of `unrar`, as it is read to refuse an `unrar` older than 7.

use std::io::{self, Cursor};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use pretty_assertions::assert_eq;

use super::too_old;
use super::version::{Version, banner_text, parse_version, require_with};
use crate::view::is_refusal;

fn major(text: &str) -> Option<u32> {
    parse_version(text).map(|version| version.major)
}

fn shown(text: &str) -> Option<String> {
    parse_version(text).map(|version| version.to_string())
}

/// A version as a program prints it.
fn banner(text: &str) -> Option<Version> {
    parse_version(text)
}

#[test]
fn the_version_is_read_from_the_banner_of_each_release() {
    for (text, expected, printed) in [
        (
            "\nUNRAR 7.30 beta 2 freeware      Copyright (c) 1993-2026 Alexander Roshal\n\nUsage: ...",
            7,
            "7.30",
        ),
        (
            "UNRAR 7.00 freeware      Copyright (c) 1993-2024 Alexander Roshal",
            7,
            "7.00",
        ),
        (
            "\r\nUNRAR 6.24 freeware      Copyright (c) 1993-2023 Alexander Roshal\r\n",
            6,
            "6.24",
        ),
        ("UNRAR 5.61 beta 1 freeware", 5, "5.61"),
        ("RAR 7.10   Copyright (c) 1993-2025 Alexander Roshal", 7, "7.10"),
        ("UNRAR 10.0 freeware", 10, "10.0"),
    ] {
        assert_eq!(major(text), Some(expected), "{text}");
        assert_eq!(shown(text).as_deref(), Some(printed), "{text}");
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
        "UNRAR 7.x",
        "UNRAR 7.123456 freeware",
    ] {
        assert_eq!(major(text), None, "{text:?}");
    }
}

#[test]
fn an_old_unrar_is_refused_with_a_fixed_text_naming_its_version_that_a_page_shows_as_it_is() {
    let error = too_old(Some("6.21"));

    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(
        error.to_string(),
        "The installed unrar 6.21 is not compatible. Update unrar to version 7 or newer to open RAR archives."
    );
    assert!(is_refusal(&error));

    let unknown = too_old(None);

    assert_eq!(unknown.kind(), io::ErrorKind::Unsupported);
    assert_eq!(
        unknown.to_string(),
        "The installed unrar is not compatible. Update unrar to version 7 or newer to open RAR archives."
    );
    assert!(is_refusal(&unknown));
}

#[test]
fn a_banner_cut_inside_a_character_still_has_its_version() {
    // "é" is two bytes; the limit falls between them.
    let mut bytes = b"UNRAR 6.21 freeware\n".to_vec();
    bytes.resize(4095, b' ');
    bytes.extend_from_slice("\u{e9}tail".as_bytes());
    assert!(std::str::from_utf8(&bytes[..4096]).is_err());

    let text = banner_text(Cursor::new(bytes));

    assert_eq!(shown(&text).as_deref(), Some("6.21"));
    assert!(text.len() <= 4096 + 2, "only the start is read");
}

#[test]
fn a_program_is_asked_again_until_its_version_was_read_and_then_it_is_remembered() {
    let program = PathBuf::from("/nonexistent/pairpane/unrar-cache-test");
    let asked = AtomicU32::new(0);
    let answers = [None, None, banner("UNRAR 6.21 freeware")];
    let ask = |_: &std::path::Path| {
        let at = asked.fetch_add(1, Ordering::Relaxed) as usize;

        answers[at].clone()
    };

    // Unreadable banners are let through and not kept.
    assert!(require_with(&program, ask).is_ok());
    assert!(require_with(&program, ask).is_ok());

    // The version that was read is refused, and kept: the next call does not ask.
    let error = require_with(&program, ask).expect_err("too old");
    assert!(
        error
            .to_string()
            .starts_with("The installed unrar 6.21 is not compatible.")
    );

    let again = require_with(&program, ask).expect_err("still too old");
    assert_eq!(again.to_string(), error.to_string());
    assert_eq!(asked.load(Ordering::Relaxed), 3);
}

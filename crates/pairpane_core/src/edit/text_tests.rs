//! Reading bytes as text and writing text as bytes: nothing is replaced, nothing is lost, nothing is guessed twice.

use std::io::ErrorKind;

use encoding_rs::{SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1250};
use pretty_assertions::assert_eq;

use crate::model::{EditSaveOptions, LineEnding};

use super::MAX_SAVE_TEXT;
use super::testing::options;
use super::text::{Breaks, Decoded, breaks, decode, encode, normalise};

fn utf16(text: &str, big: bool, mark: bool) -> Vec<u8> {
    let mut units: Vec<u16> = Vec::new();
    if mark {
        units.push(0xFEFF);
    }
    units.extend(text.encode_utf16());

    units
        .iter()
        .flat_map(|unit| if big { unit.to_be_bytes() } else { unit.to_le_bytes() })
        .collect()
}

/// What `encode` writes for `text` with the encoding, mark and line break.
fn written(text: &str, encoding: &str, bom: bool, line_ending: LineEnding) -> Vec<u8> {
    let options = EditSaveOptions {
        encoding: encoding.to_owned(),
        bom,
        line_ending,
        ..options("")
    };

    encode(text, &options).expect("encoded")
}

/// Reads `bytes`, then writes what was read the way it was found, and expects the very same bytes.
fn round_trip(bytes: &[u8], label: Option<&str>) {
    let Decoded::Text { text, encoding, bom } = decode(bytes, label).expect("decoded") else {
        panic!("not text: {bytes:?}");
    };
    let found = breaks(&text);

    assert_eq!(written(&text, encoding.name(), bom, found.ending), bytes, "{label:?}");
}

#[test]
fn text_in_every_encoding_is_written_back_byte_for_byte() {
    let text = "zażółć gęślą jaźń\nline two\n";
    let crlf = text.replace('\n', "\r\n");
    let cr = text.replace('\n', "\r");

    round_trip(text.as_bytes(), None);
    round_trip(&[&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(), None);
    round_trip(crlf.as_bytes(), None);
    round_trip(cr.as_bytes(), None);

    for (big, mark) in [(false, false), (false, true), (true, false), (true, true)] {
        round_trip(&utf16(text, big, mark), None);
        round_trip(&utf16(&crlf, big, mark), None);
    }

    let (latin2, _, lost) = WINDOWS_1250.encode(text);
    assert!(!lost);
    round_trip(&latin2, Some("windows-1250"));

    let (japanese, _, lost) = SHIFT_JIS.encode("日本語のテキスト\r\n二行目\r\n");
    assert!(!lost);
    round_trip(&japanese, Some("Shift_JIS"));
}

#[test]
fn surrogate_pairs_survive_in_both_utf16_orders() {
    let text = "a😀b\u{1F9EA}\n";

    for big in [false, true] {
        let bytes = utf16(text, big, true);
        let Decoded::Text {
            text: read,
            encoding,
            bom,
        } = decode(&bytes, None).expect("decoded")
        else {
            panic!("not text");
        };

        assert_eq!(read, text);
        assert_eq!(encoding, if big { UTF_16BE } else { UTF_16LE });
        assert!(bom);
        round_trip(&bytes, None);
    }
}

#[test]
fn a_mark_is_not_part_of_the_text_and_belongs_to_its_own_encoding_only() {
    let with_mark = [&[0xEF, 0xBB, 0xBF][..], b"abc"].concat();

    assert_eq!(
        decode(&with_mark, None).expect("decoded"),
        Decoded::Text {
            text: "abc".to_owned(),
            encoding: UTF_8,
            bom: true
        }
    );
    assert_eq!(
        decode(&with_mark, Some("windows-1250")).expect("decoded"),
        Decoded::Text {
            text: "ď»żabc".to_owned(),
            encoding: WINDOWS_1250,
            bom: false
        }
    );
}

#[test]
fn bytes_that_are_not_valid_are_undecodable_and_never_replaced() {
    let broken = [b'a', 0xFF, b'b'];

    assert_eq!(
        decode(&broken, Some("UTF-8")).expect("decoded"),
        Decoded::Undecodable(UTF_8)
    );

    // The same bytes are text in an encoding that has a character for every byte they hold.
    assert!(matches!(
        decode(&broken, Some("windows-1250")).expect("decoded"),
        Decoded::Text { .. }
    ));

    // An unpaired surrogate and an odd length are not UTF-16.
    let lone = [0x3D, 0xD8, b'a', 0x00];
    assert_eq!(
        decode(&lone, Some("UTF-16LE")).expect("decoded"),
        Decoded::Undecodable(UTF_16LE)
    );
    assert_eq!(
        decode(&[0x61, 0x00, 0x62], Some("UTF-16LE")).expect("decoded"),
        Decoded::Undecodable(UTF_16LE)
    );
}

#[test]
fn a_nul_byte_makes_a_file_binary_unless_it_is_utf16() {
    assert_eq!(decode(b"ab\0cd", None).expect("decoded"), Decoded::Binary);
    assert_eq!(
        decode(b"ab\0cd", Some("windows-1250")).expect("decoded"),
        Decoded::Binary
    );

    let bytes = utf16("some text of UTF-16\n", false, false);
    assert!(bytes.contains(&0));
    assert!(matches!(decode(&bytes, None).expect("decoded"), Decoded::Text { .. }));
}

#[test]
fn an_encoding_that_is_not_known_is_refused_with_a_fixed_text() {
    for label in ["nonsense", "", "replacement", "iso-2022-kr"] {
        let error = decode(b"abc", Some(label)).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{label:?}");
        assert_eq!(error.to_string(), "the encoding is not known");
    }
}

#[test]
fn the_most_common_line_break_wins_and_a_mix_is_told() {
    let cases = [
        ("no break", LineEnding::Lf, false),
        ("a\nb\nc\n", LineEnding::Lf, false),
        ("a\r\nb\r\nc\r\n", LineEnding::Crlf, false),
        ("a\rb\rc\r", LineEnding::Cr, false),
        ("a\r\nb\r\nc\nd", LineEnding::Crlf, true),
        ("a\rb\rc\r\nd\n", LineEnding::Cr, true),
        ("a\nb\r\n", LineEnding::Lf, true),
        ("a\r\nb\r", LineEnding::Crlf, true),
        ("\r\n\n\r", LineEnding::Lf, true),
    ];

    for (text, ending, mixed) in cases {
        assert_eq!(breaks(text), Breaks { ending, mixed }, "{text:?}");
    }
}

#[test]
fn every_kind_of_break_becomes_one_newline() {
    assert_eq!(normalise("a\r\nb\rc\nd\r\n\re"), "a\nb\nc\nd\n\ne");
}

#[test]
fn every_break_is_written_as_the_chosen_one_whatever_the_text_has() {
    let text = "a\r\nb\rc\nd";

    assert_eq!(written(text, "UTF-8", false, LineEnding::Lf), b"a\nb\nc\nd");
    assert_eq!(written(text, "UTF-8", false, LineEnding::Crlf), b"a\r\nb\r\nc\r\nd");
    assert_eq!(written(text, "UTF-8", false, LineEnding::Cr), b"a\rb\rc\rd");
    assert_eq!(
        written("a\n", "UTF-16LE", false, LineEnding::Crlf),
        utf16("a\r\n", false, false)
    );
}

#[test]
fn a_mark_is_written_only_when_asked_for() {
    assert_eq!(written("a", "UTF-8", true, LineEnding::Lf), [0xEF, 0xBB, 0xBF, b'a']);
    assert_eq!(written("a", "UTF-16LE", true, LineEnding::Lf), [0xFF, 0xFE, b'a', 0]);
    assert_eq!(written("a", "UTF-16BE", true, LineEnding::Lf), [0xFE, 0xFF, 0, b'a']);
    assert_eq!(written("a", "UTF-16BE", false, LineEnding::Lf), [0, b'a']);
}

#[test]
fn characters_the_encoding_lacks_are_refused_without_naming_them() {
    let secret = "sekret 日本語";
    let options = EditSaveOptions {
        encoding: "windows-1250".to_owned(),
        ..options("")
    };

    let error = encode(secret, &options).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "some characters cannot be saved in this encoding");
    assert!(!error.to_string().contains("sekret"));
}

#[test]
fn a_mark_for_an_encoding_without_one_is_refused() {
    let options = EditSaveOptions {
        encoding: "windows-1250".to_owned(),
        bom: true,
        ..options("")
    };

    let error = encode("abc", &options).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "a byte order mark belongs to UTF-8 and UTF-16 only");
}

#[test]
fn a_text_over_the_cap_is_refused_and_one_at_the_cap_is_written() {
    let at_cap = "a".repeat(MAX_SAVE_TEXT);
    assert_eq!(encode(&at_cap, &options("")).expect("encoded").len(), MAX_SAVE_TEXT);

    let error = encode(&format!("{at_cap}a"), &options("")).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the text is too large to save");
}

#[test]
fn an_unknown_encoding_is_refused_on_save_too() {
    let options = EditSaveOptions {
        encoding: "klingon".to_owned(),
        ..options("")
    };

    assert_eq!(
        encode("a", &options).expect_err("refused").to_string(),
        "the encoding is not known"
    );
}

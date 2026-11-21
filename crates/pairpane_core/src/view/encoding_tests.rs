//! Detecting and naming text encodings.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1250, WINDOWS_1252};
use pretty_assertions::assert_eq;

use super::encoding::{bom_len, by_name, detect, is_known_encoding};

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

fn found(sample: &[u8]) -> &'static Encoding {
    detect(sample, true)
}

#[test]
fn byte_order_marks_decide() {
    assert_eq!(found(b"\xEF\xBB\xBFhello"), UTF_8);
    assert_eq!(found(&[&[0xFF, 0xFE][..], &utf16le("hi")].concat()), UTF_16LE);
    assert_eq!(found(&[&[0xFE, 0xFF][..], &utf16be("hi")].concat()), UTF_16BE);
}

#[test]
fn only_the_mark_of_the_encoding_in_use_is_skipped() {
    assert_eq!(bom_len(b"\xEF\xBB\xBFx", UTF_8), 3);
    assert_eq!(bom_len(b"\xEF\xBB\xBFx", WINDOWS_1252), 0);
    assert_eq!(bom_len(&[0xFF, 0xFE, 0x61, 0], UTF_16LE), 2);
    assert_eq!(bom_len(&[0xFF, 0xFE, 0x61, 0], UTF_16BE), 0);
    assert_eq!(bom_len(b"plain", UTF_8), 0);
    assert_eq!(bom_len(b"", UTF_8), 0);
}

#[test]
fn utf16_without_a_mark_is_told_by_the_zero_bytes() {
    let text = "Hello, wörld! This is text.\nSecond line.\n";

    assert_eq!(found(&utf16le(text)), UTF_16LE);
    assert_eq!(found(&utf16be(text)), UTF_16BE);
}

#[test]
fn binary_with_zeros_is_not_utf16() {
    assert_ne!(found(&[0, 1, 2, 3]), UTF_16BE);
    assert_ne!(found(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), UTF_16LE);
    let pattern: Vec<u8> = (0..64u8).flat_map(|n| [n.wrapping_mul(7), 0]).collect();
    assert_ne!(found(&pattern), UTF_16LE);
}

#[test]
fn ascii_and_valid_utf8_are_utf8() {
    assert_eq!(found(b"just ascii\n"), UTF_8);
    assert_eq!(found("zażółć gęślą jaźń\n".as_bytes()), UTF_8);
    assert_eq!(found(b""), UTF_8);
}

#[test]
fn a_character_cut_at_the_end_of_a_sample_is_not_an_error() {
    let text = "zażółć gęślą jaźń".repeat(20);
    let bytes = text.as_bytes();

    // Cut inside the last two-byte character, with more of the file to come.
    let cut = bytes.len() - 1;
    assert!(std::str::from_utf8(&bytes[..cut]).is_err());

    assert_eq!(detect(&bytes[..cut], false), UTF_8);
}

#[test]
fn polish_in_windows_1250_is_detected() {
    let text = "Zażółć gęślą jaźń. Żółć, łódź, źdźbło, ślusarz, ćma, kość. ".repeat(20);
    let (bytes, _, unmappable) = WINDOWS_1250.encode(&text);
    assert!(!unmappable);

    assert_eq!(found(&bytes), WINDOWS_1250);
}

#[test]
fn invalid_utf8_is_not_called_utf8() {
    assert_ne!(found(b"caf\xE9 au lait, cr\xE8me br\xFBl\xE9e\n"), UTF_8);
}

#[test]
fn finds_an_encoding_by_label() {
    assert_eq!(by_name("UTF-8").expect("known"), UTF_8);
    assert_eq!(by_name("utf-8").expect("known"), UTF_8);
    assert_eq!(by_name(" windows-1250 ").expect("known"), WINDOWS_1250);
    assert_eq!(by_name("latin1").expect("known"), WINDOWS_1252);
    assert_eq!(by_name("UTF-16LE").expect("known"), UTF_16LE);
}

#[test]
fn refuses_unknown_and_replacement_labels() {
    for label in ["", "nonsense", "replacement", "iso-2022-kr", "hz-gb-2312", "utf-7"] {
        let error = by_name(label).expect_err(label);

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{label}");
        assert_eq!(error.to_string(), "the encoding is not known", "{label}");
        assert!(!is_known_encoding(label), "{label}");
    }

    assert!(is_known_encoding("windows-1252"));
}

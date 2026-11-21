//! Searching: both directions, across chunk borders, case folding in several encodings, hex, cancelling.

use std::io::ErrorKind;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use crate::model::{SearchHit, SearchNeedle, SearchQuery};

use super::testing::file_of;
use super::{MAX_NEEDLE, SEARCH_CHUNK, ViewFile, search};

fn text(needle: &str, case_sensitive: bool, encoding: &str, from: u64, backward: bool) -> SearchQuery {
    SearchQuery {
        needle: SearchNeedle::Text {
            text: needle.to_owned(),
            case_sensitive,
        },
        encoding: encoding.to_owned(),
        from,
        backward,
    }
}

fn hex(bytes: &[u8], from: u64, backward: bool) -> SearchQuery {
    SearchQuery {
        needle: SearchNeedle::Hex { bytes: bytes.to_vec() },
        encoding: "UTF-8".to_owned(),
        from,
        backward,
    }
}

fn run(file: &ViewFile, query: &SearchQuery) -> Option<(u64, u64)> {
    let hit = search(file, query, &AtomicBool::new(false), |_| {}).expect("a search");

    hit.map(|SearchHit { offset, len }| (offset, len))
}

fn error_of(file: &ViewFile, query: &SearchQuery) -> (ErrorKind, String) {
    let error = search(file, query, &AtomicBool::new(false), |_| {}).expect_err("refused");

    (error.kind(), error.to_string())
}

fn utf16(text: &str, big: bool) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|unit| if big { unit.to_be_bytes() } else { unit.to_le_bytes() })
        .collect()
}

#[test]
fn finds_the_next_and_the_previous_match() {
    let (_folder, file) = file_of(b"abc abc abc");

    assert_eq!(run(&file, &text("abc", true, "UTF-8", 0, false)), Some((0, 3)));
    assert_eq!(run(&file, &text("abc", true, "UTF-8", 3, false)), Some((4, 3)));
    assert_eq!(run(&file, &text("abc", true, "UTF-8", 9, false)), None);
    assert_eq!(run(&file, &text("abc", true, "UTF-8", 11, true)), Some((8, 3)));
    assert_eq!(run(&file, &text("abc", true, "UTF-8", 8, true)), Some((4, 3)));
    assert_eq!(run(&file, &text("abc", true, "UTF-8", 1, true)), Some((0, 3)));
    assert_eq!(run(&file, &text("abc", true, "UTF-8", 0, true)), None);
    assert_eq!(run(&file, &text("abc", true, "UTF-8", u64::MAX, true)), Some((8, 3)));
    assert_eq!(run(&file, &text("abc", true, "UTF-8", u64::MAX, false)), None);
}

#[test]
fn a_missing_needle_finds_nothing() {
    let (_folder, file) = file_of(b"nothing to see");
    let empty = file_of(b"").1;

    assert_eq!(run(&file, &text("zzz", true, "UTF-8", 0, false)), None);
    assert_eq!(run(&file, &text("zzz", true, "UTF-8", 14, true)), None);
    assert_eq!(run(&empty, &text("a", true, "UTF-8", 0, false)), None);
    assert_eq!(run(&empty, &text("a", true, "UTF-8", 0, true)), None);
}

/// A file of two chunks and a bit, with `needle` written at `at`.
fn with_needle(at: usize, needle: &[u8]) -> (tempfile::TempDir, ViewFile) {
    let mut bytes = vec![b'.'; 2 * SEARCH_CHUNK + 10];
    bytes[at..at + needle.len()].copy_from_slice(needle);

    file_of(&bytes)
}

#[test]
fn finds_a_match_that_straddles_a_chunk_border_going_forward() {
    for at in [SEARCH_CHUNK - 3, SEARCH_CHUNK - 1, SEARCH_CHUNK, 2 * SEARCH_CHUNK - 3] {
        let (_folder, file) = with_needle(at, b"NEEDLE");

        assert_eq!(
            run(&file, &text("NEEDLE", true, "UTF-8", 0, false)),
            Some((at as u64, 6)),
            "{at}"
        );
        assert_eq!(run(&file, &hex(b"NEEDLE", 0, false)), Some((at as u64, 6)), "{at}");
    }
}

#[test]
fn finds_a_match_that_straddles_a_chunk_border_going_backward() {
    for at in [SEARCH_CHUNK - 3, SEARCH_CHUNK - 1, SEARCH_CHUNK + 10 - 3, 10] {
        let (_folder, file) = with_needle(at, b"NEEDLE");
        let size = file.size().expect("size");

        assert_eq!(
            run(&file, &text("NEEDLE", true, "UTF-8", size, true)),
            Some((at as u64, 6)),
            "{at}"
        );
        assert_eq!(run(&file, &hex(b"NEEDLE", size, true)), Some((at as u64, 6)), "{at}");
    }
}

#[test]
fn a_folded_needle_straddles_chunks_too() {
    let (_folder, file) = with_needle(SEARCH_CHUNK - 2, "ŁÓDŹ".as_bytes());
    let size = file.size().expect("size");

    assert_eq!(
        run(&file, &text("łódź", false, "UTF-8", 0, false)),
        Some((SEARCH_CHUNK as u64 - 2, 7))
    );
    assert_eq!(
        run(&file, &text("łódź", false, "UTF-8", size, true)),
        Some((SEARCH_CHUNK as u64 - 2, 7))
    );
}

#[test]
fn a_backward_search_finds_the_last_match_before_the_offset() {
    let mut bytes = vec![b'.'; 3 * SEARCH_CHUNK];
    for at in [5, SEARCH_CHUNK + 5, 2 * SEARCH_CHUNK + 5] {
        bytes[at..at + 3].copy_from_slice(b"hit");
    }
    let (_folder, file) = file_of(&bytes);
    let from = |offset: usize| run(&file, &text("hit", true, "UTF-8", offset as u64, true));

    assert_eq!(from(3 * SEARCH_CHUNK), Some((2 * SEARCH_CHUNK as u64 + 5, 3)));
    assert_eq!(from(2 * SEARCH_CHUNK + 5), Some((SEARCH_CHUNK as u64 + 5, 3)));
    assert_eq!(from(2 * SEARCH_CHUNK + 6), Some((2 * SEARCH_CHUNK as u64 + 5, 3)));
    assert_eq!(from(SEARCH_CHUNK + 5), Some((5, 3)));
    assert_eq!(from(5), None);
}

#[test]
fn case_insensitive_search_in_utf8_matches_polish_letters() {
    let text_in_file = "Zażółć GĘŚLĄ jaźń Łódź ł";
    let (_folder, file) = file_of(text_in_file.as_bytes());
    let start_of = |needle: &str| text_in_file.find(needle).expect("in text") as u64;

    assert_eq!(
        run(&file, &text("łódź", false, "UTF-8", 0, false)),
        Some((start_of("Łódź"), 7))
    );
    assert_eq!(
        run(&file, &text("ŁÓDŹ", false, "UTF-8", 0, false)),
        Some((start_of("Łódź"), 7))
    );
    assert_eq!(
        run(&file, &text("gęślą", false, "UTF-8", 0, false)),
        Some((start_of("GĘŚLĄ"), 8))
    );
    assert_eq!(
        run(&file, &text("Ł", false, "UTF-8", 0, false)),
        Some((start_of("ł"), 2))
    );
    assert_eq!(
        run(&file, &text("ł", false, "UTF-8", u64::MAX, true)),
        Some((text_in_file.len() as u64 - 2, 2))
    );
    assert_eq!(run(&file, &text("łódź", true, "UTF-8", 0, false)), None);
    assert_eq!(
        run(&file, &text("Łódź", true, "UTF-8", 0, false)),
        Some((start_of("Łódź"), 7))
    );
}

#[test]
fn case_insensitive_search_in_windows_1250() {
    let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode("Zażółć Łódź i łąka");
    let (_folder, file) = file_of(&bytes);

    // `Ł` is 0xA3 and `ł` 0xB3 in this encoding: one byte each.
    assert_eq!(run(&file, &text("ŁÓDŹ", false, "windows-1250", 0, false)), Some((7, 4)));
    assert_eq!(run(&file, &text("łódź", true, "windows-1250", 0, false)), None);
    assert_eq!(
        run(&file, &text("ŁĄKA", false, "windows-1250", u64::MAX, true)),
        Some((bytes.len() as u64 - 4, 4))
    );
}

#[test]
fn ascii_case_folding_goes_both_ways() {
    let (_folder, file) = file_of(b"say Hello, hELLO, hello");

    assert_eq!(run(&file, &text("HeLLo", false, "UTF-8", 0, false)), Some((4, 5)));
    assert_eq!(run(&file, &text("HeLLo", false, "UTF-8", 5, false)), Some((11, 5)));
    assert_eq!(run(&file, &text("HeLLo", false, "UTF-8", 23, true)), Some((18, 5)));
    assert_eq!(run(&file, &text("HeLLo", true, "UTF-8", 0, false)), None);
}

#[test]
fn a_letter_whose_case_is_two_letters_matches_only_itself() {
    let (_folder, file) = file_of("STRASSE straße".as_bytes());

    assert_eq!(run(&file, &text("straße", false, "UTF-8", 0, false)), Some((8, 7)));
    assert_eq!(run(&file, &text("STRASSE", false, "UTF-8", 0, false)), Some((0, 7)));
}

#[test]
fn utf16_needles_are_spelled_by_hand_and_found_on_character_boundaries() {
    for (big, label) in [(false, "UTF-16LE"), (true, "UTF-16BE")] {
        let (_folder, file) = file_of(&utf16("xx ab yy Łódź 😀!", big));

        assert_eq!(run(&file, &text("ab", true, label, 0, false)), Some((6, 4)), "{label}");
        assert_eq!(
            run(&file, &text("łÓDŹ", false, label, 0, false)),
            Some((18, 8)),
            "{label}"
        );
        assert_eq!(run(&file, &text("😀", true, label, 0, false)), Some((28, 4)), "{label}");
        assert_eq!(run(&file, &text("ab", true, label, 0, true)), None, "{label}");
        assert_eq!(
            run(&file, &text("yy", true, label, 100, true)),
            Some((12, 4)),
            "{label}"
        );
    }
}

#[test]
fn a_utf16_match_at_an_odd_offset_is_no_match() {
    // `61 00 62 00` is "ab" in UTF-16LE, but here it starts at byte 1.
    let (_folder, file) = file_of(&[0x00, 0x61, 0x00, 0x62, 0x00, 0x20]);

    assert_eq!(run(&file, &text("ab", true, "UTF-16LE", 0, false)), None);
    assert_eq!(run(&file, &text("ab", true, "UTF-16LE", 6, true)), None);
    assert_eq!(run(&file, &text("ab", false, "UTF-16LE", 0, false)), None);
    assert_eq!(run(&file, &hex(&[0x61, 0x00, 0x62, 0x00], 0, false)), Some((1, 4)));
}

#[test]
fn hex_needles_are_bytes() {
    let (_folder, file) = file_of(&[0, 1, 0xDE, 0xAD, 0xBE, 0xEF, 2, 0xDE, 0xAD, 3]);

    assert_eq!(run(&file, &hex(&[0xDE, 0xAD], 0, false)), Some((2, 2)));
    assert_eq!(run(&file, &hex(&[0xDE, 0xAD], 3, false)), Some((7, 2)));
    assert_eq!(run(&file, &hex(&[0xDE, 0xAD], 9, true)), Some((7, 2)));
    assert_eq!(run(&file, &hex(&[0xDE, 0xAD, 0xBE, 0xEF], 0, false)), Some((2, 4)));
    assert_eq!(run(&file, &hex(&[0xAD, 0xDE], 0, false)), None);
    assert_eq!(run(&file, &hex(&[0], 1, false)), None);
}

#[test]
fn invalid_needles_are_refused() {
    let (_folder, file) = file_of(b"abc");
    let empty = (ErrorKind::InvalidInput, "the search text is empty".to_owned());
    let long = (ErrorKind::InvalidInput, "the search text is too long".to_owned());

    assert_eq!(error_of(&file, &hex(&[], 0, false)), empty);
    assert_eq!(error_of(&file, &text("", true, "UTF-8", 0, false)), empty);
    assert_eq!(error_of(&file, &hex(&vec![1; MAX_NEEDLE + 1], 0, false)), long);
    assert_eq!(
        error_of(&file, &text(&"a".repeat(MAX_NEEDLE + 1), true, "UTF-8", 0, false)),
        long
    );
    assert_eq!(
        error_of(&file, &text("a", true, "klingon", 0, false)),
        (ErrorKind::InvalidInput, "the encoding is not known".to_owned())
    );

    // The longest allowed needle is fine.
    assert_eq!(run(&file, &hex(&vec![1; MAX_NEEDLE], 0, false)), None);
    assert_eq!(
        run(&file, &text(&"ą".repeat(MAX_NEEDLE), false, "UTF-8", 0, false)),
        None
    );
}

#[test]
fn a_needle_the_encoding_cannot_spell_finds_nothing() {
    let (_folder, file) = file_of(b"plain ascii l and L");

    assert_eq!(run(&file, &text("ł", true, "windows-1252", 0, false)), None);
    assert_eq!(run(&file, &text("lł", false, "windows-1252", 0, false)), None);
    assert_eq!(run(&file, &text("L", false, "windows-1252", 0, false)), Some((1, 1)));
    assert_eq!(run(&file, &text("L", false, "windows-1252", 3, false)), Some((12, 1)));
}

#[test]
fn a_cancelled_search_ends_with_an_error() {
    let (_folder, file) = file_of(b"abc abc");
    let cancel = AtomicBool::new(true);

    let error = search(&file, &text("abc", true, "UTF-8", 0, false), &cancel, |_| {}).expect_err("cancelled");

    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "the search was cancelled");
}

#[cfg(unix)]
#[test]
fn progress_is_reported_every_64_mib_and_a_flag_set_there_cancels() {
    use std::sync::atomic::Ordering;

    use super::open;
    use super::testing::temp;

    let folder = temp();
    let size: u64 = 150 * 1024 * 1024;
    let handle = std::fs::File::create(folder.path().join("big.bin")).expect("create");
    handle.set_len(size).expect("length");
    drop(handle);
    let file = open(folder.path(), "big.bin").expect("open").file;
    let step = 64 * 1024 * 1024;

    let mut reports = Vec::new();
    let found = search(&file, &hex(&[1, 2, 3], 0, false), &AtomicBool::new(false), |scanned| {
        reports.push(scanned)
    });
    assert_eq!(found.expect("done"), None);
    assert_eq!(reports, [step, 2 * step]);

    let cancel = AtomicBool::new(false);
    let result = search(&file, &hex(&[1, 2, 3], 0, false), &cancel, |_| {
        cancel.store(true, Ordering::Relaxed)
    });
    assert_eq!(result.expect_err("cancelled").kind(), ErrorKind::Interrupted);

    let mut backward = Vec::new();
    search(
        &file,
        &hex(&[1, 2, 3], size, true),
        &AtomicBool::new(false),
        |scanned| backward.push(scanned),
    )
    .expect("done");
    assert_eq!(backward, [step, 2 * step]);
}

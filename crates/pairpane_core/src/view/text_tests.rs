//! Windows of text: forward and backward, mid-line snapping, line endings, encodings and long lines.

use pretty_assertions::assert_eq;

use crate::model::TextWindow;

use super::testing::{file_of, opened, query, texts, window};
use super::{MAX_SEGMENT, MAX_TEXT_LINES, MAX_TEXT_SCAN, text_window};

fn utf16(text: &str, big: bool) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|unit| if big { unit.to_be_bytes() } else { unit.to_le_bytes() })
        .collect()
}

/// Walks a file forward window by window and returns every line.
fn all_forward(file: &super::ViewFile, encoding: &str, lines: u32) -> Vec<String> {
    let mut all = Vec::new();
    let mut offset = 0;

    for _ in 0..10_000 {
        let next = text_window(file, &query(encoding, offset, false, lines, false)).expect("window");
        all.extend(next.lines.iter().map(|line| line.text.clone()));

        if next.at_end {
            return all;
        }

        assert!(next.end > offset, "the window must move on");
        offset = next.end;
    }

    panic!("never reached the end");
}

#[test]
fn forward_windows_follow_each_other() {
    let (_folder, file) = file_of(b"one\ntwo\nthree\n");

    let first = window(&file, 0, false, 2);
    assert_eq!(texts(&first), ["one", "two"]);
    assert_eq!(
        (first.start, first.end, first.at_start, first.at_end),
        (0, 8, true, false)
    );
    assert_eq!(first.lines[1].offset, 4);

    let second = window(&file, first.end, false, 2);
    assert_eq!(texts(&second), ["three"]);
    assert_eq!(
        (second.start, second.end, second.at_start, second.at_end),
        (8, 14, false, true)
    );
}

#[test]
fn a_file_without_a_final_newline_ends_with_its_last_line() {
    let (_folder, file) = file_of(b"a\nb");

    let all = window(&file, 0, false, 10);

    assert_eq!(texts(&all), ["a", "b"]);
    assert_eq!((all.end, all.at_end), (3, true));
}

#[test]
fn a_file_that_ends_with_a_newline_has_no_empty_last_line() {
    let (_folder, file) = file_of(b"a\n\nb\n\n");

    assert_eq!(texts(&window(&file, 0, false, 10)), ["a", "", "b", ""]);
}

#[test]
fn an_empty_file_is_an_empty_window() {
    let (_folder, file) = file_of(b"");

    for backward in [false, true] {
        let empty = window(&file, 0, backward, 10);

        assert_eq!(
            empty,
            TextWindow {
                start: 0,
                end: 0,
                lines: vec![],
                at_start: true,
                at_end: true
            }
        );
    }
}

#[test]
fn carriage_returns_before_line_feeds_are_dropped() {
    let (_folder, file) = file_of(b"a\r\nb\r\r\nc\r\n");

    assert_eq!(texts(&window(&file, 0, false, 10)), ["a", "b\r", "c"]);
}

#[test]
fn a_scroll_bar_offset_in_the_middle_of_a_line_snaps_to_its_start() {
    let (_folder, file) = file_of(b"hello\nworld\nagain\n");

    let snapped = text_window(&file, &query("UTF-8", 8, false, 1, true)).expect("window");
    assert_eq!(texts(&snapped), ["world"]);
    assert_eq!(snapped.start, 6);

    let at_start = text_window(&file, &query("UTF-8", 6, false, 1, true)).expect("window");
    assert_eq!(at_start.start, 6);

    let kept = text_window(&file, &query("UTF-8", 8, false, 1, false)).expect("window");
    assert_eq!(texts(&kept), ["rld"]);
    assert_eq!(kept.start, 8);
}

#[test]
fn an_offset_in_the_middle_of_a_utf8_character_moves_on_to_the_next_one() {
    let (_folder, file) = file_of("zażółć\nżółw\n".as_bytes());

    // `ż` is two bytes at 2..4; offset 3 is inside it.
    let moved = window(&file, 3, false, 1);

    assert_eq!(moved.start, 4);
    assert_eq!(texts(&moved), ["ółć"]);
}

#[test]
fn backward_windows_end_where_asked() {
    let (_folder, file) = file_of(b"one\ntwo\nthree\n");

    let last = window(&file, 14, true, 2);
    assert_eq!(texts(&last), ["two", "three"]);
    assert_eq!((last.start, last.end, last.at_start, last.at_end), (4, 14, false, true));

    let before = window(&file, last.start, true, 5);
    assert_eq!(texts(&before), ["one"]);
    assert_eq!(
        (before.start, before.end, before.at_start, before.at_end),
        (0, 4, true, false)
    );

    let nothing = window(&file, 0, true, 5);
    assert!(nothing.lines.is_empty());
    assert!(nothing.at_start);
}

#[test]
fn backward_from_the_end_of_a_file_without_a_final_newline() {
    let (_folder, file) = file_of(b"a\nb\nlast");

    let window = window(&file, 8, true, 2);

    assert_eq!(texts(&window), ["b", "last"]);
    assert_eq!((window.start, window.end), (2, 8));
}

#[test]
fn an_offset_past_the_end_is_the_end() {
    let (_folder, file) = file_of(b"one\ntwo\n");

    let forward = window(&file, u64::MAX, false, 5);
    assert!(forward.lines.is_empty());
    assert_eq!((forward.start, forward.end, forward.at_end), (8, 8, true));

    assert_eq!(texts(&window(&file, u64::MAX, true, 1)), ["two"]);
}

#[test]
fn the_number_of_lines_is_clamped() {
    let text: String = (0..1500).map(|number| format!("line {number}\n")).collect();
    let (_folder, file) = file_of(text.as_bytes());

    assert_eq!(window(&file, 0, false, 0).lines.len(), 1);
    assert_eq!(window(&file, 0, false, u32::MAX).lines.len(), MAX_TEXT_LINES);
    assert_eq!(
        window(&file, text.len() as u64, true, u32::MAX).lines.len(),
        MAX_TEXT_LINES
    );
    assert_eq!(all_forward(&file, "UTF-8", 1000).len(), 1500);
}

#[test]
fn utf8_with_a_byte_order_mark_starts_after_it() {
    let (_folder, file) = file_of(b"\xEF\xBB\xBFhi\nthere\n");

    let first = window(&file, 0, false, 5);

    assert_eq!(texts(&first), ["hi", "there"]);
    assert_eq!((first.start, first.at_start), (3, true));
    assert_eq!(window(&file, 100, true, 1).lines[0].text, "there");
}

#[test]
fn the_same_bytes_read_as_another_encoding_keep_the_mark_as_bytes() {
    let (_folder, file) = file_of(b"\xEF\xBB\xBFhi\n");

    let latin = text_window(&file, &query("windows-1252", 0, false, 5, false)).expect("window");

    assert_eq!(texts(&latin), ["\u{ef}\u{bb}\u{bf}hi"]);
}

#[test]
fn windows_1250_is_decoded() {
    let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode("Zażółć gęślą jaźń\nŁódź\n");
    let (_folder, file) = file_of(&bytes);

    let window = text_window(&file, &query("windows-1250", 0, false, 5, false)).expect("window");

    assert_eq!(texts(&window), ["Zażółć gęślą jaźń", "Łódź"]);
    assert_eq!(window.end, bytes.len() as u64);
}

#[test]
fn invalid_bytes_show_as_replacement_characters() {
    let (_folder, file) = file_of(b"ok\nbad \xFF\xFE here\n");

    assert_eq!(texts(&window(&file, 0, false, 5)), ["ok", "bad \u{fffd}\u{fffd} here"]);
}

#[test]
fn an_unknown_encoding_is_refused() {
    let (_folder, file) = file_of(b"x");

    let error = text_window(&file, &query("klingon", 0, false, 5, false)).expect_err("unknown");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "the encoding is not known");
}

#[test]
fn utf16_little_endian_with_a_mark() {
    let bytes = [&[0xFF, 0xFE][..], &utf16("ab\r\ncd\nżółw", false)].concat();
    let (_folder, file) = file_of(&bytes);

    let forward = text_window(&file, &query("UTF-16LE", 0, false, 10, false)).expect("window");
    assert_eq!(texts(&forward), ["ab", "cd", "żółw"]);
    assert_eq!((forward.start, forward.at_start, forward.at_end), (2, true, true));

    let backward = text_window(&file, &query("UTF-16LE", u64::MAX, true, 2, false)).expect("window");
    assert_eq!(texts(&backward), ["cd", "żółw"]);
}

#[test]
fn utf16_without_a_mark_is_found_and_read_in_both_orders() {
    let text = "Hello, wörld! This is some text.\nSecond line.\nThird line\n";

    for (big, label) in [(false, "UTF-16LE"), (true, "UTF-16BE")] {
        let (_folder, opened) = opened("wide.txt", &utf16(text, big));
        assert_eq!(opened.encoding, label);

        let forward = text_window(&opened.file, &query(label, 0, false, 10, false)).expect("window");
        assert_eq!(
            texts(&forward),
            ["Hello, wörld! This is some text.", "Second line.", "Third line"]
        );

        let backward = text_window(&opened.file, &query(label, u64::MAX, true, 2, false)).expect("window");
        assert_eq!(texts(&backward), ["Second line.", "Third line"]);

        // An odd offset moves to a character boundary.
        let odd = text_window(&opened.file, &query(label, 5, false, 1, false)).expect("window");
        assert_eq!(odd.start, 4);
    }
}

#[test]
fn utf16_snaps_to_the_start_of_a_line() {
    let bytes = utf16("first\nsecond\nthird\n", false);
    let (_folder, file) = file_of(&bytes);

    // Byte 16 is inside "second" (which starts at 12).
    let snapped = text_window(&file, &query("UTF-16LE", 16, false, 1, true)).expect("window");

    assert_eq!(texts(&snapped), ["second"]);
    assert_eq!(snapped.start, 12);
}

#[test]
fn a_line_of_a_hundred_kilobytes_comes_back_in_pieces() {
    let long = "x".repeat(100_000);
    let (_folder, file) = file_of(format!("{long}\nafter\n").as_bytes());

    let all = window(&file, 0, false, 100);

    let pieces: Vec<_> = all
        .lines
        .iter()
        .map(|line| (line.offset, line.text.len(), line.continues))
        .collect();
    let step = MAX_SEGMENT as u64;
    let expected: Vec<_> = (0..6u64)
        .map(|index| (index * step, MAX_SEGMENT, true))
        .chain([(6 * step, 100_000 - 6 * MAX_SEGMENT, false), (100_001, 5, false)])
        .collect();
    assert_eq!(pieces, expected);

    let joined: String = all.lines[..7].iter().map(|line| line.text.as_str()).collect();
    assert_eq!(joined, long);
}

#[test]
fn a_window_can_end_inside_a_long_line_and_the_next_one_goes_on() {
    let long = "y".repeat(100_000);
    let (_folder, file) = file_of(format!("{long}\nafter\n").as_bytes());

    let first = window(&file, 0, false, 3);
    assert_eq!(first.lines.len(), 3);
    assert_eq!(first.end, 3 * MAX_SEGMENT as u64);
    assert!(first.lines.iter().all(|line| line.continues));

    let second = window(&file, first.end, false, 100);
    assert_eq!(second.lines[0].offset, first.end);
    assert_eq!(all_forward(&file, "UTF-8", 3).concat().len(), 100_000 + "after".len());
}

#[test]
fn a_long_line_is_cut_between_characters() {
    let text = "é".repeat(10_000);
    let (_folder, file) = file_of(text.as_bytes());

    let all = window(&file, 0, false, 100);

    assert!(all.lines.len() >= 2);
    let joined: String = all.lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(joined, text);
    assert!(all.lines.iter().all(|line| !line.text.contains('\u{fffd}')));
}

#[test]
fn a_long_run_of_invalid_utf8_bytes_is_cut_into_pieces_and_ends() {
    let mut bytes = vec![0x80; 40_000];
    bytes.extend_from_slice(b"\nafter\n");
    let (_folder, file) = file_of(&bytes);

    let all = window(&file, 0, false, 100);

    // The window may start a few bytes in (an offset is moved off what looks like the middle of a character).
    let (last, run) = all.lines.split_last().expect("lines");
    assert_eq!(last.text, "after");
    assert_eq!(last.offset, 40_001);
    assert_eq!(run.len(), 3);
    assert!(run[..2].iter().all(|line| line.continues));
    assert!(
        run.windows(2)
            .all(|pair| pair[1].offset - pair[0].offset == MAX_SEGMENT as u64)
    );
}

#[test]
fn a_long_line_of_utf16_is_cut_between_characters_and_pairs() {
    let text = "a😀".repeat(9_000);
    let (_folder, file) = file_of(&[&[0xFF, 0xFE][..], &utf16(&text, false)].concat());

    let all = text_window(&file, &query("UTF-16LE", 0, false, 100, false)).expect("window");

    let joined: String = all.lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(joined, text);
}

#[test]
fn a_window_reads_at_most_a_megabyte() {
    // Two megabytes without a line ending: one window of 64 pieces, then the rest.
    let (_folder, file) = file_of(&vec![b'z'; 2 * MAX_TEXT_SCAN]);

    let first = window(&file, 0, false, 1000);

    assert_eq!(first.end, MAX_TEXT_SCAN as u64);
    assert_eq!(first.lines.len(), MAX_TEXT_SCAN / MAX_SEGMENT);
    assert!(first.lines.iter().all(|line| line.continues));
    assert!(!first.at_end);

    let last = window(&file, 2 * MAX_TEXT_SCAN as u64, true, 1000);
    assert_eq!(last.lines.len(), MAX_TEXT_SCAN / MAX_SEGMENT);
    assert_eq!(last.start, MAX_TEXT_SCAN as u64);
}

#[test]
fn a_backward_window_in_a_long_line_ends_at_the_offset() {
    let (_folder, file) = file_of(&vec![b'q'; 50_000]);

    let before = window(&file, 40_000, true, 2);

    assert_eq!(before.end, 40_000);
    assert_eq!(before.lines.len(), 2);
    assert_eq!(before.lines.last().map(|line| line.continues), Some(true));
    assert_eq!(before.lines[1].offset + before.lines[1].text.len() as u64, 40_000);
}

#[test]
fn garbage_never_panics_and_windows_stay_inside_the_file() {
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    let bytes: Vec<u8> = (0..30_000)
        .map(|_| match next() % 6 {
            0 => b'\n',
            1 => 0,
            2 => b'\r',
            _ => (next() % 256) as u8,
        })
        .collect();
    let (_folder, file) = file_of(&bytes);
    let size = bytes.len() as u64;

    for label in [
        "UTF-8",
        "UTF-16LE",
        "UTF-16BE",
        "windows-1250",
        "Shift_JIS",
        "GBK",
        "Big5",
        "EUC-KR",
        "ISO-2022-JP",
    ] {
        for _ in 0..40 {
            let offset = next() % (size + 50);
            let (backward, snap) = (next() % 2 == 0, next() % 2 == 0);

            let found =
                text_window(&file, &query(label, offset, backward, (next() % 50) as u32, snap)).expect("window");

            assert!(
                found.start <= found.end && found.end <= size,
                "{label} {offset} {found:?}"
            );
            let offsets: Vec<u64> = found.lines.iter().map(|line| line.offset).collect();
            assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]), "{label}");
            assert!(
                offsets
                    .iter()
                    .all(|at| *at >= found.start && *at < found.end.max(found.start + 1)),
                "{label}"
            );
        }
    }
}

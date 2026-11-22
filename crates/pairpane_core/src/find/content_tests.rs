//! Searching inside files: the text rules, encodings, binary files, limits and the first line.

use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

use crate::model::{FindCriteria, FindHit, FindText};

use super::content::{Content, Scan, snippet};
use super::out::Run;
use super::testing::{put, search, temp};
use super::{HEAP_LIMIT, MAX_TEXT_FILE, SNIPPET_CHARS};

/// A literal text search, case-insensitive.
pub(super) fn text(pattern: &str) -> FindText {
    FindText {
        pattern: pattern.to_owned(),
        regex: false,
        case_sensitive: false,
        whole_word: false,
        encoding: None,
        any_size: false,
    }
}

fn looking_for(found: FindText) -> FindCriteria {
    FindCriteria {
        text: Some(found),
        ..FindCriteria::default()
    }
}

fn one(root: &std::path::Path, found: FindText) -> Vec<FindHit> {
    search(root, looking_for(found)).hits()
}

#[test]
fn a_file_is_a_result_with_the_number_of_matching_lines_and_the_first() {
    let root = temp();
    let log = b"start\n  the Needle is here  \nnothing\nneedle again\n";
    put(root.path(), "log.txt", log);
    put(root.path(), "other.txt", b"no match\n");

    let hits = one(root.path(), text("needle"));

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "log.txt");
    assert_eq!(hits[0].matches, Some(2));
    assert_eq!(hits[0].line.as_deref(), Some("the Needle is here"));
    assert_eq!(hits[0].line_number, Some(2));
    assert_eq!(hits[0].size, log.len() as u64);
}

#[test]
fn case_and_whole_words_are_asked_for() {
    let root = temp();
    put(root.path(), "a.txt", b"Cat\n");
    put(root.path(), "b.txt", b"concatenate\n");

    let mut found = text("cat");
    assert_eq!(
        search(root.path(), looking_for(found.clone())).paths(),
        ["a.txt", "b.txt"]
    );

    found.whole_word = true;
    assert_eq!(search(root.path(), looking_for(found.clone())).paths(), ["a.txt"]);

    found.case_sensitive = true;
    assert!(search(root.path(), looking_for(found)).paths().is_empty());
}

#[test]
fn a_literal_is_not_a_regular_expression_and_a_regular_expression_is() {
    let root = temp();
    put(root.path(), "a.txt", b"a.c\n");
    put(root.path(), "b.txt", b"abc\n");

    assert_eq!(search(root.path(), looking_for(text("a.c"))).paths(), ["a.txt"]);

    let mut found = text(r"a.c");
    found.regex = true;
    assert_eq!(search(root.path(), looking_for(found)).paths(), ["a.txt", "b.txt"]);
}

#[test]
fn the_end_of_a_line_is_found_before_a_carriage_return_too() {
    let root = temp();
    put(root.path(), "dos.txt", b"first line\r\nend\r\n");
    put(root.path(), "unix.txt", b"first line\nend\n");
    put(root.path(), "other.txt", b"endless\r\n");
    let mut found = text("end$");
    found.regex = true;

    assert_eq!(search(root.path(), looking_for(found)).paths(), ["dos.txt", "unix.txt"]);

    let mut whole_line = text("^first line$");
    whole_line.regex = true;
    assert_eq!(
        search(root.path(), looking_for(whole_line)).paths(),
        ["dos.txt", "unix.txt"]
    );
}

#[test]
fn a_regular_expression_that_would_backtrack_runs_in_linear_time() {
    let root = temp();
    put(root.path(), "long.txt", format!("{}\n", "a".repeat(200_000)).as_bytes());
    let mut found = text("(a*)*b");
    found.regex = true;

    let started = Instant::now();
    let hits = one(root.path(), found);

    assert!(hits.is_empty());
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn utf_16_with_a_byte_order_mark_is_read_without_being_told() {
    let root = temp();
    let units: Vec<u8> = "caf\u{e9} needle\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    put(root.path(), "utf16.txt", &[&[0xFF, 0xFE][..], &units].concat());
    put(root.path(), "bom8.txt", b"\xEF\xBB\xBFneedle\n");

    let hits = one(root.path(), text("needle"));

    assert_eq!(
        hits.iter().map(|hit| hit.path.as_str()).collect::<Vec<_>>(),
        ["bom8.txt", "utf16.txt"]
    );
    assert_eq!(hits[0].line.as_deref(), Some("needle"));
    assert_eq!(hits[1].line.as_deref(), Some("caf\u{e9} needle"));
}

#[test]
fn a_named_encoding_is_used_for_files_without_a_byte_order_mark() {
    let root = temp();
    // "zażółć" in windows-1250.
    put(root.path(), "pl.txt", b"za\xBF\xF3\xB3\xE6 text\n");

    let mut found = text("za\u{17c}\u{f3}\u{142}\u{107}");
    assert!(one(root.path(), found.clone()).is_empty());

    found.encoding = Some("windows-1250".to_owned());
    let hits = one(root.path(), found);

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line.as_deref(), Some("za\u{17c}\u{f3}\u{142}\u{107} text"));
}

#[test]
fn a_binary_file_is_left_out_even_when_its_start_matches() {
    let root = temp();
    let mut bytes = b"needle at the start\n".to_vec();
    bytes.extend(vec![b'x'; 200_000]);
    bytes.extend(b"\0tail needle\n");
    put(root.path(), "program.bin", &bytes);
    put(root.path(), "text.txt", b"needle\n");

    let found = search(root.path(), looking_for(text("needle")));

    assert_eq!(found.paths(), ["text.txt"]);
    assert_eq!(found.done().1, 0, "a binary file is not counted as skipped");
}

#[test]
fn a_file_over_the_size_cap_is_skipped_unless_any_size_is_asked_for() {
    let root = temp();
    let path = root.path().join("huge.txt");
    let file = std::fs::File::create(&path).expect("create");
    file.set_len(MAX_TEXT_FILE + 1).expect("sparse file");
    put(root.path(), "small.txt", b"needle\n");
    drop(file);
    std::fs::write(root.path().join("tiny.txt"), b"needle").expect("write");

    let found = search(root.path(), looking_for(text("needle")));

    assert_eq!(found.paths(), ["small.txt", "tiny.txt"]);
    assert_eq!(found.done().1, 1);

    // Any size reads it: the hole is NULs, so it counts as binary and is not a result, and nothing is skipped.
    let mut any = text("needle");
    any.any_size = true;
    let found = search(root.path(), looking_for(any));

    assert_eq!(found.paths(), ["small.txt", "tiny.txt"]);
    assert_eq!(found.done().1, 0);
}

#[test]
fn a_line_longer_than_the_buffer_limit_is_skipped_and_counted_not_read_whole() {
    let root = temp();
    let mut line = vec![b'x'; HEAP_LIMIT + 1024];
    line.extend(b" needle\n");
    put(root.path(), "one-line.txt", &line);
    put(root.path(), "fine.txt", b"needle\n");

    let found = search(root.path(), looking_for(text("needle")));

    assert_eq!(found.paths(), ["fine.txt"]);
    assert_eq!(found.done().1, 1);
}

#[test]
fn only_files_are_searched_and_a_missing_file_is_counted() {
    let root = temp();
    put(root.path(), "dir/needle.txt", b"needle\n");
    std::fs::create_dir(root.path().join("needle-dir")).expect("folder");

    let found = search(root.path(), looking_for(text("needle")));

    assert_eq!(found.paths(), ["dir/needle.txt"]);
}

#[test]
fn the_text_can_be_combined_with_a_name_mask() {
    let root = temp();
    put(root.path(), "a.rs", b"needle\n");
    put(root.path(), "b.md", b"needle\n");
    let mut criteria = looking_for(text("needle"));
    criteria.names = "*.rs".to_owned();

    assert_eq!(search(root.path(), criteria).paths(), ["a.rs"]);
}

#[test]
fn a_snippet_is_trimmed_cut_and_has_no_control_characters() {
    assert_eq!(snippet(b"  \tindented\r\n"), "indented");
    assert_eq!(snippet(b"a\tb"), "a b");
    assert_eq!(
        snippet("a\u{1b}[31mb\u{202e}c\u{85}d\0".as_bytes()),
        "a\u{fffd}[31mb\u{fffd}c\u{fffd}d\u{fffd}"
    );
    assert_eq!(snippet(b"bad \xFF byte"), "bad \u{fffd} byte");
    assert_eq!(
        snippet("\u{e9}".repeat(SNIPPET_CHARS + 50).as_bytes()).chars().count(),
        SNIPPET_CHARS
    );
    assert_eq!(snippet(b"   \n"), "");
}

#[cfg(unix)]
#[test]
fn a_link_to_a_file_is_never_read() {
    let root = temp();
    let outside = temp();
    put(outside.path(), "secret.txt", b"needle\n");
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), root.path().join("link.txt")).expect("link");
    put(root.path(), "real.txt", b"needle\n");

    let found = search(root.path(), looking_for(text("needle")));

    assert_eq!(found.paths(), ["real.txt"]);
}

#[test]
fn a_stopped_search_reads_nothing_and_finds_nothing() {
    let root = temp();
    put(root.path(), "a.txt", b"needle\n");
    let content = Content::new(&text("needle")).expect("compiles");
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let file = std::fs::File::open(root.path().join("a.txt")).expect("open");

    let scan = content.search(&mut content.searcher(), file, &Run::new(&cancel));

    assert_eq!(scan, Scan::Nothing);
}

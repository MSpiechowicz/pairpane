//! The cases of `ui/src/lib/panes/mask.test.ts`, so the two implementations stay the same.

use std::time::{Duration, Instant};

use super::mask::{Mask, parts};

fn mask(text: &str) -> Mask {
    Mask::new(text, false)
}

#[test]
fn matches_star_and_question_wildcards() {
    let txt = mask("*.txt");
    assert!(txt.fits("notes.txt"));
    assert!(txt.fits(".txt"));
    assert!(!txt.fits("notes.txt.bak"));
    assert!(!txt.fits("txt"));

    assert!(mask("a?c").fits("abc"));
    assert!(!mask("a?c").fits("ac"));
    assert!(!mask("a?c").fits("abbc"));
    assert!(mask("a*b*c").fits("a-xx-b-yy-c"));
    assert!(!mask("a*b*c").fits("a-xx-c-yy-b"));
    assert!(mask("*b").fits("abab"));
}

#[test]
fn ignores_case_unless_asked_to_keep_it() {
    assert!(mask("*.JPG").fits("photo.jpg"));
    assert!(mask("read*").fits("README.md"));

    assert!(!Mask::new("*.JPG", true).fits("photo.jpg"));
    assert!(Mask::new("*.JPG", true).fits("photo.JPG"));
}

#[test]
fn joins_masks_with_semicolons_ignoring_spaces_and_empty_parts() {
    let list = mask(" *.png ;; *.JPG; ");

    assert!(list.fits("a.png"));
    assert!(list.fits("b.jpg"));
    assert!(!list.fits("c.gif"));
}

#[test]
fn star_dot_star_is_everything_with_a_dot_or_without() {
    let all = mask("*.*");

    assert!(all.fits("Makefile"));
    assert!(all.fits("a.b"));
    assert!(all.fits(".hidden"));
    assert!(mask("*.txt; *.*").fits("Makefile"));
}

#[test]
fn an_empty_mask_fits_nothing() {
    assert!(!mask("").fits("a"));
    assert!(!mask(" ; ").fits("a"));
    assert!(!mask("").fits(""));
}

#[test]
fn regular_expression_characters_are_literal() {
    assert!(mask("[(.").fits("[(."));
    assert!(!mask("[(.").fits("x"));
    assert!(mask("a+b").fits("a+b"));
    assert!(!mask("a+b").fits("aab"));
    assert!(!mask("a.c").fits("abc"));
    assert!(mask("(*)").fits("(x)"));
}

#[test]
fn a_pathological_mask_stays_fast() {
    let pathological = mask("*a*a*a*a*a*a*a*a*b");
    let name = "a".repeat(50_000);

    let started = Instant::now();
    assert!(!pathological.fits(&name));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn parts_are_the_trimmed_non_blank_masks() {
    assert_eq!(parts(" a ;; b;").collect::<Vec<_>>(), ["a", "b"]);
}

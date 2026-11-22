//! Search and replace over a name, and the change of case.

use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

use super::testing::rule;
use super::transform::{Search, apply_case, change_case};
use crate::model::{CaseChange, CasePart, RenameRule, RuleField};

/// More than any test name.
const LIMIT: usize = 1_024;

fn searching(search: &str, replace: &str, change: impl FnOnce(&mut RenameRule)) -> Search {
    let mut rule = rule();
    rule.search = search.to_owned();
    rule.replace = replace.to_owned();
    change(&mut rule);

    Search::new(&rule).expect("compiles").expect("a search")
}

#[test]
fn a_literal_search_finds_the_text_and_not_a_pattern() {
    let search = searching("a.b", "-", |_| {});

    assert_eq!(search.apply("a.b axb a.b", LIMIT), "- axb -");
}

#[test]
fn a_literal_replacement_is_not_expanded() {
    let search = searching("x", "$1 ${y} $$", |_| {});

    assert_eq!(search.apply("axa", LIMIT), "a$1 ${y} $$a");
}

#[test]
fn a_regular_expression_can_use_numbered_and_named_groups_in_the_replacement() {
    let numbered = searching(r"(\w+)-(\d+)", "${2}_$1", |rule| rule.regex = true);
    let named = searching(r"(?P<word>[a-z]+)", "${word}!", |rule| rule.regex = true);

    assert_eq!(numbered.apply("name-12.txt", LIMIT), "12_name.txt");
    assert_eq!(named.apply("ab 1", LIMIT), "ab! 1");
}

#[test]
fn the_first_match_only_or_every_one_is_replaced() {
    let all = searching("a", "b", |_| {});
    let first = searching("a", "b", |rule| rule.first_only = true);

    assert_eq!(all.apply("a.a.a", LIMIT), "b.b.b");
    assert_eq!(first.apply("a.a.a", LIMIT), "b.a.a");
}

#[test]
fn a_replacement_stops_once_the_result_is_over_the_limit() {
    // A pattern that matches the empty text at every position, with a long replacement.
    let every = searching("x?", &"z".repeat(100), |rule| rule.regex = true);
    let long = "a".repeat(500);

    let result = every.apply(&long, 300);

    assert!(result.len() > 300, "{}", result.len());
    assert!(result.len() <= 300 + 100 + 1, "{}", result.len());
    assert_eq!(every.apply("ab", LIMIT).len(), 3 * 100 + 2);
}

#[test]
fn the_text_after_the_last_match_is_kept_when_the_result_stays_within_the_limit() {
    let search = searching("a", "bb", |_| {});

    assert_eq!(search.apply("a-a-a", 100), "bb-bb-bb");
    assert_eq!(search.apply("a-a-a", 3), "bb-bb");
}

#[test]
fn the_case_of_the_search_matters_only_when_asked() {
    let exact = searching("abc", "-", |_| {});
    let loose = searching("abc", "-", |rule| rule.match_case = false);

    assert_eq!(exact.apply("ABC abc", LIMIT), "ABC -");
    assert_eq!(loose.apply("ABC abc", LIMIT), "- -");
}

#[test]
fn an_empty_search_does_nothing() {
    let mut rule = rule();
    rule.replace = "x".to_owned();

    assert!(Search::new(&rule).expect("compiles").is_none());
}

#[test]
fn a_regular_expression_that_is_not_valid_or_too_big_is_refused_at_once() {
    let started = Instant::now();

    for pattern in ["(", "a{1000}{1000}", "(((((a{100}){100}){100}){100}){100})"] {
        let mut rule = rule();
        rule.search = pattern.to_owned();
        rule.regex = true;

        let error = Search::new(&rule).err().expect("refused");

        assert_eq!(error.field, RuleField::Search, "{pattern}");
    }

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "no pattern may take long to refuse"
    );
}

#[test]
fn upper_and_lower_change_every_letter() {
    assert_eq!(change_case("Straße Élan", CaseChange::Upper), "STRASSE ÉLAN");
    assert_eq!(change_case("Straße ÉLAN", CaseChange::Lower), "straße élan");
    assert_eq!(change_case("As Is", CaseChange::Keep), "As Is");
}

#[test]
fn a_title_starts_each_word_in_uppercase_and_keeps_apostrophes_inside_it() {
    let titles = [
        ("don't stop", "Don't Stop"),
        ("it’s fine", "It’s Fine"),
        ("hello_world-foo.bar", "Hello_World-Foo.Bar"),
        ("ÉCOLE élève", "École Élève"),
        ("3rd place", "3rd Place"),
        ("'quoted' words", "'Quoted' Words"),
        ("ALL CAPS", "All Caps"),
        ("", ""),
    ];

    for (text, title) in titles {
        assert_eq!(change_case(text, CaseChange::Title), title, "{text}");
    }
}

#[test]
fn a_case_change_applies_to_the_name_the_extension_or_both() {
    let apply = |part| apply_case("My File.TXT", false, CaseChange::Lower, part);

    assert_eq!(apply(CasePart::Name), "my file.TXT");
    assert_eq!(apply(CasePart::Ext), "My File.txt");
    assert_eq!(apply(CasePart::Both), "my file.txt");
    assert_eq!(apply_case("a.b", false, CaseChange::Upper, CasePart::Name), "A.b");
    assert_eq!(
        apply_case("my file.txt", false, CaseChange::Title, CasePart::Both),
        "My File.Txt"
    );
}

#[test]
fn a_folder_and_a_dot_file_have_no_extension_to_change() {
    assert_eq!(
        apply_case("v1.beta", true, CaseChange::Upper, CasePart::Name),
        "V1.BETA"
    );
    assert_eq!(apply_case("v1.beta", true, CaseChange::Upper, CasePart::Ext), "v1.beta");
    assert_eq!(apply_case(".rc", false, CaseChange::Upper, CasePart::Ext), ".rc");
}

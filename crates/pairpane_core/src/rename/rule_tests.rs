//! A rule as a whole: what [`compile`] refuses, and the new name it makes (templates, dot, search and replace, case).

use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

use super::testing::{rule, templated};
use super::*;
use crate::find::MAX_PATTERN_BYTES;

fn name_of(rule: &RenameRule, name: &str) -> String {
    renamed(rule, name, 0)
}

fn renamed(rule: &RenameRule, name: &str, index: usize) -> String {
    let item = Item {
        name,
        folder: false,
        index,
        modified: Some(1_700_000_000_000),
    };

    compile(rule).expect("compiles").new_name(&item).expect("a name")
}

fn field_of(rule: &RenameRule) -> RuleField {
    compile(rule).err().expect("refused").field
}

#[test]
fn a_rule_that_changes_nothing_gives_the_name_back() {
    assert_eq!(name_of(&rule(), "report.final.txt"), "report.final.txt");
    assert_eq!(name_of(&rule(), "noext"), "noext");
    assert_eq!(name_of(&rule(), ".bashrc"), ".bashrc");
}

#[test]
fn the_name_and_the_extension_are_joined_by_a_dot_only_when_there_is_an_extension() {
    assert_eq!(name_of(&templated("[N]-new", "[E]"), "a.txt"), "a-new.txt");
    assert_eq!(name_of(&templated("[N]", "bak"), "a.txt"), "a.bak");
    assert_eq!(name_of(&templated("[N]", ""), "a.txt"), "a");
    assert_eq!(name_of(&templated("[N]", "[E]"), "noext"), "noext");
    assert_eq!(name_of(&templated("[N]", "old"), "noext"), "noext.old");
    assert_eq!(name_of(&templated("", "[E]"), "a.txt"), ".txt");
    assert_eq!(name_of(&templated("[E]", "[N]"), "a.txt"), "txt.a");
}

#[test]
fn the_counter_counts_by_the_position_in_the_list_and_is_padded() {
    let mut counting = templated("[N]-[C]", "[E]");
    counting.counter = RenameCounter {
        start: 5,
        step: 3,
        digits: 3,
    };

    assert_eq!(renamed(&counting, "a.txt", 0), "a-005.txt");
    assert_eq!(renamed(&counting, "a.txt", 2), "a-011.txt");

    counting.counter.digits = 1;
    assert_eq!(
        renamed(&counting, "a.txt", 100_000),
        "a-300005.txt",
        "a wider number is not cut"
    );
}

#[test]
fn the_search_and_replace_works_on_the_whole_new_name() {
    let mut replacing = templated("[N]", "[E]");
    replacing.search = ".txt".to_owned();
    replacing.replace = ".md".to_owned();
    assert_eq!(name_of(&replacing, "a.txt"), "a.md");

    replacing.search = r"^(\w+)\.(\w+)$".to_owned();
    replacing.replace = "$2.$1".to_owned();
    replacing.regex = true;
    assert_eq!(name_of(&replacing, "a.txt"), "txt.a");
}

#[test]
fn the_case_change_comes_after_the_search_and_replace() {
    let mut changing = rule();
    changing.search = "Photo".to_owned();
    changing.replace = "picture".to_owned();
    changing.case = CaseChange::Upper;
    changing.case_part = CasePart::Name;

    assert_eq!(name_of(&changing, "Photo 1.jpg"), "PICTURE 1.jpg");
}

#[test]
fn a_date_placeholder_needs_the_time_of_the_item() {
    let dated = compile(&templated("[YMD]-[N]", "[E]")).expect("compiles");
    let item = |modified| Item {
        name: "a.txt",
        folder: false,
        index: 0,
        modified,
    };

    assert_eq!(dated.new_name(&item(None)), Err(RenameProblem::NoDate));
    assert_eq!(dated.new_name(&item(Some(i64::MAX))), Err(RenameProblem::NoDate));

    let name = dated.new_name(&item(Some(1_700_000_000_000))).expect("a name");
    let (date, rest) = name.split_at(10);
    assert_eq!(rest, "-a.txt");
    assert!(date.chars().all(|char| char.is_ascii_digit() || char == '-'), "{date}");

    let plain = compile(&rule()).expect("compiles");
    assert_eq!(plain.new_name(&item(None)), Ok("a.txt".to_owned()), "no date is needed");
}

#[test]
fn a_new_name_over_a_kilobyte_is_cut_and_still_too_long() {
    let mut exploding = rule();
    exploding.search = ".".to_owned();
    exploding.replace = "x".repeat(MAX_PATTERN_BYTES);
    exploding.regex = true;

    let name = name_of(&exploding, &"n".repeat(200));

    assert_eq!(name.len(), MAX_NAME_BYTES);
    assert!(!crate::ops::name_fits(&name));
}

#[test]
fn a_cut_never_splits_a_character() {
    let mut exploding = rule();
    exploding.search = ".".to_owned();
    exploding.replace = "ł".repeat(500);
    exploding.regex = true;

    let name = name_of(&exploding, "abc");

    assert!(name.len() <= MAX_NAME_BYTES);
    assert!(name.chars().all(|char| char == 'ł'));
}

#[test]
fn a_placeholder_that_does_not_exist_names_its_field() {
    assert_eq!(field_of(&templated("[Q]", "[E]")), RuleField::Name);
    assert_eq!(field_of(&templated("[N]", "[N")), RuleField::Ext);

    let error = compile(&templated("[Q]", "[E]")).err().expect("refused");
    assert_eq!(error.message, "[Q] is not a placeholder");
}

#[test]
fn a_counter_out_of_range_is_refused() {
    let counting = |start, step, digits| RenameRule {
        counter: RenameCounter { start, step, digits },
        ..rule()
    };

    for bad in [
        counting(MAX_COUNTER_START + 1, 1, 1),
        counting(0, 0, 1),
        counting(0, MAX_COUNTER_STEP + 1, 1),
        counting(0, 1, 0),
        counting(0, 1, MAX_COUNTER_DIGITS + 1),
    ] {
        assert_eq!(field_of(&bad), RuleField::Counter);
    }

    assert!(compile(&counting(0, 1, 1)).is_ok());
    assert!(compile(&counting(MAX_COUNTER_START, MAX_COUNTER_STEP, MAX_COUNTER_DIGITS)).is_ok());
}

#[test]
fn a_field_over_a_kilobyte_is_refused_and_one_of_a_kilobyte_is_not() {
    let long = "a".repeat(MAX_PATTERN_BYTES + 1);
    let fits = "a".repeat(MAX_PATTERN_BYTES);

    let mut name = rule();
    name.name = long.clone();
    let mut ext = rule();
    ext.ext = long.clone();
    let mut search = rule();
    search.search = long.clone();
    let mut replace = rule();
    replace.replace = long;

    assert_eq!(field_of(&name), RuleField::Name);
    assert_eq!(field_of(&ext), RuleField::Ext);
    assert_eq!(field_of(&search), RuleField::Search);
    assert_eq!(field_of(&replace), RuleField::Replace);

    let mut exactly = rule();
    exactly.name = fits.clone();
    exactly.search = fits;
    assert!(compile(&exactly).is_ok());
}

#[test]
fn a_search_that_is_not_valid_or_too_complex_is_a_field_error_and_not_a_hang() {
    let started = Instant::now();

    for pattern in ["(", "[a-", "a{1000}{1000}"] {
        let mut searching = rule();
        searching.search = pattern.to_owned();
        searching.regex = true;

        assert_eq!(field_of(&searching), RuleField::Search, "{pattern}");
    }

    // The same text is only a literal when the rule says so.
    let mut literal = rule();
    literal.search = "a{1000}{1000}(".to_owned();
    assert!(compile(&literal).is_ok());

    assert!(started.elapsed() < Duration::from_secs(10));
}

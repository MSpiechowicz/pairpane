//! Saved searches in the state file: what loads, what is kept, what is dropped, and what a save without them keeps.

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::json;

use super::tests::{folder, saved_json, state, written};
use super::*;
use crate::model::{FindCriteria, FindText, SavedSearch, Tri};

fn search(name: &str) -> SavedSearch {
    SavedSearch {
        name: name.to_owned(),
        criteria: FindCriteria::default(),
    }
}

fn with_criteria(name: &str, criteria: FindCriteria) -> SavedSearch {
    SavedSearch {
        name: name.to_owned(),
        criteria,
    }
}

fn names(searches: &[SavedSearch]) -> Vec<&str> {
    searches.iter().map(|search| search.name.as_str()).collect()
}

fn with_searches(searches: Vec<SavedSearch>) -> SavedState {
    SavedState { searches, ..state() }
}

#[test]
fn a_file_from_before_saved_searches_loads_with_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut value = saved_json(&file);
    value.as_object_mut().expect("object").remove("searches");
    written(&file, &serde_json::to_vec(&value).expect("json"));

    let loaded = load(&file).expect("state");

    assert_eq!(loaded.searches, Vec::new());
    assert_eq!(loaded, state());
}

#[test]
fn saved_searches_survive_a_round_trip_through_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let criteria = FindCriteria {
        names: "*.rs;*.toml".to_owned(),
        text: Some(FindText {
            pattern: "todo".to_owned(),
            regex: false,
            case_sensitive: false,
            whole_word: true,
            encoding: Some("windows-1250".to_owned()),
            any_size: false,
        }),
        min_size: Some(10),
        ..FindCriteria::default()
    };
    let saved = with_searches(vec![search("everything"), with_criteria("todos", criteria)]);

    save(&file, saved.clone()).expect("saved");

    assert_eq!(load(&file), Some(saved));
}

#[test]
fn at_most_fifty_searches_are_kept_the_first_ones() {
    let many: Vec<SavedSearch> = (0..MAX_SEARCHES + 7).map(|n| search(&format!("s{n}"))).collect();

    let cleaned = clean(with_searches(many)).searches;

    assert_eq!(cleaned.len(), MAX_SEARCHES);
    assert_eq!(cleaned[0].name, "s0");
    assert_eq!(cleaned[MAX_SEARCHES - 1].name, format!("s{}", MAX_SEARCHES - 1));
}

#[test]
fn dropped_searches_do_not_count_against_the_cap() {
    let mut many = vec![search(" "); 10];
    many.extend((0..MAX_SEARCHES).map(|n| search(&format!("s{n}"))));

    assert_eq!(clean(with_searches(many)).searches.len(), MAX_SEARCHES);
}

#[test]
fn searches_with_unusable_names_are_dropped() {
    let longest = "n".repeat(MAX_SEARCH_NAME_CHARS);
    let too_long = "n".repeat(MAX_SEARCH_NAME_CHARS + 1);
    // 64 characters of two bytes each: the limit counts characters.
    let wide = "ł".repeat(MAX_SEARCH_NAME_CHARS);

    let cleaned = clean(with_searches(vec![
        search(""),
        search(" \t\n"),
        search(&too_long),
        search(&longest),
        search(&wide),
        search("  padded  "),
    ]))
    .searches;

    assert_eq!(names(&cleaned), vec![longest.as_str(), wide.as_str(), "padded"]);
}

#[test]
fn a_repeated_name_keeps_the_first_search_whatever_the_case() {
    let first = with_criteria(
        "Sources",
        FindCriteria {
            names: "*.rs".to_owned(),
            ..FindCriteria::default()
        },
    );

    let cleaned = clean(with_searches(vec![
        first.clone(),
        search("sources"),
        search("SOURCES "),
        search("other"),
    ]))
    .searches;

    assert_eq!(cleaned, vec![first, search("other")]);
}

#[test]
fn a_dropped_search_does_not_take_its_name() {
    let broken = with_criteria(
        "mine",
        FindCriteria {
            names: "(".to_owned(),
            name_regex: true,
            ..FindCriteria::default()
        },
    );

    let cleaned = clean(with_searches(vec![broken, search("Mine")])).searches;

    assert_eq!(cleaned, vec![search("Mine")]);
}

#[test]
fn searches_that_could_never_run_are_dropped() {
    let bad_regex = FindCriteria {
        names: "(unclosed".to_owned(),
        name_regex: true,
        ..FindCriteria::default()
    };
    let bad_text_regex = FindCriteria {
        text: Some(FindText {
            pattern: "[".to_owned(),
            regex: true,
            case_sensitive: false,
            whole_word: false,
            encoding: None,
            any_size: false,
        }),
        ..FindCriteria::default()
    };
    let unknown_encoding = FindCriteria {
        text: Some(FindText {
            pattern: "x".to_owned(),
            regex: false,
            case_sensitive: false,
            whole_word: false,
            encoding: Some("no-such-encoding".to_owned()),
            any_size: false,
        }),
        ..FindCriteria::default()
    };
    let backwards_sizes = FindCriteria {
        min_size: Some(9),
        max_size: Some(1),
        ..FindCriteria::default()
    };
    let archives_with_duplicates = FindCriteria {
        archives: true,
        duplicates: true,
        ..FindCriteria::default()
    };
    let mut saved = vec![search("fine")];
    for (name, criteria) in [
        ("regex", bad_regex),
        ("text regex", bad_text_regex),
        ("encoding", unknown_encoding),
        ("sizes", backwards_sizes),
        ("archives", archives_with_duplicates),
    ] {
        saved.push(with_criteria(name, criteria));
    }

    let cleaned = clean(with_searches(saved)).searches;

    assert_eq!(names(&cleaned), vec!["fine"]);
}

#[cfg(windows)]
#[test]
fn a_search_for_executables_is_dropped_on_windows_only() {
    let executable = FindCriteria {
        executable: Tri::Yes,
        ..FindCriteria::default()
    };

    let cleaned = clean(with_searches(vec![with_criteria("exe", executable), search("any")])).searches;

    assert_eq!(names(&cleaned), vec!["any"]);
}

#[cfg(not(windows))]
#[test]
fn a_search_for_executables_is_kept_where_they_can_be_told() {
    let executable = FindCriteria {
        executable: Tri::Yes,
        ..FindCriteria::default()
    };

    let cleaned = clean(with_searches(vec![with_criteria("exe", executable)])).searches;

    assert_eq!(names(&cleaned), vec!["exe"]);
}

#[test]
fn what_loads_has_its_searches_cleaned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut value = saved_json(&file);
    let good = serde_json::to_value(search("good")).expect("json");
    let nameless = serde_json::to_value(search("")).expect("json");
    value["searches"] = json!([nameless, good, serde_json::to_value(search("GOOD")).expect("json")]);
    written(&file, &serde_json::to_vec(&value).expect("json"));

    assert_eq!(load(&file).expect("state").searches, vec![search("good")]);
}

fn update(searches: Option<Vec<SavedSearch>>) -> SavedUpdate {
    SavedUpdate {
        searches,
        ..SavedUpdate::from(state())
    }
}

/// A folder with a state file that holds `searches`.
fn stored(searches: Vec<SavedSearch>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    save(&file, with_searches(searches)).expect("saved");

    (dir, file)
}

#[test]
fn an_update_without_searches_keeps_the_stored_ones() {
    let (_dir, file) = stored(vec![search("one"), search("two")]);
    let mut changed = update(None);
    changed.hotlist = vec![folder("new")];

    save_update(&file, changed, &mut None).expect("saved");

    let loaded = load(&file).expect("state");
    assert_eq!(loaded.hotlist, vec![folder("new")], "the rest is replaced");
    assert_eq!(names(&loaded.searches), vec!["one", "two"]);
}

#[test]
fn an_update_with_an_empty_list_clears_the_stored_searches() {
    let (_dir, file) = stored(vec![search("one")]);

    save_update(&file, update(Some(Vec::new())), &mut None).expect("saved");

    assert_eq!(load(&file).expect("state").searches, Vec::new());
}

#[test]
fn an_update_with_searches_replaces_the_stored_ones() {
    let (_dir, file) = stored(vec![search("one")]);

    save_update(&file, update(Some(vec![search("two")])), &mut None).expect("saved");

    assert_eq!(names(&load(&file).expect("state").searches), vec!["two"]);
}

#[test]
fn an_update_without_searches_has_none_to_keep_when_nothing_is_stored() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    written(&file, b"not json");

    assert!(save_update(&file, update(None), &mut None).expect("saved"));

    assert_eq!(load(&file), Some(state()));
}

#[test]
fn an_update_without_searches_is_one_write_after_another_like_any_state() {
    let (_dir, file) = stored(vec![search("one")]);
    let mut last = None;

    assert!(save_update(&file, update(None), &mut last).expect("first"));
    assert!(
        !save_update(&file, update(None), &mut last).expect("again"),
        "the same bytes"
    );
}

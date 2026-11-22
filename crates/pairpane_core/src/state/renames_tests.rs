//! Saved rename rules in the state file: that they round-trip, that an old file loads without them, that an update
//! without them keeps the stored ones, and that rules which could never run, and names that cannot be used, are dropped.

use pretty_assertions::assert_eq;

use super::tests::{saved_json, state, written};
use super::*;
use crate::model::{CaseChange, CasePart, RenameCounter, RenameRule, SavedRename};
use crate::state::named::MAX_NAME_CHARS;

fn preset(name: &str) -> SavedRename {
    SavedRename {
        name: name.to_owned(),
        rule: RenameRule {
            name: "[N]-[C]".to_owned(),
            ext: "[E]".to_owned(),
            counter: RenameCounter {
                start: 1,
                step: 1,
                digits: 3,
            },
            search: String::new(),
            replace: String::new(),
            regex: false,
            match_case: false,
            first_only: false,
            case: CaseChange::Keep,
            case_part: CasePart::Both,
        },
    }
}

fn update(renames: Option<Vec<SavedRename>>) -> SavedUpdate {
    SavedUpdate {
        renames,
        ..SavedUpdate::from(state())
    }
}

#[test]
fn a_file_from_before_saved_renames_loads_with_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut value = saved_json(&file);
    value.as_object_mut().expect("object").remove("renames");
    written(&file, &serde_json::to_vec(&value).expect("json"));

    assert_eq!(load(&file).expect("state").renames, Vec::new());
}

#[test]
fn saved_renames_survive_a_round_trip_and_an_update_without_them_keeps_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    save(
        &file,
        SavedState {
            renames: vec![preset("dated")],
            ..state()
        },
    )
    .expect("saved");
    assert_eq!(load(&file).expect("state").renames, vec![preset("dated")]);

    save_update(&file, update(None), &mut None).expect("saved without renames");
    assert_eq!(load(&file).expect("state").renames, vec![preset("dated")], "kept");

    save_update(&file, update(Some(Vec::new())), &mut None).expect("saved with none");
    assert_eq!(
        load(&file).expect("state").renames,
        Vec::new(),
        "an empty list clears them"
    );
}

fn with_rule(name: &str, change: impl FnOnce(&mut RenameRule)) -> SavedRename {
    let mut saved = preset(name);
    change(&mut saved.rule);

    saved
}

fn cleaned_names(renames: Vec<SavedRename>) -> Vec<String> {
    clean(SavedState { renames, ..state() })
        .renames
        .into_iter()
        .map(|saved| saved.name)
        .collect()
}

#[test]
fn a_rule_that_could_never_run_is_dropped_on_load() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let stored = SavedState {
        renames: vec![
            with_rule("bad placeholder", |rule| rule.name = "[Q]".to_owned()),
            with_rule("unclosed", |rule| rule.ext = "[N".to_owned()),
            with_rule("counter", |rule| rule.counter.digits = 11),
            with_rule("regex", |rule| {
                rule.search = "(".to_owned();
                rule.regex = true;
            }),
            with_rule("huge", |rule| rule.replace = "x".repeat(1025)),
            with_rule("fine", |rule| rule.name = "[N2-]".to_owned()),
        ],
        ..state()
    };
    // Written as it is, without the cleaning of `save`, as a file edited by hand would be.
    let mut value = saved_json(&file);
    value["renames"] = serde_json::to_value(&stored.renames).expect("json");
    written(&file, &serde_json::to_vec(&value).expect("json"));

    let loaded = load(&file).expect("state").renames;

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "fine");
}

#[test]
fn names_of_rules_are_trimmed_unique_ignoring_case_and_at_most_sixty_four_characters() {
    let too_long = "n".repeat(MAX_NAME_CHARS + 1);
    let wide = "ł".repeat(MAX_NAME_CHARS);
    let broken = with_rule("Mine", |rule| rule.counter.step = 0);

    let kept = cleaned_names(vec![
        preset(" padded "),
        preset(""),
        preset(&too_long),
        preset(&wide),
        broken,
        preset("MINE"),
        preset("mine"),
        preset("PADDED"),
    ]);

    assert_eq!(kept, ["padded", wide.as_str(), "MINE"]);
}

#[test]
fn at_most_fifty_rules_are_kept_the_first_ones() {
    let many: Vec<SavedRename> = (0..MAX_RENAMES + 3).map(|n| preset(&format!("r{n}"))).collect();

    let kept = cleaned_names(many);

    assert_eq!(kept.len(), MAX_RENAMES);
    assert_eq!(kept.last().map(String::as_str), Some("r49"));
}

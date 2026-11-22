//! Saved rename rules in the state file: that they round-trip, that an old file loads without them, and that an update
//! without them keeps the stored ones.

use pretty_assertions::assert_eq;

use super::tests::{saved_json, state, written};
use super::*;
use crate::model::{CaseChange, CasePart, RenameCounter, RenameRule, SavedRename};

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

//! The keys the user set, in the state file: what is kept, what goes, and that a page that sends none keeps them.

use pretty_assertions::assert_eq;

use super::tests::state;
use super::*;
use crate::model::{KeyChord, KeyOverride, SavedState};

fn chord(key: &str, ctrl: bool) -> KeyChord {
    KeyChord {
        key: key.to_owned(),
        ctrl,
        alt: false,
        shift: false,
    }
}

fn set(action: &str, keys: Vec<KeyChord>) -> KeyOverride {
    KeyOverride {
        action: action.to_owned(),
        keys,
    }
}

#[test]
fn keys_survive_a_round_trip_and_an_update_without_them_keeps_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let keys = vec![set("copy", vec![chord("c", true)]), set("trash", Vec::new())];
    save(
        &file,
        SavedState {
            keys: keys.clone(),
            ..state()
        },
    )
    .expect("saved");
    assert_eq!(load(&file).expect("state").keys, keys);

    let update = SavedUpdate {
        keys: None,
        ..SavedUpdate::from(state())
    };
    save_update(&file, update, &mut None).expect("saved without keys");

    assert_eq!(load(&file).expect("state").keys, keys, "kept");
}

#[test]
fn what_cannot_be_a_key_of_an_action_goes_and_the_list_is_kept_small() {
    let mut keys = vec![
        set(
            "copy",
            vec![
                chord("c", true),
                chord("", true),
                chord("\u{7}", false),
                chord(&"x".repeat(33), false),
            ],
        ),
        set("copy", vec![chord("d", true)]),
        set("bad action!", vec![chord("e", true)]),
        set("move", (0..9).map(|n| chord(&format!("F{n}"), false)).collect()),
    ];
    keys.extend((0..100).map(|n| set(&"a".repeat(n % 30 + 1).replace('a', "b"), Vec::new())));

    let cleaned = clean(SavedState { keys, ..state() }).keys;

    assert_eq!(cleaned[0], set("copy", vec![chord("c", true)]));
    assert_eq!(cleaned[1].action, "move");
    assert_eq!(cleaned[1].keys.len(), MAX_KEYS_PER_ACTION);
    assert!(cleaned.len() <= MAX_KEY_OVERRIDES);
    assert!(cleaned.iter().all(|entry| entry.action != "bad action!"));
}

#[test]
fn a_file_from_before_keys_loads_with_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    save(&file, state()).expect("saved");
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).expect("read")).expect("json");
    value.as_object_mut().expect("object").remove("keys");
    std::fs::write(&file, serde_json::to_vec(&value).expect("json")).expect("written");

    assert_eq!(load(&file).expect("state").keys, Vec::new());
}

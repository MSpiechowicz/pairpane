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

#[test]
fn an_action_none_of_whose_keys_is_left_keeps_its_defaults_and_one_set_to_none_stays() {
    let keys = vec![set("copy", vec![chord("", true)]), set("trash", Vec::new())];

    let cleaned = clean(SavedState { keys, ..state() }).keys;

    assert_eq!(cleaned, vec![set("trash", Vec::new())]);
}

#[test]
fn the_preferences_come_back_and_an_update_without_them_keeps_the_stored_ones() {
    use crate::model::{Preferences, ThemeChoice};

    let folder = tempfile::tempdir().expect("a folder");
    let file = folder.path().join("state.json");
    let preferences = Preferences {
        theme: ThemeChoice::Dark,
        show_ext: false,
        confirm_trash: false,
        ..Preferences::default()
    };
    save(
        &file,
        SavedState {
            preferences: Some(preferences.clone()),
            ..state()
        },
    )
    .expect("saved");

    let mut update = SavedUpdate::from(state());
    update.preferences = None;
    save_update(&file, update, &mut None).expect("saved again");

    assert_eq!(load(&file).expect("loaded").preferences, Some(preferences));
}

#[test]
fn a_file_from_before_the_preferences_has_none_so_the_page_keeps_what_it_had() {
    let folder = tempfile::tempdir().expect("a folder");
    let file = folder.path().join("state.json");
    save(&file, state()).expect("saved");

    let text = std::fs::read_to_string(&file).expect("read");

    assert!(!text.contains("preferences"), "{text}");
    assert_eq!(load(&file).expect("loaded").preferences, None);
}

#[test]
fn a_file_from_before_the_preferences_or_with_some_of_them_takes_the_defaults_for_the_rest() {
    let partial: crate::model::Preferences = serde_json::from_str(r#"{"showHidden": true, "later": 1}"#).expect("read");

    assert!(partial.show_hidden);
    assert!(partial.confirm_trash && partial.show_size && partial.check_updates_at_start);
}

#[test]
fn preferences_that_cannot_be_read_take_their_defaults_and_never_lose_the_rest_of_the_state() {
    use crate::model::ThemeChoice;

    let base = serde_json::to_value(state()).expect("json");
    let with = |preferences: serde_json::Value| {
        let mut value = base.clone();
        value["preferences"] = preferences;
        serde_json::from_value::<SavedState>(value).expect("the rest of the state is kept")
    };

    let newer = with(serde_json::json!({ "theme": "sepia", "showExt": false }));
    let newer = newer.preferences.expect("read");
    assert_eq!((newer.theme, newer.show_ext), (ThemeChoice::System, false));

    let broken = with(serde_json::json!({ "showExt": "no", "confirmTrash": false }));
    let read = broken.preferences.expect("read");
    assert!(
        read.show_ext && !read.confirm_trash,
        "a field of the wrong kind takes its default, the others are kept"
    );
    assert_eq!(broken.hotlist, state().hotlist);

    let not_an_object = with(serde_json::json!("dark"));
    assert_eq!(not_an_object.preferences, Some(crate::model::Preferences::default()));
}

#[test]
fn a_tab_keeps_its_layout_and_one_from_before_layouts_has_none() {
    use crate::model::PaneLayout;

    let folder = tempfile::tempdir().expect("a folder");
    let file = folder.path().join("state.json");
    let mut saved = state();
    saved.left.tabs[0].layout = Some(PaneLayout::Brief);
    save(&file, saved).expect("saved");

    let loaded = load(&file).expect("loaded");
    assert_eq!(loaded.left.tabs[0].layout, Some(PaneLayout::Brief));
    assert_eq!(loaded.left.tabs[1].layout, None);
    assert!(!std::fs::read_to_string(&file).expect("read").contains("null"));
}

#[test]
fn a_layout_this_version_does_not_know_is_the_detailed_one_and_the_state_is_kept() {
    use crate::model::PaneLayout;

    let mut value = serde_json::to_value(state()).expect("json");
    value["left"]["tabs"][0]["layout"] = serde_json::json!("icons");
    value["left"]["tabs"][1]["layout"] = serde_json::json!(7);

    let read: SavedState = serde_json::from_value(value).expect("read");

    assert_eq!(read.left.tabs[0].layout, Some(PaneLayout::Detailed));
    assert_eq!(read.left.tabs[1].layout, Some(PaneLayout::Detailed));
    assert_eq!(read.hotlist, state().hotlist);
}

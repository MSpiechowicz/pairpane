use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;
use crate::model::{PaneId, SortColumn};

fn folder(name: &str) -> String {
    let root = if cfg!(windows) { "C:\\data" } else { "/data" };

    Path::new(root).join(name).to_string_lossy().into_owned()
}

/// A path that fails `check_folder` on every system.
fn relative() -> String {
    "photos/2024".to_owned()
}

fn tab(name: &str) -> SavedTab {
    SavedTab {
        path: folder(name),
        sort: SortColumn::Name,
        desc: false,
    }
}

fn pane(names: &[&str], active: u32) -> SavedPane {
    SavedPane {
        tabs: names.iter().map(|name| tab(name)).collect(),
        active,
    }
}

fn state() -> SavedState {
    SavedState {
        active: PaneId::Right,
        left: SavedPane {
            tabs: vec![
                tab("a"),
                SavedTab {
                    path: folder("b"),
                    sort: SortColumn::Modified,
                    desc: true,
                },
            ],
            active: 1,
        },
        right: pane(&["c"], 0),
        hotlist: vec![folder("fav"), folder("other")],
        searches: Vec::new(),
    }
}

fn written(file: &Path, contents: &[u8]) {
    fs::write(file, contents).expect("write");
}

#[test]
fn a_clean_state_is_left_as_it_is() {
    assert_eq!(clean(state()), state());
}

#[test]
fn paths_that_fail_the_folder_check_are_dropped() {
    let climbing = format!("{}{}", folder("up"), if cfg!(windows) { "\\..\\x" } else { "/../x" });
    let mut saved = state();
    saved.left.tabs.insert(
        0,
        SavedTab {
            path: relative(),
            ..tab("x")
        },
    );
    saved.left.active = 2;
    saved.hotlist.insert(1, "..".to_owned());
    saved.hotlist.push(climbing);

    let cleaned = clean(saved);

    assert_eq!(cleaned.left.tabs.len(), 2);
    assert_eq!(cleaned.left.tabs[0].path, folder("a"));
    assert_eq!(
        cleaned.left.active, 1,
        "the active tab moved up with the dropped one before it"
    );
    assert_eq!(cleaned.hotlist, vec![folder("fav"), folder("other")]);
}

#[cfg(windows)]
#[test]
fn a_network_path_is_dropped_on_windows() {
    let mut saved = state();
    saved.hotlist.push(r"\\server\share\docs".to_owned());
    saved.right.tabs.push(SavedTab {
        path: r"\\server\share".to_owned(),
        ..tab("x")
    });

    let cleaned = clean(saved);

    assert_eq!(cleaned.hotlist.len(), 2);
    assert_eq!(cleaned.right.tabs.len(), 1);
}

#[test]
fn a_dropped_active_tab_hands_over_to_the_next_survivor_else_the_last() {
    let mut dropped = pane(&["a", "b", "c"], 1);
    dropped.tabs[1].path = relative();
    let mut dropped_last = pane(&["a", "b", "c"], 2);
    dropped_last.tabs[2].path = relative();

    assert_eq!(clean_pane(dropped).active, 1, "c took the place of b");
    assert_eq!(clean_pane(dropped_last).active, 1, "b is the last one left");
}

#[test]
fn the_active_index_is_clamped_to_the_tabs() {
    assert_eq!(clean_pane(pane(&["a", "b"], 7)).active, 1);
    assert_eq!(clean_pane(pane(&["a", "b"], u32::MAX)).active, 1);
    assert_eq!(clean_pane(pane(&[], 3)), pane(&[], 0));
}

#[test]
fn tabs_and_favourites_are_capped_and_the_active_tab_stays_in_range() {
    let names: Vec<String> = (0..MAX_TABS + 10).map(|n| format!("t{n}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut saved = state();
    saved.left = pane(&names, (MAX_TABS + 5) as u32);
    saved.hotlist = (0..MAX_HOTLIST + 20).map(|n| folder(&format!("f{n}"))).collect();

    let cleaned = clean(saved);

    assert_eq!(cleaned.left.tabs.len(), MAX_TABS);
    assert_eq!(cleaned.left.active as usize, MAX_TABS - 1);
    assert_eq!(cleaned.hotlist.len(), MAX_HOTLIST);
    assert_eq!(cleaned.hotlist[0], folder("f0"), "the first ones stay");
}

#[test]
fn repeated_favourites_are_kept_once_in_their_first_place() {
    let mut saved = state();
    saved.hotlist = vec![folder("a"), folder("b"), folder("a"), folder("c"), folder("b")];

    assert_eq!(clean(saved).hotlist, vec![folder("a"), folder("b"), folder("c")]);
}

/// The JSON of a state file saved from `state()`, to edit and write back.
fn saved_json(file: &Path) -> serde_json::Value {
    save(file, state()).expect("saved");

    serde_json::from_slice(&fs::read(file).expect("read")).expect("json")
}

#[test]
fn a_state_survives_a_round_trip_through_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");

    save(&file, state()).expect("saved");

    assert_eq!(load(&file), Some(state()));
}

#[test]
fn the_file_holds_the_version_and_camel_case_fields() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");

    save(&file, state()).expect("saved");

    let value: serde_json::Value = serde_json::from_slice(&fs::read(&file).expect("read")).expect("json");
    assert_eq!(value["version"], json!(VERSION));
    assert_eq!(value["active"], json!("right"));
    assert_eq!(value["left"]["tabs"][1]["sort"], json!("modified"));
    assert_eq!(value["left"]["tabs"][1]["desc"], json!(true));
}

#[test]
fn saving_cleans_the_state_and_replaces_an_earlier_file_without_leaving_a_temporary_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    save(&file, state()).expect("first");

    let mut next = state();
    next.hotlist = vec![relative(), folder("kept")];
    save(&file, next).expect("second");

    assert_eq!(load(&file).expect("state").hotlist, vec![folder("kept")]);
    let files: Vec<_> = fs::read_dir(dir.path()).expect("dir").collect();
    assert_eq!(files.len(), 1, "only state.json is left");
}

#[test]
fn saving_creates_the_missing_folder() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("config").join("app").join("state.json");

    save(&file, state()).expect("saved");

    assert_eq!(load(&file), Some(state()));
}

#[test]
fn saving_the_same_state_again_writes_nothing_but_a_changed_one_is_written() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut last = None;

    assert!(save_changed(&file, state(), &mut last).expect("first"));
    fs::remove_file(&file).expect("removed, so that a write would bring it back");

    assert!(
        !save_changed(&file, state(), &mut last).expect("same"),
        "nothing to write"
    );
    assert!(!file.exists(), "no write happened");

    let mut next = state();
    next.hotlist = vec![folder("new")];
    assert!(save_changed(&file, next, &mut last).expect("changed"));
    assert_eq!(load(&file).expect("state").hotlist, vec![folder("new")]);
}

#[test]
fn states_that_clean_to_the_same_bytes_are_written_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut last = None;
    let mut noisy = state();
    noisy.hotlist.push(relative());

    assert!(save_changed(&file, state(), &mut last).expect("first"));

    assert!(!save_changed(&file, noisy, &mut last).expect("same once cleaned"));
}

#[test]
fn a_failed_write_is_not_remembered_so_the_next_save_tries_again() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    fs::create_dir(&file).expect("a folder where the file should be");
    let mut last = None;

    save_changed(&file, state(), &mut last).expect_err("cannot replace a folder");
    assert_eq!(last, None);

    fs::remove_dir(&file).expect("removed");
    assert!(save_changed(&file, state(), &mut last).expect("retried"));
}

/// Paths within their length limit still add up to more than the file may hold when every character is escaped in
/// JSON (a control character takes six bytes); only Unix allows such names.
#[cfg(unix)]
#[test]
fn a_state_too_big_for_the_file_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let long = folder(&"\u{1}".repeat(crate::listing::platform::MAX_PATH_BYTES - 20));
    let mut saved = state();
    saved.hotlist = (0..MAX_HOTLIST).map(|n| format!("{long}{n}")).collect();

    let error = save(&file, saved).expect_err("too big");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!file.exists());
}

#[test]
fn there_is_nothing_to_load_without_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");

    assert_eq!(load(&dir.path().join("state.json")), None);
}

#[test]
fn a_file_that_is_not_a_state_loads_as_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");

    for contents in [
        &b""[..],
        b"not json",
        b"{\"version\": 1",
        b"[1, 2, 3]",
        b"{}",
        b"{\"version\": 1}",
        b"{\"version\": 1, \"active\": \"middle\", \"left\": {\"tabs\": [], \"active\": 0}}",
        b"\xff\xfe\x00\x01",
    ] {
        written(&file, contents);
        assert_eq!(load(&file), None, "{}", String::from_utf8_lossy(contents));
    }
}

#[test]
fn a_file_of_another_version_loads_as_nothing_even_when_it_looks_right() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut value = saved_json(&file);

    for version in [json!(0), json!(VERSION + 1), json!("1"), json!(null)] {
        value["version"] = version;
        written(&file, &serde_json::to_vec(&value).expect("json"));

        assert_eq!(load(&file), None);
    }
}

#[test]
fn unknown_fields_are_ignored() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let mut value = saved_json(&file);
    value["theme"] = json!("dark");
    value["left"]["zoom"] = json!(2);
    written(&file, &serde_json::to_vec(&value).expect("json"));

    assert_eq!(load(&file), Some(state()));
}

#[test]
fn a_file_over_the_size_limit_loads_as_nothing_without_being_parsed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    save(&file, state()).expect("saved");
    let mut bytes = fs::read(&file).expect("read");
    bytes.resize(MAX_FILE_BYTES as usize + 1, b' ');
    written(&file, &bytes);
    assert_eq!(load(&file), None, "valid JSON, but over the limit");

    bytes.truncate(MAX_FILE_BYTES as usize);
    written(&file, &bytes);
    assert_eq!(load(&file), Some(state()), "exactly at the limit");
}

#[test]
fn what_loads_is_cleaned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    let value = json!({
        "version": 1,
        "active": "left",
        "left": { "tabs": [
            { "path": "relative", "sort": "name", "desc": false },
            { "path": folder("a"), "sort": "size", "desc": true },
        ], "active": 9 },
        "right": { "tabs": [], "active": 4 },
        "hotlist": ["nope", folder("fav")],
    });
    written(&file, &serde_json::to_vec(&value).expect("json"));

    let loaded = load(&file).expect("state");

    assert_eq!(loaded.left.tabs.len(), 1);
    assert_eq!(loaded.left.active, 0);
    assert_eq!(loaded.right, pane(&[], 0));
    assert_eq!(loaded.hotlist, vec![folder("fav")]);
}

#[test]
fn a_folder_where_the_file_should_be_loads_as_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("state.json");
    fs::create_dir(&file).expect("dir");

    assert_eq!(load(&file), None);
}

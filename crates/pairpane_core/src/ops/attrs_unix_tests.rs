//! Changing permissions and times on Unix: modes, whole trees, links that are never followed, and the change of an
//! item that cannot be opened.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{AttributeChange, ModeChange, Outcome};

use super::Task;
use super::attrs_tests::{nothing, run};
use super::testing::{Rig, long_ago, modified, put, set_times, task, temp};

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("metadata").permissions().mode() & 0o7777
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("mode");
}

fn mode_change(mode: ModeChange, recursive: bool) -> AttributeChange {
    AttributeChange {
        mode: Some(mode),
        recursive,
        ..nothing()
    }
}

#[test]
fn permissions_are_set_exactly_or_adjusted() {
    let root = temp();
    put(root.path(), "src/f", b"x");
    let file = root.path().join("src/f");
    set_mode(&file, 0o644);

    let ended = run(
        root.path(),
        &["f"],
        mode_change(ModeChange::Set { mode: 0o4750 }, false),
    );
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(mode_of(&file), 0o4750);

    let adjust = ModeChange::Adjust {
        add: 0o005,
        remove: 0o4040,
    };
    run(root.path(), &["f"], mode_change(adjust, false));
    assert_eq!(mode_of(&file), 0o0715);
}

#[test]
fn a_whole_tree_goes_to_no_access_and_back() {
    let root = temp();
    put(root.path(), "src/d/f", b"x");
    put(root.path(), "src/d/sub/g", b"x");
    let top = root.path().join("src/d");

    // Contents first, as the folder cannot be listed once it is changed.
    let ended = run(root.path(), &["d"], mode_change(ModeChange::Set { mode: 0 }, true));
    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(mode_of(&top), 0);

    // Back: the folder first, though it is at 000 now.
    let ended = run(root.path(), &["d"], mode_change(ModeChange::Set { mode: 0o700 }, true));
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    for path in ["d", "d/f", "d/sub", "d/sub/g"] {
        assert_eq!(mode_of(&root.path().join("src").join(path)), 0o700, "{path}");
    }
}

#[test]
fn a_file_without_any_permission_can_still_be_changed() {
    let root = temp();
    put(root.path(), "src/f", b"x");
    let file = root.path().join("src/f");
    set_mode(&file, 0);

    let ended = run(root.path(), &["f"], mode_change(ModeChange::Set { mode: 0o644 }, false));

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(mode_of(&file), 0o644);
}

#[test]
fn a_link_in_the_selection_or_in_a_folder_is_skipped_and_what_it_leads_to_is_not_touched() {
    let root = temp();
    put(root.path(), "outside/target", b"x");
    put(root.path(), "outside/folder/inner", b"x");
    put(root.path(), "src/d/f", b"x");
    let target = root.path().join("outside/target");
    let folder = root.path().join("outside/folder");
    set_mode(&target, 0o644);
    set_mode(&folder, 0o755);
    set_times(&target, long_ago());
    set_times(&folder, long_ago());
    symlink(&target, root.path().join("src/link")).expect("link");
    symlink(&folder, root.path().join("src/d/folder-link")).expect("link");
    symlink(root.path().join("src/nowhere"), root.path().join("src/d/dangling")).expect("link");
    let change = AttributeChange {
        mode: Some(ModeChange::Set { mode: 0o700 }),
        modified: Some(1_500_000_000_000),
        recursive: true,
        ..nothing()
    };

    let ended = run(root.path(), &["link", "d"], change);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(ended.skipped, 3);
    assert_eq!((mode_of(&target), mode_of(&folder)), (0o644, 0o755));
    assert_eq!((modified(&target), modified(&folder)), (long_ago(), long_ago()));
    assert_eq!(mode_of(&root.path().join("src/d/f")), 0o700);
    assert_eq!(mode_of(&root.path().join("src/d")), 0o700);
}

#[test]
fn the_fallback_never_follows_a_link_swapped_in_for_the_item() {
    use super::held::Held;

    let root = temp();
    put(root.path(), "dir/x", b"x");
    put(root.path(), "victim", b"victim");
    let (item, victim) = (root.path().join("dir/x"), root.path().join("victim"));
    set_mode(&item, 0);
    set_mode(&victim, 0o644);
    let folder = Held::root(&root.path().join("dir")).expect("folder");
    let seen = folder.look("x").expect("looked at");

    // The item stays alive under another name, so no other file can be given its inode number.
    fs::hard_link(&item, root.path().join("keep")).expect("kept");

    // The item is swapped for a link to another file after it was looked at.
    fs::remove_file(&item).expect("removed");
    symlink(&victim, &item).expect("link");
    for mode in [
        ModeChange::Set { mode: 0o777 },
        ModeChange::Adjust { add: 0o111, remove: 0 },
    ] {
        let result = folder.set_without_opening(&seen, mode);

        assert!(result.is_err(), "the link is refused");
        assert_eq!(mode_of(&victim), 0o644, "what it leads to keeps its permissions");
    }

    // Swapped for another file (made first, so it has an inode of its own): not the item that was seen.
    put(root.path(), "dir/y", b"another");
    set_mode(&root.path().join("dir/y"), 0o644);
    fs::rename(root.path().join("dir/y"), &item).expect("swapped");

    let result = folder.set_without_opening(&seen, ModeChange::Set { mode: 0o777 });

    assert!(result.is_err());
    assert_eq!(mode_of(&item), 0o644);
}

#[test]
fn the_fallback_changes_the_item_that_was_seen() {
    use super::held::Held;

    let root = temp();
    put(root.path(), "dir/x", b"x");
    let item = root.path().join("dir/x");
    set_mode(&item, 0o000);
    let folder = Held::root(&root.path().join("dir")).expect("folder");
    let seen = folder.look("x").expect("looked at");

    folder
        .set_without_opening(&seen, ModeChange::Adjust { add: 0o600, remove: 0 })
        .expect("changed");

    assert_eq!(mode_of(&item), 0o600, "the mode adjusted is the one the item has now");
}

#[test]
fn a_folder_that_is_a_link_to_a_folder_can_be_the_source() {
    let root = temp();
    put(root.path(), "real/f", b"x");
    put(root.path(), "other/f", b"x");
    for path in ["real/f", "other/f"] {
        set_mode(&root.path().join(path), 0o644);
    }
    let link = root.path().join("shown");
    symlink(root.path().join("real"), &link).expect("link");

    let ended = Rig::new().run(task(
        &link,
        &["f"],
        &link,
        Task::Attributes(mode_change(ModeChange::Set { mode: 0o600 }, false)),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(mode_of(&root.path().join("real/f")), 0o600);
    assert_eq!(mode_of(&root.path().join("other/f")), 0o644);
}

#[test]
fn a_pipe_is_left_alone_and_counted() {
    let root = temp();
    put(root.path(), "src/d/f", b"x");
    super::testing::make_fifo(&root.path().join("src/d/pipe"));
    let fifo = root.path().join("src/d/pipe");
    let before = mode_of(&fifo);

    let ended = run(root.path(), &["d"], mode_change(ModeChange::Set { mode: 0o755 }, true));

    assert_eq!(ended.skipped, 1);
    assert_eq!(mode_of(&fifo), before);
}

#[test]
fn what_cannot_be_changed_is_reported_and_the_rest_goes_on() {
    let root = temp();
    put(root.path(), "src/gone", b"x");
    put(root.path(), "src/ok", b"x");
    fs::remove_file(root.path().join("src/gone")).expect("removed");

    let ended = run(
        root.path(),
        &["gone", "ok"],
        mode_change(ModeChange::Set { mode: 0o600 }, false),
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "gone");
    assert_eq!(mode_of(&root.path().join("src/ok")), 0o600);
}

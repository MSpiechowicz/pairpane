//! Walking by handle on Unix: what a folder swapped for a link while the walk is under way can and cannot do.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pretty_assertions::assert_eq;

use crate::model::{AttributeChange, HashAlgorithm, ModeChange, Outcome};

use super::Task;
use super::held::Held;
use super::hooks::Hooks;
use super::plan::Kind;
use super::testing::{Ended, Rig, long_ago, modified, put, set_times, task, temp, tree};

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("metadata").permissions().mode() & 0o7777
}

/// A rig whose walk finds the folder `rel` (below `src`) swapped for a link to `outside` just before it is opened: the
/// folder is moved aside and the link takes its name, as an attacker who can write to `src` would do.
fn swapping(root: &Path, rel: &'static str) -> Rig {
    let (folder, aside, outside): (PathBuf, PathBuf, PathBuf) = (
        root.join("src").join(rel),
        root.join("src").join(format!("{rel}.aside")),
        root.join("outside"),
    );
    let hooks = Hooks {
        before_open: Some(Arc::new(move |now| {
            if now == rel {
                fs::rename(&folder, &aside).expect("moved aside");
                symlink(&outside, &folder).expect("link");
            }
        })),
        ..Hooks::default()
    };

    Rig::with(hooks, |_, _| {})
}

/// A folder `outside` with files and a subfolder, each with a mode and times that a change would show.
fn outside(root: &Path) -> Vec<PathBuf> {
    put(root, "outside/g", b"secret");
    put(root, "outside/sub/h", b"secret");
    let paths: Vec<PathBuf> = ["outside", "outside/g", "outside/sub", "outside/sub/h"]
        .iter()
        .map(|path| root.join(path))
        .collect();

    for path in &paths {
        let mode = if path.ends_with("g") || path.ends_with("h") {
            0o644
        } else {
            0o755
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("mode");
        set_times(path, long_ago());
    }

    paths
}

fn untouched(paths: &[PathBuf]) {
    for path in paths {
        let expected = if path.ends_with("g") || path.ends_with("h") {
            0o644
        } else {
            0o755
        };

        assert_eq!(mode_of(path), expected, "{}", path.display());
        assert_eq!(modified(path), long_ago(), "{}", path.display());
    }
}

fn change_everything() -> Task {
    Task::Attributes(AttributeChange {
        mode: Some(ModeChange::Set { mode: 0o700 }),
        read_only: None,
        hidden: None,
        system: None,
        archive: None,
        modified: Some(1_500_000_000_000),
        accessed: None,
        recursive: true,
    })
}

fn run_swapped(rel: &'static str) -> (tempfile::TempDir, Ended, Vec<PathBuf>) {
    let root = temp();
    put(root.path(), "src/d/f", b"x");
    put(root.path(), "src/d/sub/f", b"x");
    let watched = outside(root.path());
    let src = root.path().join("src");

    let ended = swapping(root.path(), rel).run(task(&src, &["d"], &src, change_everything()));

    (root, ended, watched)
}

#[test]
fn a_selected_folder_swapped_for_a_link_leaves_what_it_leads_to_alone() {
    let (_root, ended, watched) = run_swapped("d");

    untouched(&watched);
    assert_eq!(ended.outcome, Outcome::Done);
    assert!(!ended.errors.is_empty(), "the folder is reported");
}

#[test]
fn a_folder_inside_a_selected_folder_swapped_for_a_link_leaves_what_it_leads_to_alone() {
    let (root, ended, watched) = run_swapped("d/sub");

    untouched(&watched);
    assert!(!ended.errors.is_empty(), "the folder is reported");
    assert_eq!(
        mode_of(&root.path().join("src/d/f")),
        0o700,
        "the rest of the tree is changed"
    );
}

#[test]
fn a_folder_swapped_for_a_link_is_not_read_by_a_checksum_job() {
    let root = temp();
    put(root.path(), "src/d/f", b"abc");
    put(root.path(), "outside/secret", b"secret");
    let src = root.path().join("src");

    let ended = swapping(root.path(), "d").run(task(
        &src,
        &["d"],
        &src,
        Task::Checksum {
            algorithm: HashAlgorithm::Sha256,
            output: "S.sha256".to_owned(),
        },
    ));

    assert!(
        matches!(ended.outcome, Outcome::Failed { .. }),
        "nothing was read to list"
    );
    assert!(!ended.errors.is_empty());
    assert_eq!(
        tree(&src),
        ["d", "d.aside/", "d.aside/f"],
        "no checksum file, and the link stays a link"
    );
}

#[test]
fn a_folder_that_is_swapped_for_a_link_or_another_folder_cannot_be_opened() {
    let root = temp();
    put(root.path(), "src/d/f", b"x");
    put(root.path(), "elsewhere/f", b"x");
    let held = Held::root(&root.path().join("src")).expect("root");
    let seen = held.look("d").expect("looked at");
    assert_eq!(seen.kind, Kind::Dir);
    assert!(held.subfolder(&seen).is_ok());

    // Swapped for a link.
    fs::rename(root.path().join("src/d"), root.path().join("aside")).expect("moved");
    symlink(root.path().join("elsewhere"), root.path().join("src/d")).expect("link");
    assert!(held.subfolder(&seen).is_err());

    // Swapped for another folder.
    fs::remove_file(root.path().join("src/d")).expect("removed");
    fs::create_dir(root.path().join("src/d")).expect("folder");
    assert!(held.subfolder(&seen).is_err());
}

#[test]
fn an_entry_is_looked_up_by_one_name_and_never_by_a_path_that_could_go_through_a_link() {
    let root = temp();
    put(root.path(), "src/z", b"x");
    put(root.path(), "outside/b.txt", b"secret");
    symlink(root.path().join("outside"), root.path().join("src/a")).expect("link");
    let held = Held::root(&root.path().join("src")).expect("root");

    assert!(held.look("a/b.txt").is_err(), "a link on the way is not followed");
    assert!(held.look("../outside").is_err());
    assert!(held.look("z").is_ok());
    assert_eq!(held.look("a").expect("the link itself").kind, Kind::Link);
}

#[test]
fn a_file_swapped_for_a_link_or_another_file_cannot_be_opened() {
    let root = temp();
    put(root.path(), "src/f", b"abc");
    put(root.path(), "other", b"other");
    let held = Held::root(&root.path().join("src")).expect("root");
    let seen = held.look("f").expect("looked at");
    assert!(held.open_file(&seen).is_ok());

    // The first file stays alive under another name, so no other file can be given its inode number.
    fs::hard_link(root.path().join("src/f"), root.path().join("keep")).expect("kept");

    // Replaced by a link (made first and renamed over the file, so nothing is ever missing).
    symlink(root.path().join("other"), root.path().join("src/tmp")).expect("link");
    fs::rename(root.path().join("src/tmp"), root.path().join("src/f")).expect("swapped");
    assert!(held.open_file(&seen).is_err());

    // Replaced by another file, which has an inode of its own.
    put(root.path(), "src/g", b"abc");
    fs::rename(root.path().join("src/g"), root.path().join("src/f")).expect("swapped");
    assert!(held.open_file(&seen).is_err());
}

#[test]
fn a_folder_is_listed_through_its_handle_with_links_as_links() {
    let root = temp();
    put(root.path(), "src/d/file", b"x");
    put(root.path(), "src/d/folder/inner", b"x");
    symlink(root.path().join("src"), root.path().join("src/d/link")).expect("link");
    let held = Held::root(&root.path().join("src")).expect("root");
    let d = held.look("d").expect("looked at");

    let listed = held.subfolder(&d).expect("opened").entries().expect("listed");

    let mut found: Vec<(String, Kind)> = listed
        .nodes
        .iter()
        .map(|node| (node.name.to_string_lossy().into_owned(), node.kind))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        vec![
            ("file".to_owned(), Kind::File),
            ("folder".to_owned(), Kind::Dir),
            ("link".to_owned(), Kind::Link),
        ]
    );
    assert!(listed.failed.is_empty());
}

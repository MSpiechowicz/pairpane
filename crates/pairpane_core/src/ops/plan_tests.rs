//! Looking before copying: a folder into itself, folder walks that never follow a link, and name comparison.

use std::fs;
use std::path::Path;

use pretty_assertions::assert_eq;

use crate::model::{IpcErrorCode, Outcome};

use super::identity::FileId;
#[cfg(unix)]
use super::plan::Kind;
use super::plan::{Child, is_inside, is_inside_with, lands_in_itself, read_children};
use super::testing::{Rig, copy, relocate, temp, tree, write};

#[test]
fn a_path_is_inside_another_by_whole_names() {
    let outer = Path::new("/data/photos");

    assert!(is_inside_with(Path::new("/data/photos"), outer, false));
    assert!(is_inside_with(Path::new("/data/photos/2024/may"), outer, false));
    assert!(!is_inside_with(Path::new("/data/photos-old"), outer, false));
    assert!(!is_inside_with(Path::new("/data"), outer, false));
    assert!(!is_inside_with(Path::new("/other/photos"), outer, false));
}

#[test]
fn case_is_ignored_only_when_asked_to() {
    let outer = Path::new("/data/Photos");

    assert!(!is_inside_with(Path::new("/data/photos/x"), outer, false));
    assert!(is_inside_with(Path::new("/DATA/photos/x"), outer, true));
    assert_eq!(
        is_inside(Path::new("/data/PHOTOS"), outer),
        cfg!(any(target_os = "macos", target_os = "windows"))
    );
}

#[test]
fn a_folder_is_refused_into_itself_and_below_itself_before_anything_is_written() {
    let root = temp();
    write(&root.path().join("src/tree/a.txt"), b"a");
    fs::create_dir_all(root.path().join("src/tree/sub")).expect("folder");
    let mut rig = Rig::new();
    let before = tree(root.path());

    for target in [
        "src",
        "src/tree",
        "src/tree/sub",
        "src/tree/new/deeper",
        "src/tree/sub/new",
    ] {
        for request in [
            copy(&root.path().join("src"), &["tree"], &root.path().join(target)),
            relocate(&root.path().join("src"), &["tree"], &root.path().join(target)),
        ] {
            let ended = rig.run(request);

            let Outcome::Failed { error } = ended.outcome else {
                panic!("{target}: {:?}", ended.outcome);
            };
            assert_eq!(error.code, IpcErrorCode::InvalidInput, "{target}");
            assert_eq!(tree(root.path()), before, "{target}: something was written");
        }
    }
}

#[test]
fn a_folder_whose_identity_cannot_be_read_is_refused_before_anything_is_written() {
    let root = temp();
    write(&root.path().join("src/tree/a.txt"), b"a");
    let mut rig = Rig::hooked(|hooks| hooks.unreadable_identity = true);
    let before = tree(root.path());

    for request in [
        copy(&root.path().join("src"), &["tree"], &root.path().join("dst")),
        relocate(&root.path().join("src"), &["tree"], &root.path().join("dst")),
    ] {
        let ended = rig.run(request);

        assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
        assert_eq!(tree(root.path()), before, "something was written");
    }
}

#[test]
fn one_bad_item_among_several_refuses_the_whole_job() {
    let root = temp();
    write(&root.path().join("src/file.txt"), b"x");
    fs::create_dir_all(root.path().join("src/tree/inside")).expect("folder");
    let mut rig = Rig::new();

    let ended = rig.run(copy(
        &root.path().join("src"),
        &["file.txt", "tree"],
        &root.path().join("src/tree/inside"),
    ));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }));
    assert_eq!(tree(&root.path().join("src/tree/inside")), Vec::<String>::new());
}

#[test]
fn a_sibling_whose_name_starts_the_same_is_not_inside() {
    let root = temp();
    write(&root.path().join("src/tree/a.txt"), b"a");
    fs::create_dir_all(root.path().join("src/tree-copy")).expect("folder");
    let mut rig = Rig::new();

    let ended = rig.run(copy(
        &root.path().join("src"),
        &["tree"],
        &root.path().join("src/tree-copy"),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(tree(&root.path().join("src/tree-copy")), ["tree/", "tree/a.txt"]);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn a_case_variant_of_the_source_is_still_into_itself() {
    let root = temp();
    write(&root.path().join("src/Tree/a.txt"), b"a");
    let mut rig = Rig::new();
    let variant = root.path().join("SRC").join("tree").join("new");

    let ended = rig.run(copy(&root.path().join("src"), &["Tree"], &variant));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(tree(&root.path().join("src")), ["Tree/", "Tree/a.txt"]);
}

#[cfg(unix)]
#[test]
fn a_link_that_leads_into_the_source_folder_is_still_into_itself() {
    let root = temp();
    write(&root.path().join("src/tree/a.txt"), b"a");
    fs::create_dir_all(root.path().join("src/tree/sub")).expect("folder");
    std::os::unix::fs::symlink(root.path().join("src/tree/sub"), root.path().join("shortcut")).expect("link");
    let mut rig = Rig::new();

    let ended = rig.run(copy(&root.path().join("src"), &["tree"], &root.path().join("shortcut")));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(tree(&root.path().join("src/tree/sub")), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn listing_a_folder_never_follows_a_link() {
    let root = temp();
    write(&root.path().join("outside/x"), b"x");
    write(&root.path().join("dir/file"), b"f");
    fs::create_dir_all(root.path().join("dir/sub")).expect("folder");
    std::os::unix::fs::symlink(root.path().join("outside"), root.path().join("dir/dirlink")).expect("link");
    std::os::unix::fs::symlink(root.path().join("outside/x"), root.path().join("dir/filelink")).expect("link");

    let listing = read_children(&root.path().join("dir")).expect("lists");

    let mut kinds: Vec<(String, Kind)> = listing.children.iter().map(|child| (child.rel(), child.kind)).collect();
    kinds.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        kinds,
        [
            ("dirlink".to_owned(), Kind::Link),
            ("file".to_owned(), Kind::File),
            ("filelink".to_owned(), Kind::Link),
            ("sub".to_owned(), Kind::Dir),
        ]
    );
    assert!(listing.failed.is_empty());
}

#[test]
fn a_folder_that_cannot_be_listed_is_an_error() {
    let root = temp();

    assert!(read_children(&root.path().join("missing")).is_err());
}

#[test]
fn a_target_reached_by_another_spelling_of_the_source_is_in_itself_by_identity_not_by_name() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("tree/a.txt"), b"a");
    fs::create_dir_all(src.join("tree/sub")).expect("folder");
    fs::create_dir_all(src.join("other")).expect("folder");
    let item = Child::lookup(&src, "tree").expect("the item");
    let missing = Path::new("new");
    let none = Path::new("");
    let id = FileId::of_path(&item.path).expect("an identity");
    let lands = |existing: &Path, missing: &Path| lands_in_itself(&item, &id, existing, missing);

    // Each of these paths leads to a folder in or at the item, or to the item's own place, by a route that no
    // comparison of names sees through.
    let inside = src.join("other").join("..").join("tree").join("sub");
    let itself = src.join("other").join("..").join("tree");
    let own_place = src.join("other").join("..");
    assert!(!is_inside(&inside, &src.join("tree")));
    assert!(lands(&inside, missing));
    assert!(lands(&inside, none));
    assert!(lands(&itself, none));
    assert!(lands(&own_place, none));

    // A neighbour is not.
    assert!(!lands(&src.join("other"), missing));
    assert!(!lands(&src.join("other").join("..").join("other"), missing));
    assert!(!lands(root.path(), Path::new("dst")));
}

#[cfg(windows)]
mod windows {
    use pretty_assertions::assert_eq;

    use super::super::testing::other_spelling;
    use super::*;

    const FOLDER: &str = "A Long Folder Name";

    /// `src/A Long Folder Name/a.txt` under a temporary folder, and the other spelling of the temporary folder;
    /// `None` when it has only one.
    fn scene() -> Option<(tempfile::TempDir, std::path::PathBuf)> {
        let root = temp();
        write(&root.path().join("src").join(FOLDER).join("a.txt"), b"a");

        other_spelling(root.path()).map(|other| (root, other))
    }

    #[test]
    fn another_spelling_of_the_source_is_still_into_itself() {
        let Some((root, other)) = scene() else { return };
        let mut rig = Rig::new();
        let before = tree(root.path());

        for target in [
            other.join("src").join(FOLDER),
            other.join("src").join(FOLDER).join("new"),
        ] {
            for request in [
                copy(&root.path().join("src"), &[FOLDER], &target),
                relocate(&root.path().join("src"), &[FOLDER], &target),
            ] {
                let ended = rig.run(request);

                assert!(
                    matches!(ended.outcome, Outcome::Failed { .. }),
                    "{target:?}: {:?}",
                    ended.outcome
                );
                assert_eq!(tree(root.path()), before, "{target:?}");
            }
        }
    }

    #[test]
    fn a_folder_moved_into_another_spelling_of_its_own_folder_is_refused_and_loses_nothing() {
        let Some((root, other)) = scene() else { return };
        let mut rig = Rig::new();

        let ended = rig.run(relocate(&root.path().join("src"), &[FOLDER], &other.join("src")));

        assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
        assert_eq!(
            tree(&root.path().join("src")),
            [format!("{FOLDER}/"), format!("{FOLDER}/a.txt")]
        );
    }
}

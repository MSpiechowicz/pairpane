//! Reaching a folder below another one name by name, and checking it is the folder an operation acts on.

use std::fs;

use super::{check_folder_below, check_same_folder_below};

fn tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a folder");
    fs::create_dir_all(root.path().join("a/b")).expect("folders");
    fs::write(root.path().join("a/file"), b"x").expect("file");

    root
}

#[test]
fn a_folder_is_reached_through_folders_only_and_the_base_is_the_empty_name() {
    let root = tree();

    for dir in ["", "a", "a/b"] {
        check_folder_below(root.path(), dir).unwrap_or_else(|error| panic!("{dir:?}: {error}"));
    }

    for dir in [
        "missing",
        "a/missing",
        "a/file",
        "a/file/x",
        "..",
        "a/../a",
        "a//b",
        "/a",
    ] {
        assert!(check_folder_below(root.path(), dir).is_err(), "{dir:?}");
    }
}

#[test]
fn the_folder_reached_must_be_the_one_the_operation_acts_on() {
    let root = tree();
    let other = tempfile::tempdir().expect("a folder");

    check_same_folder_below(root.path(), "a/b", &root.path().join("a/b")).expect("the same folder");
    check_same_folder_below(root.path(), "", root.path()).expect("the base itself");

    assert!(check_same_folder_below(root.path(), "a/b", &root.path().join("a")).is_err());
    assert!(check_same_folder_below(root.path(), "a", other.path()).is_err());
    assert!(check_same_folder_below(root.path(), "a/b", &root.path().join("gone")).is_err());
}

#[cfg(unix)]
mod links {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn a_link_on_the_way_is_refused_even_when_it_leads_to_the_folder() {
        let root = tree();
        fs::rename(root.path().join("a"), root.path().join("real")).expect("rename");
        symlink(root.path().join("real"), root.path().join("a")).expect("link");

        assert!(check_folder_below(root.path(), "a").is_err());
        assert!(check_folder_below(root.path(), "a/b").is_err());
        assert!(check_same_folder_below(root.path(), "a/b", &root.path().join("real/b")).is_err());
        check_folder_below(root.path(), "real/b").expect("the folder itself");
    }

    #[test]
    fn a_folder_swapped_for_a_link_to_another_folder_is_refused_though_the_path_still_resolves() {
        let root = tree();
        let elsewhere = tempfile::tempdir().expect("a folder");
        fs::create_dir(elsewhere.path().join("b")).expect("folder");
        let target = root.path().join("a/b");

        fs::remove_dir_all(root.path().join("a")).expect("remove");
        symlink(elsewhere.path(), root.path().join("a")).expect("link");

        assert!(target.is_dir(), "the path now leads into the other folder");
        assert!(check_same_folder_below(root.path(), "a/b", &target).is_err());
    }

    #[test]
    fn a_target_that_is_a_link_to_the_folder_is_the_folder() {
        let root = tree();
        let holder = tempfile::tempdir().expect("a folder");
        let link = holder.path().join("link");
        symlink(root.path().join("a"), &link).expect("link");

        check_same_folder_below(root.path(), "a", &link).expect("a pane may show the folder by a link");
    }
}

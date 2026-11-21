//! The properties of items on real files.

use std::fs;
use std::io;

use pretty_assertions::assert_eq;

use crate::model::EntryKind;

use super::properties;
use super::testing::{long_ago, put, set_times, temp};

#[test]
fn a_file_shows_its_size_and_times() {
    let root = temp();
    put(root.path(), "a.txt", b"hello");
    set_times(&root.path().join("a.txt"), long_ago());

    let shown = properties(root.path(), "a.txt").expect("properties");

    assert_eq!(shown.name, "a.txt");
    assert_eq!(shown.kind, EntryKind::File);
    assert_eq!(shown.size, 5);
    assert_eq!(shown.modified, Some(1_000_000_000_000));
    assert_eq!(shown.accessed, Some(1_000_000_000_000));
    assert_eq!(shown.link_target, None);
}

#[test]
fn a_folder_has_no_size_of_its_own() {
    let root = temp();
    put(root.path(), "d/inner", b"hello");

    let shown = properties(root.path(), "d").expect("properties");

    assert_eq!(shown.kind, EntryKind::Dir);
    assert_eq!(shown.size, 0);
    assert!(shown.modified.is_some());
}

#[test]
fn a_name_that_is_not_plain_or_not_there_is_refused() {
    let root = temp();

    for name in ["", ".", "..", "a/b", "../x"] {
        let error = properties(root.path(), name).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        assert!(!error.to_string().contains(&*root.path().to_string_lossy()));
    }
    assert_eq!(
        properties(root.path(), "gone").expect_err("missing").kind(),
        io::ErrorKind::NotFound
    );
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn the_permissions_and_the_owner_are_the_files_own() {
        let root = temp();
        put(root.path(), "a.txt", b"x");
        fs::set_permissions(root.path().join("a.txt"), fs::Permissions::from_mode(0o640)).expect("mode");

        let shown = properties(root.path(), "a.txt").expect("properties");

        assert_eq!(shown.mode, Some(0o640));
        assert_eq!(
            shown.owner,
            uzers::get_current_username().map(|name| name.to_string_lossy().into_owned())
        );
        assert_eq!(
            shown.group,
            uzers::get_current_groupname().map(|name| name.to_string_lossy().into_owned())
        );
        assert_eq!(shown.windows, None);
        assert!(shown.created.is_none_or(|created| created > 1_000_000_000_000));
    }

    #[test]
    fn a_link_shows_what_it_leads_to_and_where_it_points() {
        let root = temp();
        put(root.path(), "real.txt", b"12345");
        put(root.path(), "real-dir/x", b"x");
        symlink(root.path().join("real.txt"), root.path().join("to-file")).expect("link");
        symlink(root.path().join("real-dir"), root.path().join("to-dir")).expect("link");
        symlink("missing", root.path().join("broken")).expect("link");

        let file = properties(root.path(), "to-file").expect("properties");
        let dir = properties(root.path(), "to-dir").expect("properties");
        let broken = properties(root.path(), "broken").expect("properties");

        assert_eq!((file.kind, file.size), (EntryKind::File, 5));
        assert_eq!(
            file.link_target,
            Some(root.path().join("real.txt").to_string_lossy().into_owned())
        );
        assert_eq!((dir.kind, dir.size), (EntryKind::Dir, 0));
        assert_eq!((broken.kind, broken.size), (EntryKind::Other, 0));
        assert_eq!(broken.link_target.as_deref(), Some("missing"));
    }
}

#[cfg(windows)]
#[test]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "the Windows read-only attribute; there is no world-writable bit to set"
)]
fn the_windows_attributes_are_shown_and_there_is_no_owner() {
    let root = temp();
    put(root.path(), "a.txt", b"x");
    let path = root.path().join("a.txt");
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).expect("attribute");

    let shown = properties(root.path(), "a.txt").expect("properties");

    let attributes = shown.windows.expect("windows attributes");
    assert!(attributes.read_only);
    assert!(!attributes.hidden);
    assert_eq!((shown.mode, shown.owner, shown.group), (None, None, None));

    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).expect("writable again");
}

#[cfg(not(windows))]
#[test]
fn there_are_no_windows_attributes_elsewhere() {
    let root = temp();
    put(root.path(), "a.txt", b"x");

    assert_eq!(properties(root.path(), "a.txt").expect("properties").windows, None);
}

//! Tests of the permanent delete.

use std::fs;
use std::io;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[cfg(unix)]
use super::delete::{is_mount, refuse_mounts};
use super::delete_permanently;
use super::items_tests::{read, symlink, write};

#[test]
fn delete_permanently_removes_files_and_folders_with_their_contents() {
    let temp = TempDir::new().expect("temp dir");
    write(&temp.path().join("a.txt"), "a");
    fs::create_dir_all(temp.path().join("tree/deep/deeper")).expect("tree");
    write(&temp.path().join("tree/deep/deeper/f.txt"), "f");
    write(&temp.path().join("keep.txt"), "kept");

    let names = ["a.txt".to_owned(), "tree".to_owned()];
    delete_permanently(temp.path(), &names).expect("deleted");

    assert!(!temp.path().join("a.txt").exists());
    assert!(!temp.path().join("tree").exists());
    assert_eq!(read(&temp.path().join("keep.txt")), "kept");
}

#[test]
fn delete_permanently_removes_a_link_to_a_folder_and_never_what_it_leads_to() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target");
    fs::create_dir(&target).expect("folder");
    write(&target.join("inner.txt"), "inner");
    if !symlink(&target, &temp.path().join("link")) {
        return;
    }

    delete_permanently(temp.path(), &["link".to_owned()]).expect("deleted");

    assert!(fs::symlink_metadata(temp.path().join("link")).is_err());
    assert_eq!(read(&target.join("inner.txt")), "inner");
}

#[test]
fn delete_permanently_removes_a_link_to_a_file_and_a_dangling_link() {
    let temp = TempDir::new().expect("temp dir");
    let target = temp.path().join("target.txt");
    write(&target, "kept");
    if !symlink(&target, &temp.path().join("soft")) {
        return;
    }
    let missing = temp.path().join("missing");
    assert!(symlink(&missing, &temp.path().join("dangling")));

    delete_permanently(temp.path(), &["soft".to_owned(), "dangling".to_owned()]).expect("deleted");

    assert!(fs::symlink_metadata(temp.path().join("soft")).is_err());
    assert!(fs::symlink_metadata(temp.path().join("dangling")).is_err());
    assert_eq!(read(&target), "kept");
}

#[test]
fn delete_permanently_stops_at_a_missing_item_and_names_it() {
    let temp = TempDir::new().expect("temp dir");
    write(&temp.path().join("first"), "1");
    write(&temp.path().join("last"), "3");

    let names = ["first".to_owned(), "missing".to_owned(), "last".to_owned()];
    let failure = delete_permanently(temp.path(), &names).expect_err("stopped");

    assert_eq!(failure.name, "missing");
    assert_eq!(failure.error.kind(), io::ErrorKind::NotFound);
    assert!(!temp.path().join("first").exists());
    assert_eq!(read(&temp.path().join("last")), "3");
}

#[test]
fn delete_permanently_refuses_a_name_that_is_not_plain_before_touching_anything() {
    let temp = TempDir::new().expect("temp dir");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).expect("folder");
    write(&temp.path().join("sibling.txt"), "kept");

    for bad in ["..", ".", "", "../sibling.txt"] {
        let failure = delete_permanently(&folder, &[bad.to_owned()]).expect_err(bad);

        assert_eq!(failure.error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        assert_eq!(failure.name, bad);
    }
    assert!(folder.is_dir());
    assert_eq!(read(&temp.path().join("sibling.txt")), "kept");
}

/// A tree `root/a/b/c` with a file in each folder.
#[cfg(unix)]
fn tree(temp: &TempDir) -> std::path::PathBuf {
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("a/b/c")).expect("tree");
    for folder in ["a", "a/b", "a/b/c"] {
        write(&root.join(folder).join("f.txt"), "f");
    }

    root
}

#[cfg(unix)]
#[test]
fn a_tree_with_no_mount_in_it_passes_the_check() {
    let temp = TempDir::new().expect("temp dir");
    let root = tree(&temp);

    assert!(refuse_mounts(&root, &|_, _| Ok(false)).is_ok());
}

#[cfg(unix)]
#[test]
fn a_mount_anywhere_in_the_tree_refuses_it_with_a_fixed_text_that_holds_no_path() {
    let temp = TempDir::new().expect("temp dir");
    let root = tree(&temp);

    for mount in ["root", "root/a", "root/a/b/c"] {
        let error = refuse_mounts(&root, &|folder, _| Ok(folder.ends_with(mount))).expect_err(mount);

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{mount}");
        assert_eq!(
            error.to_string(),
            "contains a mounted drive or volume; unmount or delete it separately"
        );
    }
}

#[cfg(unix)]
#[test]
fn the_check_never_reads_inside_a_mount_and_gives_every_folder_the_device_of_the_outer_folder() {
    use std::cell::RefCell;
    use std::os::unix::fs::MetadataExt;

    let temp = TempDir::new().expect("temp dir");
    let root = tree(&temp);
    let outer = fs::metadata(temp.path()).expect("outer").dev();
    let seen = RefCell::new(Vec::new());

    let result = refuse_mounts(&root, &|folder, device| {
        assert_eq!(device, outer);
        seen.borrow_mut().push(folder.to_path_buf());

        Ok(folder.ends_with("a/b"))
    });

    assert!(result.is_err());
    assert!(seen.borrow().iter().any(|folder| folder.ends_with("a/b")));
    assert!(
        !seen.borrow().iter().any(|folder| folder.ends_with("a/b/c")),
        "{:?}",
        seen.borrow()
    );
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_checked_fails_the_check() {
    let temp = TempDir::new().expect("temp dir");
    let root = tree(&temp);

    let error = refuse_mounts(&root, &|folder, _| {
        if folder.ends_with("a/b") {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }

        Ok(false)
    })
    .expect_err("fails");

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_is_not_walked_so_a_mount_behind_it_does_not_count() {
    let temp = TempDir::new().expect("temp dir");
    let root = tree(&temp);
    let outside = temp.path().join("outside");
    fs::create_dir_all(outside.join("inner")).expect("outside");
    if !symlink(&outside, &root.join("a/link")) {
        return;
    }

    let result = refuse_mounts(&root, &|folder, _| Ok(folder.starts_with(&outside)));

    assert!(result.is_ok());
    delete_permanently(temp.path(), &["root".to_owned()]).expect("deleted");
    assert!(outside.join("inner").is_dir());
}

#[cfg(unix)]
#[test]
fn a_plain_folder_is_not_a_mount_and_a_folder_on_another_device_is() {
    use std::os::unix::fs::MetadataExt;

    let temp = TempDir::new().expect("temp dir");
    let root = tree(&temp);
    let device = fs::metadata(&root).expect("metadata").dev();

    assert!(!is_mount(&root, device).expect("plain"));
    assert!(is_mount(&root, device.wrapping_add(1)).expect("other device"));
    assert_eq!(
        is_mount(&root.join("missing"), device).expect_err("gone").kind(),
        io::ErrorKind::NotFound
    );
}

/// The top of a mount on the same device as the folder around it (a bind mount) has the same `st_dev`: only the
/// kernel's mount-root flag tells it, so the root of a drive is checked against its own device.
#[cfg(target_os = "linux")]
#[test]
fn the_top_of_a_mount_is_a_mount_even_with_the_device_of_its_outer_folder() {
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    let slash = Path::new("/");
    let device = fs::metadata(slash).expect("root").dev();

    assert!(is_mount(slash, device).expect("the root of the system is a mount root"));
}

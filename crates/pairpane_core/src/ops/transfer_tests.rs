//! Whole copy jobs on real folders: what a copy keeps, links, and special files.

use std::fs;
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;

#[cfg(unix)]
use crate::model::OpsEvent;
use crate::model::Outcome;

use super::testing::{
    Rig, copy, has_no_partial_files, long_ago, modified, pattern, read, set_times, temp, tree, write,
};

/// Copies the folder `tree` of `root/src` into `root/dst`, which must go without a single error. Returns the
/// rig (for its events) and the copy.
fn copy_tree(root: &Path) -> (Rig, PathBuf) {
    let mut rig = Rig::new();

    let ended = rig.run_copy(root, &["tree"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);

    (rig, root.join("dst/tree"))
}

#[test]
fn a_copied_file_keeps_its_bytes_and_times() {
    let root = temp();
    let source = root.path().join("src/a.bin");
    write(&source, &pattern(3 * 1024 * 1024 + 17));
    set_times(&source, long_ago());
    let mut rig = Rig::new();

    let ended = rig.run(copy(&root.path().join("src"), &["a.bin"], &root.path().join("dst")));

    // The times first: reading the file below may update its access time.
    let copied = root.path().join("dst/a.bin");
    let times = fs::metadata(&copied).expect("metadata");
    assert_eq!(times.modified().expect("modified"), long_ago());
    assert_eq!(times.accessed().expect("accessed"), long_ago());
    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(read(&copied), read(&source));
    assert_eq!(modified(&source), long_ago(), "the source is left as it was");
    assert!(has_no_partial_files(root.path()));
}

#[cfg(unix)]
#[test]
fn a_copied_file_keeps_its_permissions_without_setuid() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp();
    let source = root.path().join("src/a.sh");
    write(&source, b"not a script\n");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o4750)).expect("mode");
    let mut rig = Rig::new();

    rig.run(copy(&root.path().join("src"), &["a.sh"], &root.path().join("dst")));

    let mode = fs::metadata(root.path().join("dst/a.sh"))
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o750);
}

#[cfg(windows)]
#[test]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "the Windows read-only attribute; there is no world-writable bit to set"
)]
fn a_copied_file_keeps_the_read_only_attribute() {
    let root = temp();
    let source = root.path().join("src/a.txt");
    write(&source, b"x");
    let mut permissions = fs::metadata(&source).expect("metadata").permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions).expect("read-only");
    let mut rig = Rig::new();

    rig.run(copy(&root.path().join("src"), &["a.txt"], &root.path().join("dst")));

    let copied = root.path().join("dst/a.txt");
    assert!(fs::metadata(&copied).expect("metadata").permissions().readonly());
    for path in [&source, &copied] {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).expect("writable again");
    }
}

#[test]
fn a_copied_tree_keeps_nested_and_empty_folders_and_their_times() {
    let root = temp();
    let src = root.path().join("src/tree");
    write(&src.join("a.txt"), b"a");
    write(&src.join("sub/b.txt"), b"b");
    write(&src.join("sub/deep/c.txt"), b"c");
    fs::create_dir_all(src.join("empty")).expect("folder");
    for folder in ["sub/deep", "sub", "empty", ""] {
        set_times(&src.join(folder), long_ago());
    }

    let (_rig, copied) = copy_tree(root.path());

    assert_eq!(tree(&copied), tree(&src));
    assert_eq!(read(&copied.join("sub/deep/c.txt")), b"c");
    for folder in ["", "sub", "sub/deep", "empty"] {
        assert_eq!(modified(&copied.join(folder)), long_ago(), "{folder:?}");
    }
    assert!(has_no_partial_files(root.path()));
}

#[test]
fn the_target_folder_is_made_when_missing_and_items_that_are_gone_are_reported() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"a");
    let mut rig = Rig::new();

    let ended = rig.run(copy(
        &root.path().join("src"),
        &["a.txt", "gone.txt"],
        &root.path().join("new/deeper/dst"),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(read(&root.path().join("new/deeper/dst/a.txt")), b"a");
    assert_eq!(ended.errors.len(), 1);
    assert_eq!(ended.errors[0].path, "gone.txt");
}

#[test]
fn a_target_that_is_a_file_fails_the_job() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"a");
    write(&root.path().join("file"), b"in the way");
    let mut rig = Rig::new();

    let ended = rig.run(copy(&root.path().join("src"), &["a.txt"], &root.path().join("file")));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(read(&root.path().join("file")), b"in the way");
}

#[cfg(unix)]
#[test]
fn links_are_copied_as_links_and_never_followed() {
    use std::os::unix::fs::symlink;

    let root = temp();
    let outside = root.path().join("outside");
    write(&outside.join("big"), &pattern(2 * 1024 * 1024));
    let tree_dir = root.path().join("src/tree");
    write(&tree_dir.join("a.txt"), b"a");
    symlink(&outside, tree_dir.join("dirlink")).expect("link");
    symlink(outside.join("big"), tree_dir.join("filelink")).expect("link");
    symlink("missing-target", tree_dir.join("dangling")).expect("link");
    symlink(&tree_dir, tree_dir.join("loop")).expect("link");

    let (rig, copied) = copy_tree(root.path());

    assert_eq!(tree(&copied), ["a.txt", "dangling", "dirlink", "filelink", "loop"]);
    for (name, target) in [
        ("dirlink", outside.as_path()),
        ("filelink", &outside.join("big")),
        ("dangling", Path::new("missing-target")),
        ("loop", &tree_dir),
    ] {
        assert_eq!(fs::read_link(copied.join(name)).expect("a link"), target, "{name}");
    }
    let last_progress = rig.seen.iter().rev().find_map(|event| match event {
        OpsEvent::Progress { total_bytes, .. } => Some(*total_bytes),
        _ => None,
    });
    assert_eq!(
        last_progress,
        Some(1),
        "only the 1 byte of a.txt is copied, never what the links lead to"
    );
}

#[cfg(unix)]
#[test]
fn a_link_named_as_an_item_is_copied_as_a_link() {
    use std::os::unix::fs::symlink;

    let root = temp();
    write(&root.path().join("outside/big"), b"data");
    fs::create_dir_all(root.path().join("src")).expect("folder");
    symlink(root.path().join("outside"), root.path().join("src/link")).expect("link");
    let mut rig = Rig::new();

    let ended = rig.run(copy(&root.path().join("src"), &["link"], &root.path().join("dst")));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(
        fs::read_link(root.path().join("dst/link")).expect("a link"),
        root.path().join("outside")
    );
    assert_eq!(tree(&root.path().join("dst")), ["link"]);
}

#[cfg(windows)]
#[test]
fn a_link_to_a_network_path_is_copied_as_a_link_and_never_opened() {
    use crate::listing::platform::link_tests::{REMOTES, network_chain};

    for remote in REMOTES {
        let root = temp();
        let tree_dir = root.path().join("src/tree");
        write(&tree_dir.join("a.txt"), b"a");
        let Some((first, _)) = network_chain(&tree_dir, remote) else {
            return;
        };

        let (_rig, copied) = copy_tree(root.path());

        assert!(
            fs::symlink_metadata(copied.join("a")).expect("copied").is_symlink(),
            "{remote}"
        );
        assert_eq!(
            fs::read_link(copied.join("a")).expect("a link"),
            fs::read_link(&first).expect("the original")
        );
    }
}

#[cfg(unix)]
#[test]
fn a_pipe_is_skipped_and_never_opened() {
    use super::testing::make_fifo;

    let root = temp();
    let tree_dir = root.path().join("src/tree");
    write(&tree_dir.join("a.txt"), b"a");
    make_fifo(&tree_dir.join("pipe"));
    make_fifo(&root.path().join("src/toplevel"));
    let mut rig = Rig::new();

    // A worker that opened the pipe would wait for a writer for good, and the rig gives up after 30 seconds.
    let ended = rig.run(copy(
        &root.path().join("src"),
        &["tree", "toplevel"],
        &root.path().join("dst"),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.skipped, 2);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(tree(&root.path().join("dst")), ["tree/", "tree/a.txt"]);
}

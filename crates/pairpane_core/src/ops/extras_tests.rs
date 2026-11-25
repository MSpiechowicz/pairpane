//! A move across disks takes the file's extended attributes (alternate streams on Windows) with it, or keeps the
//! source and says what could not go (#73).

use pretty_assertions::assert_eq;

use crate::model::Outcome;

use super::testing::{Rig, read, temp, tree, write};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod attributes {
    use std::path::Path;

    use rustix::fs::{XattrFlags, getxattr, setxattr};

    pub(super) const NAME: &str = "user.pairpane.test";

    pub(super) fn mark(path: &Path, value: &[u8]) {
        setxattr(path, NAME, value, XattrFlags::empty()).expect("the test folder's disk holds extended attributes");
    }

    pub(super) fn read_mark(path: &Path) -> Option<Vec<u8>> {
        let mut buffer = vec![0; 256];
        let length = getxattr(path, NAME, &mut buffer[..]).ok()?;
        buffer.truncate(length);
        Some(buffer)
    }
}

#[cfg(windows)]
mod attributes {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    pub(super) const NAME: &str = "pairpane.test";

    fn stream(path: &Path) -> OsString {
        let mut named = path.as_os_str().to_owned();
        named.push(format!(":{NAME}"));
        named
    }

    pub(super) fn mark(path: &Path, value: &[u8]) {
        fs::write(stream(path), value).expect("the test folder's disk holds streams");
    }

    pub(super) fn read_mark(path: &Path) -> Option<Vec<u8>> {
        fs::read(stream(path)).ok()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn a_move_across_disks_takes_the_file_s_extended_attributes_along() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"a");
    write(&src.join("folder/b.txt"), b"b");
    attributes::mark(&src.join("a.txt"), b"tag-a");
    attributes::mark(&src.join("folder/b.txt"), b"tag-b");
    let mut rig = Rig::hooked(|hooks| hooks.cross_device = true);

    let ended = rig.run_move(root.path(), &["a.txt", "folder"]);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    let dst = root.path().join("dst");
    assert_eq!(
        attributes::read_mark(&dst.join("a.txt")).as_deref(),
        Some(&b"tag-a"[..])
    );
    assert_eq!(
        attributes::read_mark(&dst.join("folder/b.txt")).as_deref(),
        Some(&b"tag-b"[..])
    );
    assert_eq!(tree(&src), Vec::<String>::new());
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn a_move_to_a_disk_that_cannot_hold_them_keeps_the_original() {
    let root = temp();
    let src = root.path().join("src");
    write(&src.join("a.txt"), b"a");
    attributes::mark(&src.join("a.txt"), b"tag-a");
    let mut rig = Rig::hooked(|hooks| {
        hooks.cross_device = true;
        hooks.refuse_extras = true;
    });

    let ended = rig.run_move(root.path(), &["a.txt"]);

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert!(
        ended.errors[0]
            .error
            .message
            .starts_with("Kept the original: the target cannot hold its "),
        "{}",
        ended.errors[0].error.message
    );
    assert_eq!(read(&src.join("a.txt")), b"a");
    assert_eq!(
        attributes::read_mark(&src.join("a.txt")).as_deref(),
        Some(&b"tag-a"[..])
    );
    assert_eq!(tree(&root.path().join("dst")), Vec::<String>::new());
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn a_copy_to_a_disk_that_cannot_hold_them_still_copies() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"a");
    attributes::mark(&root.path().join("src/a.txt"), b"tag-a");
    let mut rig = Rig::hooked(|hooks| hooks.refuse_extras = true);

    let ended = rig.run_copy(root.path(), &["a.txt"]);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"a");
}

#[test]
fn a_plain_file_moves_across_disks_as_before() {
    let root = temp();
    write(&root.path().join("src/a.txt"), b"a");
    let mut rig = Rig::hooked(|hooks| hooks.cross_device = true);

    let ended = rig.run_move(root.path(), &["a.txt"]);

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(read(&root.path().join("dst/a.txt")), b"a");
}

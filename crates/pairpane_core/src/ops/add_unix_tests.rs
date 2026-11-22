//! Add jobs and the permissions of the new archive (Unix): the mode and the group of the old one are kept, through the open
//! file and not its path.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, chown, symlink};
use std::sync::Arc;

use rustix::process::{getegid, getgroups};

use crate::model::{ArchiveFormat, Outcome};

use super::add_tests::{add, knows, scene_with};
use super::hooks::Hooks;
use super::testing::Rig;

/// The scene with an `a.zip` that has the mode `mode`.
fn scene_of_mode(mode: u32) -> tempfile::TempDir {
    let root = scene_with("a.zip", ArchiveFormat::Zip, None, false);
    fs::set_permissions(root.path().join("dst/a.zip"), fs::Permissions::from_mode(mode)).expect("mode");

    root
}

/// The mode and the group of the archive.
fn mode_and_group(root: &tempfile::TempDir) -> (u32, u32) {
    let meta = fs::metadata(root.path().join("dst/a.zip")).expect("metadata");

    (meta.permissions().mode() & 0o777, meta.gid())
}

#[test]
fn the_group_and_the_mode_of_the_archive_are_kept() {
    // Another group of the user, when the user is in one: a file may be given to it, and the new archive must be too.
    let own = getegid().as_raw();
    let group = getgroups()
        .expect("groups")
        .into_iter()
        .map(|group| group.as_raw())
        .find(|group| *group != own)
        .unwrap_or(own);
    let root = scene_of_mode(0o660);
    chown(root.path().join("dst/a.zip"), None, Some(group)).expect("a group of the user");

    let ended = Rig::new().run(add(root.path(), &["new.txt"], "a.zip", "", knows(None)));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(mode_and_group(&root), (0o660, group));
}

#[test]
fn a_group_that_cannot_be_given_to_the_new_archive_takes_its_bits_from_the_mode() {
    let root = scene_of_mode(0o664);
    let hooks = Hooks {
        refuse_group: true,
        ..Hooks::default()
    };

    let ended = Rig::with(hooks, |_, _| {}).run(add(root.path(), &["new.txt"], "a.zip", "", knows(None)));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(mode_and_group(&root).0, 0o604);
}

#[test]
fn a_temporary_file_swapped_for_a_link_does_not_get_the_mode_of_the_archive() {
    let root = scene_of_mode(0o640);
    let victim = root.path().join("victim.txt");
    fs::write(&victim, b"not part of the job").expect("a file");
    fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).expect("mode");
    let target = victim.clone();
    let hooks = Hooks {
        after_temp: Some(Arc::new(move |temp| {
            fs::rename(temp, temp.with_extension("aside")).expect("moved aside");
            symlink(&target, temp).expect("a link");
        })),
        ..Hooks::default()
    };

    Rig::with(hooks, |_, _| {}).run(add(root.path(), &["new.txt"], "a.zip", "", knows(None)));

    assert_eq!(
        fs::metadata(&victim).expect("metadata").permissions().mode() & 0o777,
        0o600
    );
}

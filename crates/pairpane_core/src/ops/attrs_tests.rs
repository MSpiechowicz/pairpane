//! Changing permissions and times through jobs, on real files. Nothing is changed through a link.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use pretty_assertions::assert_eq;

use crate::model::{AttributeChange, ModeChange, Outcome};

use super::testing::{Ended, Rig, long_ago, modified, put, set_times, task, temp};
use super::{Refused, Task};

/// A change that does nothing yet: tests set the fields they need.
pub(super) fn nothing() -> AttributeChange {
    AttributeChange {
        mode: None,
        read_only: None,
        hidden: None,
        system: None,
        archive: None,
        modified: None,
        accessed: None,
        recursive: false,
    }
}

fn at(millis: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_millis(millis)
}

/// Runs `change` on the `names` of `root/src`, to its end.
pub(super) fn run(root: &Path, names: &[&str], change: AttributeChange) -> Ended {
    let src = root.join("src");

    Rig::new().run(task(&src, names, &src, Task::Attributes(change)))
}

#[test]
fn the_times_in_milliseconds_are_what_the_files_get() {
    let root = temp();
    put(root.path(), "src/f", b"x");

    let ended = run(
        root.path(),
        &["f"],
        AttributeChange {
            modified: Some(1_500_000_000_123),
            accessed: Some(1_400_000_000_000),
            ..nothing()
        },
    );

    assert_eq!(ended.outcome, Outcome::Done);
    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    let meta = fs::metadata(root.path().join("src/f")).expect("metadata");
    assert_eq!(meta.modified().expect("modified"), at(1_500_000_000_123));
    assert_eq!(meta.accessed().expect("accessed"), at(1_400_000_000_000));
}

#[test]
fn a_change_that_sets_no_permissions_never_closes_a_folder() {
    use super::attrs_apply::stays_open;

    assert!(stays_open(&nothing(), 0o000));
    assert!(stays_open(&nothing(), 0o755));
}

#[test]
fn a_time_that_is_not_given_is_left_alone() {
    let root = temp();
    put(root.path(), "src/f", b"x");
    set_times(&root.path().join("src/f"), long_ago());

    run(
        root.path(),
        &["f"],
        AttributeChange {
            modified: Some(1_500_000_000_000),
            ..nothing()
        },
    );

    let meta = fs::metadata(root.path().join("src/f")).expect("metadata");
    assert_eq!(meta.modified().expect("modified"), at(1_500_000_000_000));
    assert_eq!(meta.accessed().expect("accessed"), long_ago());
}

#[test]
fn times_before_the_epoch_can_be_set() {
    let root = temp();
    put(root.path(), "src/f", b"x");

    let ended = run(
        root.path(),
        &["f"],
        AttributeChange {
            modified: Some(-86_400_000),
            ..nothing()
        },
    );

    assert!(ended.errors.is_empty(), "{:?}", ended.errors);
    assert_eq!(
        modified(&root.path().join("src/f")),
        SystemTime::UNIX_EPOCH - Duration::from_secs(86_400)
    );
}

#[test]
fn a_folder_takes_the_times_and_its_contents_only_when_the_change_is_recursive() {
    let root = temp();
    put(root.path(), "src/d/f", b"x");
    put(root.path(), "src/d/sub/g", b"x");
    let change = |recursive| AttributeChange {
        modified: Some(1_500_000_000_000),
        recursive,
        ..nothing()
    };

    run(root.path(), &["d"], change(false));
    assert_eq!(modified(&root.path().join("src/d")), at(1_500_000_000_000));
    assert_ne!(modified(&root.path().join("src/d/f")), at(1_500_000_000_000));

    run(root.path(), &["d"], change(true));
    for path in ["src/d", "src/d/f", "src/d/sub", "src/d/sub/g"] {
        assert_eq!(modified(&root.path().join(path)), at(1_500_000_000_000), "{path}");
    }
}

#[test]
fn a_change_of_nothing_is_not_accepted_and_neither_is_the_other_systems_permissions() {
    let rig = Rig::new();
    let root = temp();
    let src = root.path().join("src");
    let refused = |change: AttributeChange| {
        rig.engine
            .submit(task(&src, &["f"], &src, Task::Attributes(change)))
            .expect_err("refused")
    };
    let foreign = if cfg!(windows) {
        vec![AttributeChange {
            mode: Some(ModeChange::Set { mode: 0o644 }),
            ..nothing()
        }]
    } else {
        // Every attribute that Windows has and no other system does.
        let read_only = Some(true);
        let hidden = Some(true);
        let system = Some(false);
        let archive = Some(true);

        vec![
            AttributeChange { read_only, ..nothing() },
            AttributeChange { hidden, ..nothing() },
            AttributeChange { system, ..nothing() },
            AttributeChange { archive, ..nothing() },
        ]
    };

    assert_eq!(refused(nothing()), Refused::InvalidTask);
    for change in foreign {
        assert_eq!(refused(change), Refused::InvalidTask);
    }
    assert_eq!(
        refused(AttributeChange {
            recursive: true,
            ..nothing()
        }),
        Refused::InvalidTask
    );
}

#[test]
fn what_is_both_added_and_removed_is_added() {
    use super::attrs_apply::new_mode;

    let adjust = ModeChange::Adjust {
        add: 0o100,
        remove: 0o170,
    };

    assert_eq!(new_mode(adjust, 0o644), 0o604 | 0o100);
    assert_eq!(new_mode(ModeChange::Set { mode: 0o640 }, 0o777), 0o640);
}

#[test]
fn the_time_conversion_covers_both_sides_of_the_epoch() {
    use super::attrs_apply::system_time;

    assert_eq!(system_time(0), Some(SystemTime::UNIX_EPOCH));
    assert_eq!(system_time(1_500), Some(at(1_500)));
    assert_eq!(
        system_time(-1_500),
        Some(SystemTime::UNIX_EPOCH - Duration::from_millis(1_500))
    );
}

#[cfg(windows)]
mod windows {
    use pretty_assertions::assert_eq;

    use super::*;

    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the Windows read-only attribute; there is no world-writable bit to set"
    )]
    fn set_read_only(path: &Path, read_only: bool) {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(read_only);
        fs::set_permissions(path, permissions).expect("attribute");
    }

    fn read_only(path: &Path) -> bool {
        fs::metadata(path).expect("metadata").permissions().readonly()
    }

    #[test]
    fn the_read_only_attribute_is_set_and_cleared_on_files_and_the_times_go_with_it() {
        let root = temp();
        put(root.path(), "src/f", b"x");
        let file = root.path().join("src/f");

        let ended = run(
            root.path(),
            &["f"],
            AttributeChange {
                read_only: Some(true),
                modified: Some(1_500_000_000_000),
                ..nothing()
            },
        );
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        assert!(read_only(&file));
        assert_eq!(modified(&file), at(1_500_000_000_000));

        // A read-only file takes a change of its attributes.
        run(
            root.path(),
            &["f"],
            AttributeChange {
                read_only: Some(false),
                ..nothing()
            },
        );
        assert!(!read_only(&file));
    }

    fn attributes(path: &Path) -> u32 {
        use std::os::windows::fs::MetadataExt;

        fs::metadata(path).expect("metadata").file_attributes()
    }

    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const ARCHIVE: u32 = 0x20;

    #[test]
    fn hidden_and_system_are_set_and_cleared_on_a_file_and_on_a_folder() {
        let root = temp();
        put(root.path(), "src/f", b"x");
        put(root.path(), "src/d/g", b"x");
        let file = root.path().join("src/f");
        let folder = root.path().join("src/d");

        let ended = run(
            root.path(),
            &["f", "d"],
            AttributeChange {
                hidden: Some(true),
                system: Some(true),
                ..nothing()
            },
        );
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        for path in [&file, &folder] {
            assert_eq!(
                attributes(path) & (HIDDEN | SYSTEM),
                HIDDEN | SYSTEM,
                "{}",
                path.display()
            );
        }
        // The contents of a folder are not touched without `recursive`.
        assert_eq!(attributes(&folder.join("g")) & (HIDDEN | SYSTEM), 0);

        let ended = run(
            root.path(),
            &["f", "d"],
            AttributeChange {
                hidden: Some(false),
                system: Some(false),
                ..nothing()
            },
        );
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        for path in [&file, &folder] {
            assert_eq!(attributes(path) & (HIDDEN | SYSTEM), 0, "{}", path.display());
        }
    }

    #[test]
    fn the_archive_attribute_is_cleared_and_set_and_read_only_goes_with_the_others() {
        let root = temp();
        put(root.path(), "src/f", b"x");
        let file = root.path().join("src/f");

        for archive in [true, false] {
            run(
                root.path(),
                &["f"],
                AttributeChange {
                    archive: Some(archive),
                    ..nothing()
                },
            );
            assert_eq!(attributes(&file) & ARCHIVE != 0, archive);
        }

        let ended = run(
            root.path(),
            &["f"],
            AttributeChange {
                read_only: Some(true),
                hidden: Some(true),
                archive: Some(true),
                ..nothing()
            },
        );
        assert!(ended.errors.is_empty(), "{:?}", ended.errors);
        assert!(read_only(&file));
        assert_eq!(attributes(&file) & (HIDDEN | ARCHIVE), HIDDEN | ARCHIVE);

        // Clearing everything leaves the file with no attribute at all, and a read-only file takes the change.
        run(
            root.path(),
            &["f"],
            AttributeChange {
                read_only: Some(false),
                hidden: Some(false),
                archive: Some(false),
                ..nothing()
            },
        );
        assert_eq!(attributes(&file) & (HIDDEN | SYSTEM | ARCHIVE), 0);
        assert!(!read_only(&file));
    }

    #[test]
    fn hidden_goes_through_a_whole_tree_when_recursive() {
        let root = temp();
        put(root.path(), "src/d/sub/f", b"x");

        run(
            root.path(),
            &["d"],
            AttributeChange {
                hidden: Some(true),
                recursive: true,
                ..nothing()
            },
        );

        for rel in ["src/d", "src/d/sub", "src/d/sub/f"] {
            assert_eq!(attributes(&root.path().join(rel)) & HIDDEN, HIDDEN, "{rel}");
        }
    }

    #[test]
    fn a_folder_is_entered_and_its_files_change_but_the_folder_keeps_its_attribute() {
        let root = temp();
        put(root.path(), "src/d/f", b"x");

        run(
            root.path(),
            &["d"],
            AttributeChange {
                read_only: Some(true),
                recursive: true,
                ..nothing()
            },
        );

        assert!(read_only(&root.path().join("src/d/f")));
        assert!(!read_only(&root.path().join("src/d")));
        set_read_only(&root.path().join("src/d/f"), false);
    }

    #[test]
    fn a_symbolic_link_is_skipped() {
        let root = temp();
        put(root.path(), "src/f", b"x");
        let link = root.path().join("src/link");
        if std::os::windows::fs::symlink_file(root.path().join("src/f"), &link).is_err() {
            // Creating one takes Developer Mode or administrator rights.
            return;
        }

        let ended = run(
            root.path(),
            &["link"],
            AttributeChange {
                read_only: Some(true),
                hidden: Some(true),
                ..nothing()
            },
        );

        assert_eq!(ended.skipped, 1);
        assert!(!read_only(&root.path().join("src/f")));
        assert_eq!(attributes(&root.path().join("src/f")) & HIDDEN, 0);
    }
}

//! Mirror removals that meet links, unreadable folders and the real Trash: nothing outside the compared folders is ever
//! reached, and a removal to the Trash is a move, not a deletion.

use std::fs;
use std::time::SystemTime;

use pretty_assertions::assert_eq;

use crate::model::{EntryKind, Outcome, PaneId, SyncFacts};

use super::SyncRemove;
use super::sync_remove_tests::{millis, mirror, removal, run};
use super::sync_tests::{Pair, file, is_done};
use super::testing::{Rig, read};

/// `right/dir/a.txt` to be removed, and a lookalike `elsewhere/a.txt` beside the two folders that must survive.
#[cfg(unix)]
fn lookalikes() -> (Pair, std::path::PathBuf, SyncRemove) {
    let pair = Pair::new();
    let elsewhere = pair.root.path().join("elsewhere");
    file(&elsewhere.join("a.txt"), b"keep me", 100);
    file(&pair.right().join("dir/a.txt"), b"keep me", 100);
    let seen = removal(&pair.right(), "dir/a.txt", PaneId::Right);

    (pair, elsewhere, seen)
}

#[cfg(unix)]
#[test]
fn a_link_on_the_way_is_never_followed() {
    let (pair, elsewhere, seen) = lookalikes();
    let right = pair.right();
    fs::remove_dir_all(right.join("dir")).expect("removed");
    std::os::unix::fs::symlink(&elsewhere, right.join("dir")).expect("link");

    let ended = run(mirror(&pair.left(), &right, &[], &[("dir/a.txt", seen)], true));

    is_done(&ended, 1);
    assert_eq!(read(&elsewhere.join("a.txt")), b"keep me");
}

#[cfg(unix)]
#[test]
fn a_link_where_the_scan_saw_a_folder_is_kept_and_not_followed() {
    let pair = Pair::new();
    let right = pair.right();
    let elsewhere = pair.root.path().join("elsewhere");
    file(&elsewhere.join("a.txt"), b"keep me", 100);
    std::os::unix::fs::symlink(&elsewhere, right.join("link")).expect("link");
    // A link is `other` in the facts, which a removal cannot be asked for.
    let seen = SyncRemove {
        side: PaneId::Right,
        seen: SyncFacts {
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
        },
        files: 1,
        bytes: 7,
        scanned: millis(SystemTime::now()),
    };

    let ended = run(mirror(&pair.left(), &right, &[], &[("link", seen)], true));

    is_done(&ended, 1);
    assert!(
        fs::symlink_metadata(right.join("link")).is_ok(),
        "the link is not as the scan saw it"
    );
    assert_eq!(read(&elsewhere.join("a.txt")), b"keep me");
}

#[cfg(unix)]
#[test]
fn a_folder_that_holds_a_file_it_cannot_read_is_kept_and_reported() {
    use std::os::unix::fs::PermissionsExt;

    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("dir/locked/a.txt"), b"a", 100);
    let seen = removal(&right, "dir", PaneId::Right);
    fs::set_permissions(right.join("dir/locked"), fs::Permissions::from_mode(0o000)).expect("locked");
    // Root reads every folder: there is nothing to show then.
    let readable = fs::read_dir(right.join("dir/locked")).is_ok();

    let ended = run(mirror(&pair.left(), &right, &[], &[("dir", seen)], true));

    fs::set_permissions(right.join("dir/locked"), fs::Permissions::from_mode(0o755)).expect("unlocked");
    if readable {
        return;
    }
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
    assert_eq!(read(&right.join("dir/locked/a.txt")), b"a");
}

/// Removals to the Trash of the user, from a folder under the home folder (a temporary folder may be on a disk with no
/// Trash); what the test leaves in the Trash is purged.
#[cfg(target_os = "linux")]
mod to_the_trash {
    use std::fs;

    use tempfile::TempDir;

    use super::super::trash::{home_trash, purge_below};
    use super::{PaneId, file, is_done, mirror, removal, run};

    struct Place(TempDir);

    impl Drop for Place {
        fn drop(&mut self) {
            if let Some(trash) = home_trash() {
                purge_below(&trash, self.0.path());
            }
        }
    }

    #[test]
    fn a_removal_goes_to_the_trash_unless_it_is_for_good() {
        let home = std::env::home_dir().expect("a home directory");
        let place = Place(
            tempfile::Builder::new()
                .prefix("pairpane-mirror-test-")
                .tempdir_in(home)
                .expect("temp dir under the home directory"),
        );
        let (left, right) = (place.0.path().join("left"), place.0.path().join("right"));
        fs::create_dir_all(&left).expect("left");
        file(&right.join("dir/a.txt"), b"a", 100);
        let seen = removal(&right, "dir", PaneId::Right);

        let ended = run(mirror(&left, &right, &[], &[("dir", seen)], false));

        is_done(&ended, 0);
        assert!(!right.join("dir").exists());
        let trash = home_trash().expect("a home Trash");
        let found = fs::read_dir(trash.join("info"))
            .expect("the Trash has records")
            .filter_map(Result::ok)
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .any(|record| record.contains(&right.join("dir").to_string_lossy().into_owned()));
        assert!(found, "the folder is in the Trash");
    }
}

/// A folder on the way swapped for a link to another folder after it was looked at, before it is entered: nothing in the
/// other folder is removed, for good or to the Trash.
#[cfg(unix)]
#[test]
fn a_folder_on_the_way_swapped_for_a_link_mid_job_leads_nowhere() {
    use std::sync::Arc;

    for permanent in [true, false] {
        let (pair, elsewhere, seen) = lookalikes();
        let right = pair.right();
        let (dir, target) = (right.join("dir"), elsewhere.clone());
        let hooks = super::hooks::Hooks {
            before_open: Some(Arc::new(move |walked: &str| {
                if walked == "dir" {
                    fs::rename(&dir, dir.with_extension("old")).expect("moved aside");
                    std::os::unix::fs::symlink(&target, &dir).expect("link");
                }
            })),
            ..Default::default()
        };

        let ended =
            Rig::with(hooks, |_, _| {}).run(mirror(&pair.left(), &right, &[], &[("dir/a.txt", seen)], permanent));

        assert_eq!(ended.outcome, Outcome::Done);
        assert_eq!(ended.errors.len(), 1, "{:?}", ended.errors);
        assert_eq!(read(&elsewhere.join("a.txt")), b"keep me");
        assert_eq!(read(&right.join("dir.old/a.txt")), b"keep me");
    }
}

/// Two links to one file in two folders of a folder deleted for good: removing the first moves the change time of the
/// second, which the walk knows it did itself, so the folder goes whole.
#[test]
fn hard_links_in_a_folder_deleted_for_good_do_not_stop_it_halfway() {
    let pair = Pair::new();
    let right = pair.right();
    file(&right.join("dir/a/f.txt"), b"linked", 100);
    fs::create_dir_all(right.join("dir/b")).expect("b");
    fs::hard_link(right.join("dir/a/f.txt"), right.join("dir/b/f.txt")).expect("link");
    file(&right.join("dir/c/g.txt"), b"g", 100);
    // The scan begins once the setup is older than the slack the times are given.
    std::thread::sleep(std::time::Duration::from_millis(2_100));
    let mut seen = removal(&right, "dir", PaneId::Right);
    seen.scanned = millis(SystemTime::now());

    let ended = run(mirror(&pair.left(), &right, &[], &[("dir", seen)], true));

    is_done(&ended, 0);
    assert!(!right.join("dir").exists());
}

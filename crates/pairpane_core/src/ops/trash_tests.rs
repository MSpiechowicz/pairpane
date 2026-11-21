//! Tests of the Trash through the public functions: the checks every system makes, choosing the items to put back
//! where the system lists the Trash (pure), and real round trips through the Trash of the user.

use std::fs;
use std::io;

use tempfile::TempDir;

use super::trash::trash;

/// Choosing the items to put back where the system lists the Trash and the crate does the work (Windows, macOS).
#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
mod picking {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use ::trash::TrashItem;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::super::trash::{Trashed, pick, windows_comparable};

    const FOLDER: &str = "/somewhere/in/the/home";

    fn item(id: &str, name: &str, parent: &str, time: i64) -> TrashItem {
        TrashItem {
            id: OsString::from(id),
            name: OsString::from(name),
            original_parent: PathBuf::from(parent),
            time_deleted: time,
        }
    }

    fn record(names: &[&str]) -> Trashed {
        Trashed {
            folder: PathBuf::from(FOLDER),
            names: names.iter().map(|name| (*name).to_owned()).collect(),
            from: 100,
            to: 105,
        }
    }

    fn ids(items: &[TrashItem]) -> Vec<&str> {
        items.iter().map(|item| item.id.to_str().expect("text")).collect()
    }

    #[test]
    fn pick_keeps_the_items_with_the_names_of_the_record() {
        let all = vec![
            item("1", "a.txt", FOLDER, 101),
            item("2", "b.txt", FOLDER, 101),
            item("3", "c.txt", FOLDER, 101),
        ];

        let picked = pick(all, &record(&["a.txt", "c.txt"]));

        assert_eq!(ids(&picked), ["1", "3"]);
    }

    #[test]
    fn pick_keeps_only_what_was_trashed_inside_the_time_window_edges_included() {
        let all = vec![
            item("early", "a", FOLDER, 99),
            item("from", "b", FOLDER, 100),
            item("inside", "c", FOLDER, 103),
            item("to", "d", FOLDER, 105),
            item("late", "e", FOLDER, 106),
        ];

        let picked = pick(all, &record(&["a", "b", "c", "d", "e"]));

        assert_eq!(ids(&picked), ["from", "inside", "to"]);
    }

    #[test]
    fn pick_ignores_items_from_other_folders() {
        let all = vec![
            item("here", "a", FOLDER, 101),
            item("elsewhere", "a", "/somewhere/else", 101),
            item("deeper", "a", "/somewhere/in/the/home/sub", 101),
        ];

        let picked = pick(all, &record(&["a"]));

        assert_eq!(ids(&picked), ["here"]);
    }

    #[test]
    fn pick_takes_the_newest_of_items_with_one_original_path() {
        let all = vec![
            item("old", "a", FOLDER, 101),
            item("new", "a", FOLDER, 104),
            item("middle", "a", FOLDER, 102),
        ];

        let picked = pick(all, &record(&["a"]));

        assert_eq!(ids(&picked), ["new"]);
    }

    #[test]
    fn pick_orders_the_items_by_their_original_path() {
        let all = vec![
            item("z", "z", FOLDER, 101),
            item("a", "a", FOLDER, 101),
            item("m", "m", FOLDER, 101),
        ];

        let picked = pick(all, &record(&["z", "a", "m"]));

        assert_eq!(ids(&picked), ["a", "m", "z"]);
    }

    #[test]
    fn pick_finds_nothing_when_the_record_has_no_names_or_the_trash_is_empty() {
        assert!(pick(vec![item("1", "a", FOLDER, 101)], &record(&[])).is_empty());
        assert!(pick(Vec::new(), &record(&["a"])).is_empty());
    }

    #[test]
    fn pick_recognises_the_folder_under_another_spelling() {
        let real = TempDir::new().expect("real");
        let alias = TempDir::new().expect("alias");
        let link = alias.path().join("link");
        #[cfg(unix)]
        if std::os::unix::fs::symlink(real.path(), &link).is_err() {
            return;
        }
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(real.path(), &link).is_err() {
            return;
        }
        let mut wanted = record(&["a"]);
        wanted.folder = link;
        let all = vec![item("1", "a", real.path().to_str().expect("text"), 101)];

        let picked = pick(all, &wanted);

        assert_eq!(ids(&picked), ["1"]);
    }

    #[test]
    fn a_windows_path_compares_without_its_verbatim_prefix_and_without_case() {
        assert_eq!(windows_comparable(r"\\?\C:\Users\Me\Docs"), r"c:\users\me\docs");
        assert_eq!(windows_comparable(r"C:\Users\Me\Docs"), r"c:\users\me\docs");
        // Not a drive path: the prefix stays, so a network path is never taken for a local one.
        assert_eq!(windows_comparable(r"\\?\UNC\Host\Share"), r"\\?\unc\host\share");
    }
}

#[test]
fn trash_refuses_a_name_that_is_not_plain_and_reports_nothing_trashed() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(temp.path().join("kept.txt"), "kept").expect("write");

    for bad in ["", "..", "x/y"] {
        let (record, result) = trash(temp.path(), &[bad.to_owned()]);

        let failure = result.expect_err(bad);
        assert_eq!(failure.error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        assert_eq!(failure.name, bad);
        assert!(record.names.is_empty());
    }
    assert!(temp.path().join("kept.txt").exists());
}

#[cfg(not(any(
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))),
    windows
)))]
#[test]
fn undoing_is_unsupported_where_the_system_cannot_list_the_trash() {
    use super::trash::{CAN_UNTRASH, Trashed, UntrashFailure, untrash};

    assert!(!CAN_UNTRASH);
    assert_eq!(untrash(&Trashed::default()), Err(UntrashFailure::Unsupported));
}

/// Moves to the real Trash of the user and back. Every item is made in a folder of its own under the home directory
/// (a temporary folder may be on a disk with no Trash), and what a test leaves in the Trash is purged.
#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))),
    windows
))]
mod round_trips {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::super::trash::{Trashed, UntrashFailure, trash, untrash};

    /// A folder to trash from; its leftovers in the Trash are purged when it goes.
    struct Place {
        dir: TempDir,
    }

    impl Place {
        fn new() -> Self {
            let home = std::env::home_dir().expect("a home directory");
            let dir = tempfile::Builder::new()
                .prefix("pairpane-trash-test-")
                .tempdir_in(home)
                .expect("temp dir under the home directory");

            Self { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn join(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
    }

    #[cfg(windows)]
    impl Drop for Place {
        fn drop(&mut self) {
            use ::trash::{TrashItem, os_limited};

            use super::super::trash::comparable;

            let Some(folder) = comparable(self.dir.path()) else {
                return;
            };
            let left: Vec<TrashItem> = os_limited::list()
                .unwrap_or_default()
                .into_iter()
                .filter(|item| comparable(&item.original_parent).is_some_and(|parent| parent.starts_with(&folder)))
                .collect();

            let _ = os_limited::purge_all(left);
        }
    }

    /// Removes what the tests left in the home Trash: the items (and their records) that came from this folder,
    /// found by the unique name of the folder in the path of the record.
    #[cfg(target_os = "linux")]
    impl Drop for Place {
        fn drop(&mut self) {
            // The home Trash is found by the rule the code uses (`$XDG_DATA_HOME`, else `$HOME/.local/share`).
            if let Some(trash) = super::super::trash::home_trash() {
                super::super::trash::purge_below(&trash, self.dir.path());
            }
        }
    }

    /// A place with the file `a.txt` ("old") moved to the Trash from it.
    fn trashed_a() -> (Place, Trashed) {
        let place = Place::new();
        fs::write(place.join("a.txt"), "old").expect("write");
        let (moved, result) = trash(place.path(), &names(&["a.txt"]));
        result.expect("trashed");

        (place, moved)
    }

    /// What an undo says when an item of this name is at the old place.
    fn in_the_way(name: &str) -> Result<usize, UntrashFailure> {
        Err(UntrashFailure::InTheWay { name: name.to_owned() })
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_file_and_a_folder_go_to_the_trash_and_come_back() {
        let place = Place::new();
        fs::write(place.join("f.txt"), "file").expect("write");
        fs::create_dir(place.join("d")).expect("folder");
        fs::write(place.join("d/inner.txt"), "inner").expect("write");

        let (moved, result) = trash(place.path(), &names(&["f.txt", "d"]));

        result.expect("trashed");
        assert_eq!(moved.names, ["f.txt", "d"]);
        #[cfg(windows)]
        assert!(moved.to - moved.from >= 2);
        assert!(!place.join("f.txt").exists());
        assert!(!place.join("d").exists());

        assert_eq!(untrash(&moved), Ok(2));

        assert_eq!(fs::read_to_string(place.join("f.txt")).expect("back"), "file");
        assert_eq!(fs::read_to_string(place.join("d/inner.txt")).expect("back"), "inner");
        assert_eq!(untrash(&moved), Err(UntrashFailure::NothingFound));
    }

    #[test]
    fn a_new_item_at_the_old_place_blocks_the_restore_until_it_is_gone() {
        let (place, moved) = trashed_a();
        fs::write(place.join("a.txt"), "new").expect("write");

        let blocked = untrash(&moved);

        assert_eq!(blocked, in_the_way("a.txt"));
        assert_eq!(fs::read_to_string(place.join("a.txt")).expect("untouched"), "new");

        fs::remove_file(place.join("a.txt")).expect("remove");

        assert_eq!(untrash(&moved), Ok(1));
        assert_eq!(fs::read_to_string(place.join("a.txt")).expect("back"), "old");
    }

    #[test]
    fn a_link_to_a_folder_is_trashed_alone_and_the_target_keeps_its_contents() {
        let place = Place::new();
        let target = place.join("target");
        fs::create_dir(&target).expect("folder");
        fs::write(target.join("inner.txt"), "inner").expect("write");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, place.join("link"));
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&target, place.join("link"));
        if made.is_err() {
            return;
        }

        let (moved, result) = trash(place.path(), &names(&["link"]));

        result.expect("trashed");
        assert!(fs::symlink_metadata(place.join("link")).is_err());
        assert_eq!(fs::read_to_string(target.join("inner.txt")).expect("kept"), "inner");

        assert_eq!(untrash(&moved), Ok(1));

        assert_eq!(fs::read_link(place.join("link")).expect("a link again"), target);
        assert_eq!(fs::read_to_string(target.join("inner.txt")).expect("kept"), "inner");
    }

    #[test]
    fn a_failure_stops_the_move_and_lists_only_what_reached_the_trash() {
        let place = Place::new();
        fs::write(place.join("first"), "1").expect("write");
        fs::write(place.join("last"), "3").expect("write");

        let (moved, result) = trash(place.path(), &names(&["first", "missing", "last"]));

        let failure = result.expect_err("stopped");
        assert_eq!(failure.name, "missing");
        assert_eq!(failure.error.kind(), io::ErrorKind::NotFound);
        let text = failure.error.to_string();
        assert!(!text.contains("pairpane-trash-test"), "{text}");
        assert_eq!(moved.names, ["first"]);
        assert!(!place.join("first").exists());
        assert!(place.join("last").exists());

        assert_eq!(untrash(&moved), Ok(1));
        assert!(place.join("first").exists());
    }

    #[test]
    fn nothing_is_found_for_a_move_that_trashed_nothing() {
        let place = Place::new();

        let (moved, result) = trash(place.path(), &names(&["missing"]));

        assert!(result.is_err());
        assert!(moved.names.is_empty());
        assert_eq!(untrash(&moved), Err(UntrashFailure::NothingFound));
    }

    /// Windows asks the shell to put an item back with no prompt, and the crate takes a link that leads nowhere for a
    /// free place: the restore looks at the place first, without following links.
    #[cfg(windows)]
    #[test]
    fn a_link_that_leads_nowhere_at_the_old_place_blocks_the_restore() {
        let (place, moved) = trashed_a();
        if std::os::windows::fs::symlink_file(place.join("nowhere"), place.join("a.txt")).is_err() {
            return;
        }

        let blocked = untrash(&moved);

        assert_eq!(blocked, in_the_way("a.txt"));
        assert!(fs::symlink_metadata(place.join("a.txt")).expect("kept").is_symlink());
    }
}

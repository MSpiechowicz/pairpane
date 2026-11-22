//! The walk on real folders: pairing, recursion, one-sided folders, hidden entries, links, limits, a stop and the refusals.
//! An archive as a side is in `archive_tests`.

use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

use pretty_assertions::assert_eq;

use crate::model::{CompareEvent, CompareOptions, CompareStatus, EntryKind};

#[cfg(unix)]
use super::testing::link_to_real;
use super::testing::{compare, compare_with, compared, folder, folders, normal, plain, put, refused, temp};
use super::{Limits, Side};

use CompareStatus::{Differs, Equal, LeftNewer, LeftOnly, RightNewer, RightOnly};

fn without_subfolders() -> CompareOptions {
    CompareOptions {
        subfolders: false,
        ..plain()
    }
}

#[test]
fn names_are_paired_and_the_rows_say_which_side_has_them_and_which_is_newer() {
    let (left, right) = (temp(), temp());
    put(left.path(), "same.txt", b"abc", 10_000);
    put(right.path(), "same.txt", b"abc", 10_500);
    put(left.path(), "newer.txt", b"abc", 90_000);
    put(right.path(), "newer.txt", b"abc", 10_000);
    put(left.path(), "older.txt", b"abc", 10_000);
    put(right.path(), "older.txt", b"abc", 90_000);
    put(left.path(), "size.txt", b"abc", 10_000);
    put(right.path(), "size.txt", b"abcd", 10_000);
    put(left.path(), "only-left.txt", b"x", 10_000);
    put(right.path(), "only-right.txt", b"x", 10_000);

    let found = compare(left.path(), right.path(), plain());

    assert_eq!(
        found.statuses(),
        [
            ("newer.txt".to_owned(), LeftNewer),
            ("older.txt".to_owned(), RightNewer),
            ("only-left.txt".to_owned(), LeftOnly),
            ("only-right.txt".to_owned(), RightOnly),
            ("same.txt".to_owned(), Equal),
            ("size.txt".to_owned(), Differs),
        ]
    );
    assert_eq!(found.done(), (6, 0, false, false));

    let rows = found.rows();
    let only_left = &rows[2];
    assert!(only_left.right.is_none());
    assert_eq!(only_left.left.expect("left").size, 1);
}

#[test]
fn common_folders_are_entered_and_paths_are_joined_with_a_slash() {
    let (left, right) = (temp(), temp());
    put(left.path(), "a/b/deep.txt", b"1", 10_000);
    put(right.path(), "a/b/deep.txt", b"1", 90_000);
    put(left.path(), "a/only.txt", b"1", 10_000);
    folder(left.path(), "empty");
    folder(right.path(), "empty");

    let found = compare(left.path(), right.path(), plain());

    // A folder both sides have is not a row when its contents are compared.
    assert_eq!(
        found.statuses(),
        [
            ("a/b/deep.txt".to_owned(), RightNewer),
            ("a/only.txt".to_owned(), LeftOnly)
        ]
    );
}

#[test]
fn without_subfolders_a_common_folder_is_one_row_that_both_have() {
    let (left, right) = (temp(), temp());
    put(left.path(), "a/inner.txt", b"1", 10_000);
    put(right.path(), "a/other.txt", b"1", 10_000);

    let found = compare(left.path(), right.path(), without_subfolders());

    assert_eq!(found.statuses(), [("a".to_owned(), Equal)]);
    assert_eq!(found.rows()[0].left.expect("left").kind, EntryKind::Dir);
}

#[test]
fn a_folder_only_one_side_has_is_one_row_with_the_totals_of_what_is_in_it() {
    let (left, right) = (temp(), temp());
    put(left.path(), "tree/a.txt", b"12345", 10_000);
    put(left.path(), "tree/sub/b.txt", b"123", 10_000);
    put(left.path(), "tree/sub/.hidden", b"12", 10_000);
    folder(left.path(), "tree/sub/empty");

    let found = compare(left.path(), right.path(), plain());

    let rows = found.rows();
    assert_eq!(rows.len(), 1);
    let facts = rows[0].left.expect("left");
    assert_eq!((rows[0].path.as_str(), rows[0].status), ("tree", LeftOnly));
    assert_eq!(facts.kind, EntryKind::Dir);
    // Hidden files are in the totals: a copy of the folder copies them.
    assert_eq!((facts.files, facts.bytes), (3, 10));
}

// Windows hides by an attribute and not by a leading dot.
#[cfg(not(windows))]
#[test]
fn hidden_entries_are_left_out_unless_asked_for() {
    let (left, right) = (temp(), temp());
    put(left.path(), ".secret", b"1", 10_000);
    put(left.path(), ".dir/inside.txt", b"1", 10_000);
    put(left.path(), "shown.txt", b"1", 10_000);
    let hidden = CompareOptions {
        hidden: true,
        ..plain()
    };

    let without = compare(left.path(), right.path(), plain());
    let with = compare(left.path(), right.path(), hidden);

    assert_eq!(without.statuses(), [("shown.txt".to_owned(), LeftOnly)]);
    let paths: Vec<_> = with.statuses().into_iter().map(|(path, _)| path).collect();
    assert_eq!(paths, [".dir", ".secret", "shown.txt"]);
}

#[test]
fn a_file_and_a_folder_of_one_name_differ_and_nothing_below_them_is_compared() {
    let (left, right) = (temp(), temp());
    put(left.path(), "x", b"1", 10_000);
    put(right.path(), "x/inside.txt", b"1", 10_000);

    let found = compare(left.path(), right.path(), plain());

    assert_eq!(found.statuses(), [("x".to_owned(), Differs)]);
}

#[test]
fn names_pair_ignoring_case_only_where_the_system_does() {
    let (left, right) = (temp(), temp());
    put(left.path(), "Readme.TXT", b"1", 10_000);
    put(right.path(), "readme.txt", b"1", 10_000);
    let by = |fold| compared(folders(left.path(), right.path()), plain(), Limits { fold, ..normal() });

    assert_eq!(by(true).statuses(), [("Readme.TXT".to_owned(), Equal)]);
    assert_eq!(
        by(false).statuses(),
        [
            ("Readme.TXT".to_owned(), LeftOnly),
            ("readme.txt".to_owned(), RightOnly)
        ]
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn of_two_names_of_one_side_that_fold_together_the_first_is_paired_and_the_other_is_skipped() {
    let (left, right) = (temp(), temp());
    put(left.path(), "A.txt", b"1", 10_000);
    put(left.path(), "a.txt", b"2", 10_000);
    put(right.path(), "a.txt", b"1", 10_000);

    let found = compared(
        folders(left.path(), right.path()),
        plain(),
        Limits { fold: true, ..normal() },
    );

    assert_eq!(found.statuses(), [("A.txt".to_owned(), Equal)]);
    assert_eq!(found.done(), (1, 1, false, false));
}

#[cfg(unix)]
#[test]
fn a_link_is_a_row_of_its_own_and_never_followed() {
    let (left, right, elsewhere) = (temp(), temp(), temp());
    link_to_real(elsewhere.path(), left.path(), "to-folder");
    link_to_real(elsewhere.path(), right.path(), "to-folder");
    link_to_real(elsewhere.path(), left.path(), "one-sided");
    std::os::unix::fs::symlink("nowhere", right.path().join("dangling")).expect("link");

    let found = compare(left.path(), right.path(), plain());

    assert_eq!(
        found.statuses(),
        [
            ("dangling".to_owned(), RightOnly),
            ("one-sided".to_owned(), LeftOnly),
            ("to-folder".to_owned(), Equal),
        ]
    );
    let one_sided = found.rows()[1].left.expect("left");
    assert_eq!((one_sided.kind, one_sided.link), (EntryKind::Other, true));
    // The link to a folder is not a folder to total.
    assert_eq!((one_sided.files, one_sided.bytes), (0, 0));
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_against_a_real_folder_differs_and_is_not_entered() {
    let (left, right, elsewhere) = (temp(), temp(), temp());
    link_to_real(elsewhere.path(), left.path(), "same-name");
    put(right.path(), "same-name/file.txt", b"1", 10_000);

    let found = compare(left.path(), right.path(), plain());

    assert_eq!(found.statuses(), [("same-name".to_owned(), Differs)]);
}

#[test]
fn the_same_folder_and_a_folder_inside_the_other_are_refused_with_no_path_in_the_text() {
    let root = temp();
    folder(root.path(), "inner");
    let inner = root.path().join("inner");

    for (left, right) in [(root.path(), root.path()), (root.path(), &inner), (&inner, root.path())] {
        let error = refused(folders(left, right));

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!error.to_string().contains(root.path().to_str().expect("text")));
    }
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_does_not_hide_that_it_is_inside_the_other() {
    use std::os::unix::fs::symlink;

    let root = temp();
    folder(root.path(), "inner");
    symlink(root.path().join("inner"), root.path().join("alias")).expect("link");

    let error = refused(folders(root.path(), &root.path().join("alias")));

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_root_that_is_not_a_folder_or_not_absolute_fails_before_any_event() {
    let (left, right) = (temp(), temp());
    put(left.path(), "file.txt", b"1", 10_000);

    let missing = refused(folders(&left.path().join("missing"), right.path()));
    let file = refused(folders(&left.path().join("file.txt"), right.path()));
    let relative = refused((Side::Folder("relative".into()), Side::Folder(right.path().to_owned())));

    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    assert_eq!(file.kind(), io::ErrorKind::NotADirectory);
    assert_eq!(relative.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn rows_stop_at_the_cap_and_done_says_truncated() {
    let (left, right) = (temp(), temp());
    for number in 0..10 {
        put(left.path(), &format!("f{number}.txt"), b"1", 10_000);
    }

    let found = compared(
        folders(left.path(), right.path()),
        plain(),
        Limits { rows: 3, ..normal() },
    );

    assert_eq!(found.rows().len(), 3);
    assert_eq!(found.done(), (3, 0, true, false));
}

#[test]
fn a_stop_during_the_walk_ends_it_with_cancelled_and_the_rows_so_far() {
    let (left, right) = (temp(), temp());
    for number in 0..50 {
        put(left.path(), &format!("f{number:02}.txt"), b"1", 10_000);
    }
    let at_once = Limits {
        tick: Duration::ZERO,
        ..normal()
    };

    // Every event goes out at once, and the first one raises the flag.
    let found = compare_with(folders(left.path(), right.path()), plain(), at_once, |event, cancel| {
        if matches!(event, CompareEvent::Rows { .. }) {
            cancel.store(true, Ordering::Relaxed);
        }
    })
    .expect("a comparison");

    let (rows, _, truncated, cancelled) = found.done();
    assert!(cancelled);
    assert!(!truncated);
    assert!(rows < 50, "stopped early: {rows}");
}

#[test]
fn rows_are_sent_in_batches_with_progress_and_one_done_last() {
    let (left, right) = (temp(), temp());
    for number in 0..600 {
        put(left.path(), &format!("f{number:03}.txt"), b"1", 10_000);
    }

    let found = compare(left.path(), right.path(), plain());

    let batches: Vec<usize> = found
        .events
        .iter()
        .filter_map(|event| match event {
            CompareEvent::Rows { rows } => Some(rows.len()),
            _ => None,
        })
        .collect();
    assert!(batches.iter().all(|size| *size <= 256), "{batches:?}");
    assert_eq!(batches.iter().sum::<usize>(), 600);
    assert!(
        found
            .events
            .iter()
            .any(|event| matches!(event, CompareEvent::Progress { files: 600, .. }))
    );
    assert_eq!(found.done(), (600, 0, false, false));
}

#[cfg(unix)]
#[test]
fn a_name_that_is_not_text_is_skipped_and_counted() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (left, right) = (temp(), temp());
    put(left.path(), "fine.txt", b"1", 10_000);
    std::fs::write(left.path().join(OsStr::from_bytes(b"bad\xff.txt")), b"1").expect("write");

    let found = compare(left.path(), right.path(), plain());

    assert_eq!(found.statuses(), [("fine.txt".to_owned(), LeftOnly)]);
    assert_eq!(found.done(), (1, 1, false, false));
}

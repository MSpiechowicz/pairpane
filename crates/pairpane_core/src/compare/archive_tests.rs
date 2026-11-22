//! An archive as one side of a comparison: listed from its index, with the folders its entries imply, its own hidden rule and
//! the entries it cannot give left out.

use std::io;

use pretty_assertions::assert_eq;

use crate::archive::EntryType;
use crate::model::CompareOptions;
use crate::model::CompareStatus::{Equal, LeftNewer, LeftOnly, RightOnly};

use super::testing::{archive_and, compared, index_of, normal, plain, put, refused, temp};

#[test]
fn an_archive_side_is_listed_from_its_index_with_implicit_folders_and_the_hidden_rule() {
    let other = temp();
    put(other.path(), "a.txt", b"12345", 10_000);
    put(other.path(), "docs/b.txt", b"123", 5_000);
    put(other.path(), "docs/only-here.txt", b"123", 5_000);
    let index = index_of(&[
        ("a.txt", EntryType::File, 5, 10_000),
        ("docs/b.txt", EntryType::File, 3, 90_000),
        ("docs/deep/c.txt", EntryType::File, 3, 10_000),
        (".hidden", EntryType::File, 1, 10_000),
    ]);

    let found = compared(archive_and(index, "", other.path()), plain(), normal());

    assert_eq!(
        found.statuses(),
        [
            ("a.txt".to_owned(), Equal),
            ("docs/b.txt".to_owned(), LeftNewer),
            ("docs/deep".to_owned(), LeftOnly),
            ("docs/only-here.txt".to_owned(), RightOnly),
        ]
    );
    // The folder that only the archive's entries imply has the totals of what is below.
    assert_eq!(found.rows()[2].left.expect("left").files, 1);
}

#[test]
fn an_archive_folder_is_the_root_of_its_side_and_a_missing_one_is_refused() {
    let other = temp();
    put(other.path(), "b.txt", b"123", 10_000);
    let index = index_of(&[("docs/b.txt", EntryType::File, 3, 10_000)]);

    let found = compared(archive_and(index.clone(), "docs", other.path()), plain(), normal());
    let missing = refused(archive_and(index, "nope", other.path()));

    assert_eq!(found.statuses(), [("b.txt".to_owned(), Equal)]);
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
}

#[test]
fn content_is_not_compared_with_an_archive_side() {
    let other = temp();
    put(other.path(), "a.txt", b"12345", 10_000);
    let index = index_of(&[("a.txt", EntryType::File, 5, 10_000)]);
    let by_content = CompareOptions {
        content: true,
        ..plain()
    };

    let found = compared(archive_and(index, "", other.path()), by_content, normal());

    assert_eq!(found.statuses(), [("a.txt".to_owned(), Equal)]);
    assert_eq!(found.done().1, 0);
}

#[test]
fn entries_of_an_archive_with_refused_names_are_counted_as_skipped() {
    let other = temp();
    put(other.path(), "a.txt", b"12345", 10_000);
    let index = index_of(&[
        ("a.txt", EntryType::File, 5, 10_000),
        ("../evil.txt", EntryType::File, 5, 10_000),
    ]);

    let found = compared(archive_and(index, "", other.path()), plain(), normal());

    assert_eq!(found.statuses(), [("a.txt".to_owned(), Equal)]);
    assert_eq!(found.done(), (1, 1, false, false));
}

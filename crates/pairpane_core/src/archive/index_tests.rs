//! The index: the tree of folders, duplicates, flagged names, what an extraction selects, and the limits.

use pretty_assertions::assert_eq;

use crate::model::ArchiveFormat;

use super::entries::{EntryType, Flow, Place, Record};
use super::index::{ArchiveIndex, Builder, Child};
use super::names::Unsafe;
use super::{MAX_INDEX_ENTRIES, MAX_INDEX_NAME_BYTES};

fn record(name: &str, kind: EntryType, size: u64) -> Record {
    Record {
        name: name.to_owned(),
        kind,
        size,
        modified: Some(1000),
        mode: Some(0o644),
        link: None,
        encrypted: false,
        place: Place::Walk,
    }
}

fn index_of(records: &[(&str, EntryType)]) -> ArchiveIndex {
    let mut builder = Builder::new(ArchiveFormat::Tar);

    for (name, kind) in records {
        assert_eq!(builder.push(record(name, *kind, 1)), Flow::Continue);
    }

    builder.finish(false)
}

fn names(children: Vec<Child<'_>>) -> Vec<(&str, bool)> {
    children.into_iter().map(|child| (child.name, child.dir)).collect()
}

fn relative(index: &ArchiveIndex, dir: &str, wanted: &[&str]) -> (Vec<String>, u32, u32) {
    let wanted: Vec<String> = wanted.iter().map(|name| (*name).to_owned()).collect();
    let selection = index.select(dir, &wanted).expect("a selection");
    let paths = selection.items.iter().map(|item| item.relative.to_owned()).collect();

    (paths, selection.skipped, selection.refused)
}

#[test]
fn folders_are_listed_by_what_is_inside_them_in_archive_order() {
    let index = index_of(&[
        ("docs/", EntryType::Dir),
        ("docs/readme.txt", EntryType::File),
        ("src/main.rs", EntryType::File),
        ("top.txt", EntryType::File),
        ("src/lib/a.rs", EntryType::File),
    ]);

    assert_eq!(
        names(index.list("").expect("root")),
        [("docs", true), ("src", true), ("top.txt", false)]
    );
    assert_eq!(names(index.list("docs").expect("docs")), [("readme.txt", false)]);
    // `src` has no entry of its own: it exists because of what is below it, as does `src/lib`.
    assert_eq!(
        names(index.list("src").expect("src")),
        [("main.rs", false), ("lib", true)]
    );
    assert_eq!(names(index.list("src/lib").expect("src/lib")), [("a.rs", false)]);
    assert!(index.list("src").expect("src")[1].entry.is_none());
    assert!(
        index.list("docs").expect("docs")[0]
            .entry
            .is_some_and(|entry| entry.size == 1)
    );
    assert!(index.list("nope").is_none());
    // A file is not a folder.
    assert!(index.list("top.txt").is_none());
    assert!(index.has_dir("src/lib") && !index.has_dir("top.txt") && index.has_dir(""));
}

#[test]
fn find_returns_the_name_in_a_folder() {
    let index = index_of(&[("a/b/c.txt", EntryType::File)]);

    let found = index.find("a/b", "c.txt").expect("found");
    assert!(!found.dir && found.entry.is_some_and(|entry| entry.path == "a/b/c.txt"));
    assert!(index.find("a", "b").expect("a folder").dir);
    assert!(index.find("", "a").is_some());
    assert!(index.find("a", "c.txt").is_none());
}

#[test]
fn the_last_of_two_entries_with_one_path_wins_and_a_folder_beats_a_file() {
    let mut builder = Builder::new(ArchiveFormat::Tar);
    builder.push(record("a.txt", EntryType::File, 1));
    builder.push(record("a.txt", EntryType::File, 2));
    builder.push(record("x", EntryType::File, 3));
    builder.push(record("x/inner", EntryType::File, 4));
    builder.push(record("y/", EntryType::Dir, 0));
    builder.push(record("y", EntryType::File, 5));
    let index = builder.finish(false);

    let size_of = |dir: &str, name: &str| {
        index
            .find(dir, name)
            .and_then(|child| child.entry)
            .map(|entry| entry.size)
    };

    assert_eq!(size_of("", "a.txt"), Some(2));
    // `x` was a file and then a folder: the folder stays, and its file is gone.
    assert!(index.find("", "x").expect("x").dir);
    assert_eq!(size_of("", "x"), None);
    // `y` was a folder; a file with its path comes too late to replace it.
    assert!(index.find("", "y").expect("y").dir);
    assert_eq!(
        names(index.list("").expect("root")),
        [("a.txt", false), ("x", true), ("y", true)]
    );
    assert_eq!(index.entries().len(), 6, "every entry is kept, replaced ones too");
}

#[test]
fn an_unsafe_name_is_kept_flagged_and_not_in_the_tree() {
    let index = index_of(&[
        ("ok.txt", EntryType::File),
        ("../evil", EntryType::File),
        ("/etc/passwd", EntryType::File),
        ("a/../../b", EntryType::File),
        ("C:\\x", EntryType::File),
        ("fine/x", EntryType::File),
    ]);

    assert_eq!(
        names(index.list("").expect("root")),
        [("ok.txt", false), ("fine", true)]
    );
    assert_eq!(index.refused(), 4);
    assert_eq!(index.entries()[1].refused, Some(Unsafe::Parent));
    assert_eq!(index.entries()[2].refused, Some(Unsafe::Absolute));
    assert!(
        index
            .entries()
            .iter()
            .filter(|entry| entry.refused.is_some())
            .all(|entry| !entry.writable())
    );

    let (paths, skipped, refused) = relative(&index, "", &[]);
    assert_eq!(paths, ["ok.txt", "fine/x"]);
    assert_eq!((skipped, refused), (0, 4));
}

#[test]
fn the_roots_own_entry_is_kept_and_never_listed() {
    let index = index_of(&[("./", EntryType::Dir), ("./a", EntryType::File)]);

    assert_eq!(index.entries().len(), 2);
    assert_eq!(index.entries()[0].path, "");
    assert!(!index.entries()[0].writable());
    assert_eq!(names(index.list("").expect("root")), [("a", false)]);
    assert_eq!(index.refused(), 0);
}

#[test]
fn a_selection_has_the_chosen_names_and_everything_below_in_archive_order() {
    let index = index_of(&[
        ("a/", EntryType::Dir),
        ("a/b/", EntryType::Dir),
        ("a/b/one.txt", EntryType::File),
        ("a/two.txt", EntryType::File),
        ("a/link", EntryType::Symlink),
        ("a/pipe", EntryType::Special),
        ("a/hard", EntryType::Hardlink),
        ("other.txt", EntryType::File),
        ("a/b/three.txt", EntryType::File),
    ]);

    let (paths, skipped, refused) = relative(&index, "", &["a"]);
    assert_eq!(paths, ["a", "a/b", "a/b/one.txt", "a/two.txt", "a/b/three.txt"]);
    assert_eq!((skipped, refused), (3, 0));

    // Selected from inside a folder, the paths start at the chosen name.
    let (paths, ..) = relative(&index, "a", &["b", "two.txt"]);
    assert_eq!(paths, ["b", "b/one.txt", "two.txt", "b/three.txt"]);

    let (paths, skipped, _) = relative(&index, "a/b", &[]);
    assert_eq!(paths, ["one.txt", "three.txt"]);
    assert_eq!(skipped, 0);

    let selection = index.select("a", &["two.txt".to_owned()]).expect("selected");
    assert_eq!(selection.ordinals(), [3]);
    assert!(!selection.any_encrypted());
}

#[test]
fn selecting_what_is_not_there_gives_nothing() {
    let index = index_of(&[("a/b.txt", EntryType::File)]);

    assert!(index.select("nope", &[]).is_none());
    assert!(index.select("a/b.txt", &[]).is_none(), "a file is not a folder");
    assert!(index.select("a", &["missing".to_owned()]).is_none());
}

#[test]
fn refused_names_are_counted_only_where_the_selection_reaches() {
    let index = index_of(&[
        ("a/ok", EntryType::File),
        ("a/../x", EntryType::File),
        ("b/ok", EntryType::File),
        ("b/../y", EntryType::File),
    ]);

    assert_eq!(relative(&index, "", &["a"]).2, 1);
    assert_eq!(relative(&index, "", &["b"]).2, 1);
    assert_eq!(relative(&index, "", &[]).2, 2);
}

#[test]
fn an_encrypted_file_is_noticed() {
    let mut builder = Builder::new(ArchiveFormat::Zip);
    builder.push(Record {
        encrypted: true,
        ..record("secret.txt", EntryType::File, 3)
    });
    builder.push(record("open.txt", EntryType::File, 3));
    let index = builder.finish(false);

    assert!(index.select("", &[]).expect("all").any_encrypted());
    assert!(!index.select("", &["open.txt".to_owned()]).expect("one").any_encrypted());
}

#[test]
fn the_index_stops_at_its_entry_and_name_limits_and_is_marked_cut() {
    let mut builder = Builder::new(ArchiveFormat::Tar);

    for number in 0..MAX_INDEX_ENTRIES {
        assert_eq!(
            builder.push(record(&format!("f{number}"), EntryType::File, 0)),
            Flow::Continue
        );
    }
    assert_eq!(builder.push(record("one more", EntryType::File, 0)), Flow::Stop);
    let index = builder.finish(false);

    assert_eq!(index.entries().len(), MAX_INDEX_ENTRIES);
    assert!(index.truncated());

    // The names together over the limit: each name is within the length limit, so it takes many.
    let mut builder = Builder::new(ArchiveFormat::Tar);
    let long = "n".repeat(255);
    let mut count = 0;
    while builder.push(record(&format!("{long}{count}"), EntryType::File, 0)) == Flow::Continue {
        count += 1;
    }
    assert!(count > 0 && count <= MAX_INDEX_NAME_BYTES / 255);
    assert!(builder.finish(false).truncated());
}

#[test]
fn a_walk_that_ended_early_marks_the_index_cut() {
    assert!(Builder::new(ArchiveFormat::Tar).finish(true).truncated());
    assert!(!Builder::new(ArchiveFormat::Tar).finish(false).truncated());
}

//! Tests of the folders a pane lists: a folder on disk and a folder inside an archive, through the one interface.

use std::fs;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pretty_assertions::assert_eq;

use super::*;
use crate::archive::{ArchiveIndex, EntryType};
use crate::compare::testing::index_of;
use crate::listing::Outcome;

fn sample() -> Arc<ArchiveIndex> {
    index_of(&[
        ("docs/", EntryType::Dir, 0, 1_000),
        ("docs/guide.txt", EntryType::File, 5, 2_000),
        ("docs/deep/more.txt", EntryType::File, 4, 3_000),
        ("readme.txt", EntryType::File, 7, 4_000),
        (".secret", EntryType::File, 1, 5_000),
    ])
}

fn archive_folder(dir: &str) -> ArchiveFolder {
    let folder = std::env::temp_dir().join("pairpane-vfs");

    ArchiveFolder::new(sample(), folder, "pack.tar".to_owned(), dir.to_owned())
}

/// Lists `place` to the end and returns every event.
fn listed(place: &dyn VfsFolder, show_hidden: bool) -> Vec<ListEvent> {
    let mut events = Vec::new();
    let outcome = place
        .list(show_hidden, &AtomicBool::new(false), &mut |event| events.push(event))
        .expect("listable");
    assert_eq!(outcome, Outcome::Done);

    events
}

fn entries(events: &[ListEvent]) -> Vec<Entry> {
    let batches = events.iter().filter_map(|event| match event {
        ListEvent::Batch { entries, .. } => Some(entries.as_slice()),
        _ => None,
    });

    batches.flatten().cloned().collect()
}

fn names(events: &[ListEvent]) -> Vec<(String, EntryKind)> {
    entries(events)
        .into_iter()
        .map(|entry| (entry.name, entry.kind))
        .collect()
}

#[test]
fn a_folder_on_disk_lists_where_it_is_and_can_be_written() {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::write(temp.path().join("a.txt"), b"12345").expect("file");
    fs::create_dir(temp.path().join("sub")).expect("dir");
    let local = LocalFolder::resolve(temp.path().to_owned()).expect("resolved");

    let events = listed(&local, false);

    let shown = temp.path().to_string_lossy().into_owned();
    assert_eq!(local.location(), PaneLocation::Folder { path: shown.clone() });
    assert_eq!(local.capabilities(), PaneCapabilities::WRITABLE);
    let ListEvent::Start {
        path,
        location,
        capabilities,
        total,
        ..
    } = &events[0]
    else {
        panic!("start comes first: {events:?}");
    };
    assert_eq!((path, *total), (&shown, 2));
    assert_eq!(*location, local.location());
    assert_eq!(*capabilities, PaneCapabilities::WRITABLE);
    assert_eq!(
        names(&events),
        [
            ("sub".to_owned(), EntryKind::Dir),
            ("a.txt".to_owned(), EntryKind::File)
        ]
    );
}

#[test]
fn a_folder_on_disk_is_watched_and_a_folder_in_an_archive_is_not() {
    let temp = tempfile::tempdir().expect("temp dir");
    let local = LocalFolder::resolve(temp.path().to_owned()).expect("resolved");

    assert!(matches!(local.watch(Box::new(|| {})), Some(Ok(_))));
    assert!(archive_folder("").watch(Box::new(|| {})).is_none());
}

#[test]
fn the_root_of_an_archive_is_listed_read_only_with_the_crumbs_of_its_place() {
    let place = archive_folder("");
    let folder = std::env::temp_dir().join("pairpane-vfs");
    let archive = folder.join("pack.tar").to_string_lossy().into_owned();

    let events = listed(&place, false);

    assert_eq!(
        place.location(),
        PaneLocation::Archive {
            archive: archive.clone(),
            dir: String::new()
        }
    );
    assert_eq!(place.capabilities(), PaneCapabilities::READ_ONLY);
    let ListEvent::Start {
        path,
        parent,
        crumbs,
        total,
        location,
        capabilities,
    } = &events[0]
    else {
        panic!("start comes first: {events:?}");
    };
    assert_eq!(*path, folder.to_string_lossy());
    assert_eq!(
        parent.as_deref(),
        folder.parent().map(|parent| parent.to_string_lossy()).as_deref()
    );
    assert_eq!((*total, location), (2, &place.location()));
    assert_eq!(*capabilities, PaneCapabilities::READ_ONLY);

    let last = crumbs.last().expect("crumbs");
    assert_eq!((last.name.as_str(), last.path.as_str()), ("pack.tar", archive.as_str()));
    assert_eq!(last.inside.as_deref(), Some(""));
    let real = &crumbs[crumbs.len() - 2];
    assert_eq!(
        (real.path.as_str(), real.inside.as_deref()),
        (&*folder.to_string_lossy(), None)
    );

    assert_eq!(
        names(&events),
        [
            ("docs".to_owned(), EntryKind::Dir),
            ("readme.txt".to_owned(), EntryKind::File)
        ]
    );
    assert!(matches!(
        events.last(),
        Some(ListEvent::Done {
            dirs: 1,
            files: 1,
            bytes: 7,
            free: None,
            ..
        })
    ));
}

#[test]
fn an_inner_folder_of_an_archive_has_a_crumb_per_folder_and_read_only_entries() {
    let events = listed(&archive_folder("docs/deep"), false);

    let ListEvent::Start { crumbs, location, .. } = &events[0] else {
        panic!("start comes first");
    };
    let inside: Vec<_> = crumbs
        .iter()
        .filter_map(|crumb| crumb.inside.as_deref().map(|inside| (crumb.name.as_str(), inside)))
        .collect();
    assert_eq!(inside, [("pack.tar", ""), ("docs", "docs"), ("deep", "docs/deep")]);
    assert!(matches!(location, PaneLocation::Archive { dir, .. } if dir == "docs/deep"));

    let ListEvent::Batch { entries, .. } = &events[1] else {
        panic!("one batch follows: {events:?}");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].name.as_str(), entries[0].size), ("more.txt", 4));
    assert!(entries[0].readonly);
    assert_eq!(entries[0].mode, Some(0o644));
    assert_eq!(entries[0].modified, Some(3_000));
}

#[test]
fn hidden_names_of_an_archive_show_only_when_asked_for() {
    let place = archive_folder("");

    let without = names(&listed(&place, false));
    let with = names(&listed(&place, true));

    assert!(without.iter().all(|(name, _)| name != ".secret"));
    assert!(with.iter().any(|(name, _)| name == ".secret"));
}

#[test]
fn a_folder_that_is_not_in_the_archive_fails_before_any_event() {
    let mut events = Vec::new();

    let error = archive_folder("nope")
        .list(false, &AtomicBool::new(false), &mut |event| events.push(event))
        .expect_err("not there");

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "This folder is not in the archive.");
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn a_listing_of_an_archive_that_was_cancelled_stops_before_done() {
    let mut events = Vec::new();

    let outcome = archive_folder("")
        .list(false, &AtomicBool::new(true), &mut |event| events.push(event))
        .expect("listable");

    assert_eq!(outcome, Outcome::Cancelled);
    assert!(matches!(events.as_slice(), [ListEvent::Start { .. }]), "{events:?}");
}

#[test]
fn entries_are_sorted_folders_first_then_by_name() {
    let file = |name: &str, kind| Entry {
        name: name.to_owned(),
        kind,
        size: 0,
        modified: None,
        hidden: false,
        readonly: false,
        mode: None,
        link_target: None,
    };

    let ordered = sorted(vec![
        file("b.txt", EntryKind::File),
        file("z", EntryKind::Dir),
        file("a.txt", EntryKind::File),
        file("m", EntryKind::Dir),
    ]);

    let ordered: Vec<_> = ordered.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(ordered, ["m", "z", "a.txt", "b.txt"]);
}

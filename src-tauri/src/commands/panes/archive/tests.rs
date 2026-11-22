//! Tests of `archive_open`: the listing of a tar and a zip with its crumbs, hidden and read-only entries, a folder that
//! is not there, and a later open of the pane superseding the earlier one.

use std::time::{Duration, Instant};

use pairpane_core::model::ListEvent;

use super::super::lock;
use super::*;
use crate::commands::archives::fixtures::{archive_in_folder, collecting, item, path_text, tar_of, zip_of};

/// Opens `dir` of `archive` in the left pane and returns what arrived once the listing ended.
fn listed(panes: &Panes, archives: &Archives, archive: &str, dir: &str, show_hidden: bool) -> Vec<ListEvent> {
    let (channel, events) = collecting();
    start(
        panes,
        archives,
        PaneId::Left,
        archive.to_owned(),
        dir.to_owned(),
        show_hidden,
        channel,
    )
    .expect("accepted");

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let seen = events.lock().expect("not poisoned").clone();
        if seen
            .iter()
            .any(|event| matches!(event, ListEvent::Done { .. } | ListEvent::Error { .. }))
        {
            return seen;
        }
        assert!(Instant::now() < deadline, "the listing ended");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn names_of(events: &[ListEvent]) -> Vec<(String, EntryKind)> {
    events
        .iter()
        .flat_map(|event| match event {
            ListEvent::Batch { entries, .. } => entries.clone(),
            _ => Vec::new(),
        })
        .map(|entry| (entry.name, entry.kind))
        .collect()
}

fn sample_tar() -> Vec<u8> {
    tar_of(&[
        item("docs/", b""),
        item("docs/guide.txt", b"guide"),
        item("docs/deep/more.txt", b"more"),
        item("readme.txt", b"hello"),
        item(".secret", b"s"),
    ])
}

#[test]
fn the_root_of_a_tar_is_listed_folders_first_with_the_crumbs_of_its_place() {
    let (folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let events = listed(&Panes::default(), &Archives::default(), &archive, "", false);

    let ListEvent::Start {
        path,
        parent,
        crumbs,
        total,
        archive: spot,
    } = &events[0]
    else {
        panic!("start comes first: {events:?}");
    };
    assert_eq!(*path, path_text(folder.path()));
    assert_eq!(parent.as_deref(), folder.path().parent().map(path_text).as_deref());
    assert_eq!(*total, 2);
    assert_eq!(
        spot,
        &Some(ArchiveSpot {
            archive: archive.clone(),
            dir: String::new()
        })
    );

    let last = crumbs.last().expect("crumbs");
    assert_eq!((last.name.as_str(), last.path.as_str()), ("pack.tar", archive.as_str()));
    assert_eq!(last.inside.as_deref(), Some(""));
    let real = &crumbs[crumbs.len() - 2];
    assert_eq!(real.path, path_text(folder.path()));
    assert_eq!(real.inside, None);

    assert_eq!(
        names_of(&events),
        vec![
            ("docs".to_owned(), EntryKind::Dir),
            ("readme.txt".to_owned(), EntryKind::File)
        ]
    );
    assert!(matches!(
        events.last(),
        Some(ListEvent::Done {
            dirs: 1,
            files: 1,
            bytes: 5,
            ..
        })
    ));
}

#[test]
fn an_inner_folder_is_listed_with_one_crumb_per_folder_and_every_entry_is_read_only() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let events = listed(&Panes::default(), &Archives::default(), &archive, "docs/deep", false);

    let ListEvent::Start {
        crumbs, archive: spot, ..
    } = &events[0]
    else {
        panic!("start comes first");
    };
    let inside: Vec<_> = crumbs
        .iter()
        .filter_map(|crumb| crumb.inside.as_deref().map(|inside| (crumb.name.as_str(), inside)))
        .collect();
    assert_eq!(inside, [("pack.tar", ""), ("docs", "docs"), ("deep", "docs/deep")]);
    assert_eq!(spot.as_ref().map(|spot| spot.dir.as_str()), Some("docs/deep"));

    let entries: Vec<Entry> = events
        .iter()
        .flat_map(|event| match event {
            ListEvent::Batch { entries, .. } => entries.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "more.txt");
    assert_eq!(entries[0].size, 4);
    assert!(entries[0].readonly);
    assert_eq!(entries[0].mode, Some(0o644));
    assert!(entries[0].modified.is_some());
}

#[test]
fn hidden_names_show_only_when_asked_for() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let (panes, archives) = (Panes::default(), Archives::default());

    let without = names_of(&listed(&panes, &archives, &archive, "", false));
    let with = names_of(&listed(&panes, &archives, &archive, "", true));

    assert!(without.iter().all(|(name, _)| name != ".secret"));
    assert!(with.iter().any(|(name, _)| name == ".secret"));
}

#[test]
fn a_stored_zip_lists_its_folders_including_those_only_its_paths_imply() {
    let zip = zip_of(&[item("a/b/c.txt", b"data"), item("top.txt", b"t")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);

    let events = listed(&Panes::default(), &Archives::default(), &archive, "a", false);

    assert_eq!(names_of(&events), vec![("b".to_owned(), EntryKind::Dir)]);
}

#[test]
fn a_folder_that_is_not_in_the_archive_is_a_not_found_error() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());

    let events = listed(&Panes::default(), &Archives::default(), &archive, "nope", false);

    let Some(ListEvent::Error { error }) = events.last() else {
        panic!("an error: {events:?}");
    };
    assert_eq!(error.code, IpcErrorCode::NotFound);
}

#[test]
fn a_file_that_is_not_an_archive_is_an_error_without_its_path() {
    let (folder, archive) = archive_in_folder("notes.tar", b"this is not a tar file at all");

    let events = listed(&Panes::default(), &Archives::default(), &archive, "", false);

    let Some(ListEvent::Error { error }) = events.last() else {
        panic!("an error: {events:?}");
    };
    assert!(!error.message.contains(&path_text(folder.path())), "{}", error.message);
}

#[test]
fn a_later_open_of_the_pane_supersedes_the_earlier_one_and_no_folder_is_watched() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let (panes, archives) = (Panes::default(), Archives::default());

    let (first, _) = collecting();
    start(
        &panes,
        &archives,
        PaneId::Left,
        archive.clone(),
        String::new(),
        false,
        first,
    )
    .expect("accepted");
    let earlier = lock(&panes.slots)
        .get(&PaneId::Left)
        .map(|slot| Arc::clone(&slot.cancel))
        .expect("slot");

    let events = listed(&panes, &archives, &archive, "docs", false);

    assert!(earlier.load(Ordering::Relaxed), "the earlier open was cancelled");
    assert!(matches!(&events[0], ListEvent::Start { archive: Some(spot), .. } if spot.dir == "docs"));
    let slots = lock(&panes.slots);
    let slot = slots.get(&PaneId::Left).expect("slot");
    assert_eq!(slot.generation, 2);
    assert!(slot.watch.is_none());
}

#[test]
fn a_bad_archive_path_is_refused_before_the_pane_changes() {
    let panes = Panes::default();
    let (channel, _) = collecting();

    let error = start(
        &panes,
        &Archives::default(),
        PaneId::Left,
        "relative.zip".to_owned(),
        String::new(),
        false,
        channel,
    )
    .expect_err("refused");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(lock(&panes.slots).is_empty());
}

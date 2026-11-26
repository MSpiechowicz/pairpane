//! Tests of `find_open`: the results listed with their relative names, hits that vanished left out, and a later open
//! of the pane superseding the earlier one.

use std::fs;

use pairpane_core::model::{EntryKind, IpcErrorCode};
use tauri::async_runtime::block_on;

use super::super::tests::{Log, gated, hit_paths, query, searched, tree};
use super::*;
use crate::commands::archives::fixtures::collecting;

/// Opens the results of `id` in the left pane; the command answers once the listing is sent, so the events are all there.
fn opened(panes: &Panes, finder: &Finder, id: FindId) -> (CommandResult<()>, Vec<ListEvent>) {
    let (channel, events) = collecting();
    let result = block_on(open(panes, finder, PaneId::Left, id, channel));
    let seen = events.lock().expect("not poisoned").clone();

    (result, seen)
}

fn names_of(events: &[ListEvent]) -> Vec<(String, EntryKind)> {
    let mut names = Vec::new();

    for event in events {
        if let ListEvent::Batch { entries, .. } = event {
            names.extend(entries.iter().map(|entry| (entry.name.clone(), entry.kind)));
        }
    }

    names
}

fn hit(path: &str, inner: Option<&str>) -> FindHit {
    FindHit {
        path: path.to_owned(),
        inner: inner.map(str::to_owned),
        kind: EntryKind::File,
        link: false,
        size: 0,
        modified: None,
        matches: None,
        line: None,
        line_number: None,
    }
}

/// A search of everything below a fresh tree, ended.
fn everything() -> (tempfile::TempDir, String, Finder, FindId) {
    let (folder, root) = tree();
    let finder = Finder::default();
    let (id, events) = searched(&finder, &query(&root, ""));
    assert_eq!(hit_paths(&events).len(), 4);

    (folder, root, finder, id)
}

#[test]
fn the_results_are_listed_under_the_folder_searched_with_relative_names_folders_first() {
    let (_folder, root, finder, id) = everything();

    let (result, events) = opened(&Panes::default(), &finder, id);

    result.expect("listed");
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
    assert_eq!(*path, root);
    assert_eq!(
        parent.as_deref(),
        Some(root.as_str()),
        "up from the results is the folder searched"
    );
    assert_eq!(crumbs.last().map(|crumb| crumb.path.as_str()), Some(root.as_str()));
    assert_eq!(*total, 4);
    assert_eq!(*location, PaneLocation::Results { id });
    assert_eq!(*capabilities, PaneCapabilities::WRITABLE);

    assert_eq!(
        names_of(&events),
        [
            ("sub".to_owned(), EntryKind::Dir),
            ("a.txt".to_owned(), EntryKind::File),
            ("c.log".to_owned(), EntryKind::File),
            ("sub/b.txt".to_owned(), EntryKind::File),
        ]
    );
    assert!(matches!(
        events.last(),
        Some(ListEvent::Done {
            dirs: 1,
            files: 3,
            bytes: 14,
            ..
        })
    ));
}

#[test]
fn a_hit_that_vanished_since_the_search_is_left_out_and_not_counted() {
    let (folder, _root, finder, id) = everything();
    fs::remove_file(folder.path().join("a.txt")).expect("removed");
    fs::remove_dir_all(folder.path().join("sub")).expect("removed");

    let (result, events) = opened(&Panes::default(), &finder, id);

    result.expect("listed");
    assert!(matches!(&events[0], ListEvent::Start { total: 1, .. }));
    assert_eq!(names_of(&events), [("c.log".to_owned(), EntryKind::File)]);
}

#[test]
fn a_search_that_is_not_kept_rejects_the_open_and_sends_nothing() {
    let (_folder, _root, finder, id) = everything();
    finder.forget(id).expect("known");

    let (result, events) = opened(&Panes::default(), &finder, id);

    assert_eq!(result.expect_err("gone").code(), IpcErrorCode::NotFound);
    assert!(events.is_empty());
}

#[test]
fn the_results_found_so_far_of_a_search_that_still_runs_can_be_listed() {
    let (_folder, root) = tree();
    let finder = Finder::default();
    let (channel, gate, log): (_, _, Log) = gated();
    let id = finder.start(&query(&root, "*.log"), channel).expect("started");
    super::super::tests::wait_until("the results", || {
        hit_paths(&log.lock().expect("not poisoned")).len() == 1
    });

    let (result, events) = opened(&Panes::default(), &finder, id);
    gate.release();

    result.expect("listed");
    assert_eq!(names_of(&events), [("c.log".to_owned(), EntryKind::File)]);
}

#[test]
fn a_later_open_of_the_pane_supersedes_the_earlier_one_and_a_raised_flag_stops_the_listing() {
    let panes = Panes::default();
    let (channel, _) = collecting();
    let earlier = panes
        .claim(PaneId::Left, SlotKey::Results(FindId(1)), &channel)
        .expect("claimed");
    let later = panes
        .claim(PaneId::Left, SlotKey::Results(FindId(2)), &channel)
        .expect("claimed");
    assert!(earlier.cancel.load(Ordering::Relaxed), "the earlier open was cancelled");
    assert!(!later.cancel.load(Ordering::Relaxed));
    assert_eq!(later.generation, earlier.generation + 1);

    let (_folder, root) = tree();
    let mut sent = Vec::new();
    let started = std::cell::Cell::new(false);
    list_results(
        Path::new(&root),
        &[hit("a.txt", None)],
        FindId(1),
        &earlier.cancel,
        || started.set(true),
        |event| sent.push(event),
    );

    assert!(sent.is_empty() && !started.get(), "a superseded listing sends nothing");
}

#[test]
fn a_hit_inside_an_archive_or_with_a_path_that_climbs_is_never_listed() {
    let (_folder, root) = tree();
    let hits = [
        hit("a.txt", Some("inside.txt")),
        hit("../x", None),
        hit("sub/../a.txt", None),
    ];

    let entries = present(Path::new(&root), &hits, &AtomicBool::new(false));

    assert!(entries.is_empty(), "{entries:?}");
}

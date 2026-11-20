use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::watch::{DEBOUNCE, MAX_WAIT};
use super::*;

/// Lists `path` to the end and returns every event.
fn events(path: &Path, show_hidden: bool) -> Vec<ListEvent> {
    let mut events = Vec::new();
    let outcome = list(path, show_hidden, &AtomicBool::new(false), |event| events.push(event)).expect("listable");
    assert_eq!(outcome, Outcome::Done);
    events
}

/// The entries of every batch, in order.
fn entries(events: &[ListEvent]) -> Vec<Entry> {
    events
        .iter()
        .flat_map(|event| match event {
            ListEvent::Batch { entries, .. } => entries.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn names(events: &[ListEvent]) -> Vec<String> {
    entries(events).into_iter().map(|entry| entry.name).collect()
}

/// A watch on `path` and the channel its changes arrive on.
fn watched(path: &Path) -> (FolderWatch, mpsc::Receiver<()>) {
    let (changed, changes) = mpsc::channel();
    let watch = watch(path, move || {
        changed.send(()).ok();
    })
    .expect("watchable");

    (watch, changes)
}

fn folder(files: &[&str], dirs: &[&str]) -> TempDir {
    let temp = TempDir::new().expect("temp dir");
    for file in files {
        fs::write(temp.path().join(file), b"12345").expect("file");
    }
    for dir in dirs {
        fs::create_dir(temp.path().join(dir)).expect("dir");
    }
    temp
}

#[test]
fn folders_first_then_names_and_hidden_left_out() {
    let temp = folder(&["b.txt", "a.txt", ".hidden"], &["zeta", "alpha"]);

    let events = events(temp.path(), false);

    let expected_hidden = !platform::HIDDEN_NEEDS_METADATA; // dot files are only hidden on Unix
    let mut expected = vec!["alpha", "zeta"];
    if !expected_hidden {
        expected.push(".hidden");
    }
    expected.extend(["a.txt", "b.txt"]);
    assert_eq!(names(&events), expected);
}

#[test]
fn hidden_entries_are_listed_and_marked_on_request() {
    let temp = folder(&[".hidden", "shown"], &[]);

    let listed = entries(&events(temp.path(), true));

    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].hidden, !platform::HIDDEN_NEEDS_METADATA);
    assert!(!listed[1].hidden);
}

#[test]
fn start_batches_and_done_describe_the_folder() {
    let temp = folder(&["one", "two"], &["sub"]);

    let events = events(temp.path(), false);

    let ListEvent::Start {
        path,
        parent,
        crumbs,
        total,
    } = &events[0]
    else {
        panic!("first event is start: {:?}", events[0]);
    };
    assert_eq!(path, &temp.path().to_string_lossy());
    assert_eq!(
        parent.as_deref(),
        temp.path().parent().map(|p| p.to_string_lossy()).as_deref()
    );
    assert_eq!(crumbs.last().map(|crumb| crumb.path.as_str()), Some(path.as_str()));
    assert_eq!(crumbs[0].path, crumbs[0].name, "the root is named as written");
    assert_eq!(*total, 3);

    let listed = entries(&events);
    assert_eq!(listed[0].kind, EntryKind::Dir);
    assert_eq!(listed[0].size, 0);
    assert_eq!(listed[1].kind, EntryKind::File);
    assert_eq!(listed[1].size, 5);
    assert!(listed[1].modified.is_some_and(|millis| millis > 1_600_000_000_000));
    assert_eq!(
        events.last(),
        Some(&ListEvent::Done {
            dirs: 1,
            files: 2,
            bytes: 10
        })
    );
}

#[test]
fn a_small_first_batch_then_full_batches_in_order() {
    let count = FIRST_BATCH + BATCH + 7;
    let temp = TempDir::new().expect("temp dir");
    for index in 0..count {
        fs::write(temp.path().join(format!("f{index:05}")), b"").expect("file");
    }

    let events = events(temp.path(), false);

    let batches: Vec<(u32, usize)> = events
        .iter()
        .filter_map(|event| match event {
            ListEvent::Batch { offset, entries } => Some((*offset, entries.len())),
            _ => None,
        })
        .collect();
    assert_eq!(batches, [(0, FIRST_BATCH), (200, BATCH), (2_200, 7)]);
    let listed = names(&events);
    assert!(listed.windows(2).all(|pair| pair[0] < pair[1]), "sorted across batches");
}

#[cfg(unix)]
#[test]
fn symbolic_links_show_their_target() {
    use std::os::unix::fs::symlink;

    let temp = folder(&["file"], &["dir"]);
    symlink(temp.path().join("dir"), temp.path().join("to-dir")).expect("link");
    symlink("missing", temp.path().join("broken")).expect("link");

    let listed = entries(&events(temp.path(), false));
    let find = |name: &str| listed.iter().find(|entry| entry.name == name).expect("listed").clone();

    assert_eq!(
        names(&events(temp.path(), false))[..2],
        ["dir", "to-dir"],
        "a link to a folder sorts as one"
    );
    assert_eq!(find("to-dir").kind, EntryKind::Dir);
    assert_eq!(
        find("to-dir").link_target.as_deref(),
        Some(&*temp.path().join("dir").to_string_lossy())
    );
    assert_eq!(find("broken").kind, EntryKind::Other);
    assert_eq!(find("broken").link_target.as_deref(), Some("missing"));
    assert_eq!(find("file").mode.map(|mode| mode & 0o600), Some(0o600));
}

#[test]
fn a_cancelled_listing_stops_before_done() {
    let temp = folder(&["a", "b"], &[]);
    let mut events = Vec::new();

    let outcome = list(temp.path(), false, &AtomicBool::new(true), |event| events.push(event)).expect("listable");

    assert_eq!(outcome, Outcome::Cancelled);
    assert!(events.is_empty());
}

#[test]
fn missing_and_relative_paths_fail() {
    let temp = TempDir::new().expect("temp dir");
    let cancel = AtomicBool::new(false);

    let missing = list(&temp.path().join("nope"), false, &cancel, |_| {}).expect_err("missing");
    let relative = list(Path::new("relative"), false, &cancel, |_| {}).expect_err("relative");

    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    assert_eq!(relative.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_watched_folder_reports_a_change() {
    let temp = TempDir::new().expect("temp dir");
    let (_watch, changes) = watched(temp.path());

    fs::write(temp.path().join("new"), b"").expect("file");

    changes
        .recv_timeout(Duration::from_secs(10))
        .expect("a change is reported");
}

#[test]
fn listing_a_watched_folder_is_not_a_change() {
    let temp = folder(&["a", "b"], &["sub"]);
    let (_watch, changes) = watched(temp.path());
    // macOS (FSEvents) reports changes made shortly before a watch started; let those pass first.
    while changes.recv_timeout(DEBOUNCE * 4).is_ok() {}

    // Reading the folder opens it (and on some systems updates its access time): no change, or a pane that
    // re-lists on every change would list forever.
    events(temp.path(), true);

    assert!(changes.recv_timeout(DEBOUNCE * 4).is_err(), "listing reported a change");
}

#[test]
fn cancelling_after_the_first_batch_stops_the_second_pass() {
    let temp = TempDir::new().expect("temp dir");
    for index in 0..FIRST_BATCH + 10 {
        fs::write(temp.path().join(format!("f{index:04}")), b"").expect("file");
    }
    let cancel = AtomicBool::new(false);
    let mut batches = 0;

    let outcome = list(temp.path(), false, &cancel, |event| {
        if matches!(event, ListEvent::Batch { .. }) {
            batches += 1;
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })
    .expect("listable");

    assert_eq!(outcome, Outcome::Cancelled);
    assert_eq!(batches, 1);
}

#[test]
fn a_burst_of_changes_is_reported_once_and_a_dropped_watch_goes_quiet() {
    let temp = TempDir::new().expect("temp dir");
    let (watch, changes) = watched(temp.path());

    for index in 0..5 {
        fs::write(temp.path().join(format!("f{index}")), b"").expect("file");
    }
    changes
        .recv_timeout(Duration::from_secs(10))
        .expect("the burst is reported");
    assert!(changes.recv_timeout(DEBOUNCE * 3).is_err(), "only once");

    drop(watch);
    fs::write(temp.path().join("after"), b"").expect("file");
    assert!(
        changes.recv_timeout(DEBOUNCE * 3).is_err(),
        "nothing after the watch is dropped"
    );
}

#[test]
fn a_folder_that_never_pauses_is_still_reported() {
    let temp = TempDir::new().expect("temp dir");
    let (_watch, changes) = watched(temp.path());

    let file = temp.path().join("growing.log");
    let started = std::time::Instant::now();
    let mut reported = false;
    while started.elapsed() < MAX_WAIT * 3 && !reported {
        fs::write(&file, started.elapsed().as_nanos().to_string()).expect("write");
        reported = changes.recv_timeout(DEBOUNCE / 3).is_ok();
    }

    assert!(
        reported,
        "reported within the maximum wait despite writes every {:?}",
        DEBOUNCE / 3
    );
}

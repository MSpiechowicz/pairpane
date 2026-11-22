//! Tests of the find commands: what a search streams, what refuses it, how many run and are kept at once, and how it is
//! stopped.

use std::fs;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use pairpane_core::model::FindCriteria;
use tauri::ipc::InvokeResponseBody;

use super::*;
use crate::commands::archives::fixtures::collecting;
use crate::commands::testing::{elsewhere, folder_with};

/// How long a test waits for something that must happen before it calls it a hang.
const PATIENCE: Duration = Duration::from_secs(20);

pub(super) type Log = Arc<Mutex<Vec<FindEvent>>>;

/// The search of `names` (blank is every name) in `root`.
pub(super) fn query(root: &str, names: &str) -> FindQuery {
    FindQuery {
        root: root.to_owned(),
        criteria: FindCriteria {
            names: names.to_owned(),
            ..FindCriteria::default()
        },
    }
}

/// A folder holding `a.txt`, `c.log` and `sub/b.txt`, and its path.
pub(super) fn tree() -> (tempfile::TempDir, String) {
    let (folder, path) = folder_with(&[("a.txt", b"alpha"), ("c.log", b"gamma")]);
    fs::create_dir(folder.path().join("sub")).expect("folder");
    fs::write(folder.path().join("sub").join("b.txt"), b"beta").expect("file");

    (folder, path)
}

pub(super) fn wait_until(what: &str, ready: impl Fn() -> bool) {
    let limit = Instant::now() + PATIENCE;

    while !ready() {
        assert!(Instant::now() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn ended(log: &Log) -> bool {
    let events = log.lock().expect("not poisoned");

    matches!(events.last(), Some(FindEvent::Done { .. } | FindEvent::Error { .. }))
}

/// Starts a search and waits for its last event.
pub(super) fn searched(finder: &Finder, query: &FindQuery) -> (FindId, Vec<FindEvent>) {
    let (channel, log) = collecting();
    let id = finder.start(query, channel).expect("started");
    wait_until("the search to end", || ended(&log));
    let events = log.lock().expect("not poisoned").clone();

    (id, events)
}

/// Starts a search of everything in `root` and waits until it has delivered its first event, so it is running (and held
/// at the gate).
fn held(finder: &Finder, root: &str) -> (FindId, Arc<Gate>, Log) {
    let (channel, gate, log) = gated();
    let id = finder.start(&query(root, ""), channel).expect("started");
    wait_until("the first event", || !log.lock().expect("not poisoned").is_empty());

    (id, gate, log)
}

/// Releases the gate, waits for the search to end and checks that it ended as cancelled.
fn ends_cancelled(gate: &Gate, log: &Log) {
    gate.release();
    wait_until("the search to end", || ended(log));

    assert!(matches!(
        log.lock().expect("not poisoned").last(),
        Some(FindEvent::Done { cancelled: true, .. })
    ));
}

pub(super) fn hit_paths(events: &[FindEvent]) -> Vec<String> {
    let mut paths: Vec<String> = events
        .iter()
        .flat_map(|event| match event {
            FindEvent::Found { hits } => hits.clone(),
            _ => Vec::new(),
        })
        .map(|hit| hit.path)
        .collect();
    paths.sort();

    paths
}

/// A gate that holds the page's channel: while it is shut, the search waits in the delivery of an event, so it runs on.
pub(super) struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    pub(super) fn release(&self) {
        *self.open.lock().expect("not poisoned") = true;
        self.changed.notify_all();
    }

    fn pass(&self) {
        let open = self.open.lock().expect("not poisoned");
        let (_open, result) = self
            .changed
            .wait_timeout_while(open, PATIENCE, |open| !*open)
            .expect("not poisoned");

        assert!(!result.timed_out(), "the gate was never opened");
    }
}

/// A channel whose events are recorded and then held at the gate, until it is released.
pub(super) fn gated() -> (Channel<FindEvent>, Arc<Gate>, Log) {
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        changed: Condvar::new(),
    });
    let log = Log::default();
    let (held, recorded) = (Arc::clone(&gate), Arc::clone(&log));

    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(text) = body else {
            panic!("events are sent as JSON");
        };
        recorded
            .lock()
            .expect("not poisoned")
            .push(serde_json::from_str(&text).expect("an event"));
        held.pass();

        Ok(())
    });

    (channel, gate, log)
}

#[test]
fn a_search_streams_its_results_and_ends_with_one_done() {
    let (_folder, root) = tree();

    let (id, events) = searched(&Finder::default(), &query(&root, "*.txt"));

    assert_eq!(id, FindId(1));
    assert_eq!(hit_paths(&events), ["a.txt", "sub/b.txt"]);
    let done: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, FindEvent::Done { .. }))
        .collect();
    assert_eq!(
        done,
        [&FindEvent::Done {
            found: 2,
            skipped: 0,
            truncated: false,
            cancelled: false
        }]
    );
    assert!(matches!(events.last(), Some(FindEvent::Done { .. })));
}

#[test]
fn a_search_that_cannot_run_is_refused_before_anything_starts() {
    let (_folder, root) = tree();
    let finder = Finder::default();

    let relative = finder.start(&query("relative/dir", ""), Channel::new(|_| Ok(())));
    assert_eq!(relative.expect_err("relative").code(), IpcErrorCode::InvalidInput);

    let mut bad = query(&root, "(");
    bad.criteria.name_regex = true;
    let error = finder.start(&bad, Channel::new(|_| Ok(()))).expect_err("bad regex");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    let text = serde_json::to_value(&error).expect("serializable")["message"].to_string();
    assert!(!text.contains(&root), "no path in the text: {text}");

    let mut empty = query(&root, "");
    empty.criteria.min_size = Some(5);
    empty.criteria.max_size = Some(1);
    assert!(finder.start(&empty, Channel::new(|_| Ok(()))).is_err());

    assert!(finder.get(FindId(1)).is_err(), "nothing was kept");
}

#[test]
fn a_root_that_is_not_a_folder_is_reported_as_the_search_s_error() {
    let (folder, _) = tree();
    let file = folder.path().join("a.txt").to_string_lossy().into_owned();

    let (_, events) = searched(&Finder::default(), &query(&file, ""));

    let [FindEvent::Error { error }] = events.as_slice() else {
        panic!("one error and nothing else: {events:?}");
    };
    assert_eq!(error.code, IpcErrorCode::InvalidInput);
    assert!(!error.message.contains("a.txt"));

    let missing = searched(&Finder::default(), &query(&elsewhere(), "")).1;
    assert!(matches!(missing.as_slice(), [FindEvent::Error { error }] if error.code == IpcErrorCode::NotFound));
}

#[test]
fn a_third_search_at_once_is_busy_until_one_ends() {
    let (_folder, root) = tree();
    let finder = Finder::default();
    let (first, first_gate, first_log) = gated();
    let (second, second_gate, _) = gated();

    finder.start(&query(&root, ""), first).expect("first");
    finder.start(&query(&root, ""), second).expect("second");

    let error = finder
        .start(&query(&root, ""), Channel::new(|_| Ok(())))
        .expect_err("busy");
    assert_eq!(error.code(), IpcErrorCode::Busy);
    assert_eq!(finder.lock().len(), 2, "the refused search is not kept");

    first_gate.release();
    wait_until("the first search to end", || ended(&first_log));
    wait_until("a place to be free", || {
        finder.start(&query(&root, ""), Channel::new(|_| Ok(()))).is_ok()
    });

    second_gate.release();
}

#[test]
fn cancelling_ends_a_search_with_a_cancelled_done_and_keeps_its_results() {
    let (_folder, root) = tree();
    let finder = Finder::default();
    let (id, gate, log) = held(&finder, &root);

    finder.cancel(id).expect("known");
    ends_cancelled(&gate, &log);

    assert!(finder.get(id).is_ok(), "a cancelled search is still kept");
}

#[test]
fn an_unknown_search_is_not_found_and_forgetting_stops_and_drops_one() {
    let (_folder, root) = tree();
    let finder = Finder::default();

    assert_eq!(
        finder.cancel(FindId(9)).expect_err("unknown").code(),
        IpcErrorCode::NotFound
    );
    assert_eq!(
        finder.forget(FindId(9)).expect_err("unknown").code(),
        IpcErrorCode::NotFound
    );

    let (id, gate, log) = held(&finder, &root);

    finder.forget(id).expect("known");
    ends_cancelled(&gate, &log);

    assert_eq!(finder.get(id).expect_err("gone").code(), IpcErrorCode::NotFound);
    assert_eq!(finder.forget(id).expect_err("gone").code(), IpcErrorCode::NotFound);
}

#[test]
fn the_oldest_search_that_has_ended_is_dropped_to_keep_four_and_one_that_runs_never_is() {
    let finder = Finder::default();
    let search = |ended: bool| {
        let search = Arc::new(Search::new(PathBuf::from(elsewhere())));
        search.finished.store(ended, Ordering::Release);

        search
    };

    // 1 runs, 2 and 3 have ended, 4 runs.
    for (id, ended) in [(1, false), (2, true), (3, true), (4, false)] {
        finder.store(FindId(id), search(ended)).expect("room");
    }

    finder.store(FindId(5), search(false)).expect("room by dropping 2");
    assert!(finder.get(FindId(2)).is_err(), "the oldest ended search went");
    for kept in [1, 3, 4, 5] {
        assert!(finder.get(FindId(kept)).is_ok(), "{kept}");
    }

    // 3 is the only one that has ended; dropping it leaves four that run.
    finder
        .get(FindId(1))
        .expect("kept")
        .finished
        .store(false, Ordering::Release);
    finder.store(FindId(6), search(false)).expect("room by dropping 3");
    let error = finder.store(FindId(7), search(false)).expect_err("all run");
    assert_eq!(error.code(), IpcErrorCode::Busy);
    assert_eq!(finder.lock().len(), STORED);
}

#[test]
fn searches_that_end_are_evicted_in_the_order_they_started() {
    let (_folder, root) = tree();
    let finder = Finder::default();

    let ids: Vec<FindId> = (0..STORED + 1)
        .map(|_| {
            let (id, _) = searched(&finder, &query(&root, "*.log"));
            wait_until("the search to be marked ended", || {
                finder.get(id).is_ok_and(|s| s.finished())
            });

            id
        })
        .collect();

    assert!(finder.get(ids[0]).is_err());
    assert!(ids[1..].iter().all(|id| finder.get(*id).is_ok()));
}

#[test]
fn a_search_whose_channel_is_gone_stops() {
    let (_folder, root) = tree();
    let finder = Finder::default();
    let gone = Channel::new(|_| Err(tauri::Error::Io(std::io::Error::other("closed"))));

    let id = finder.start(&query(&root, ""), gone).expect("started");

    let search = finder.get(id).expect("kept");
    wait_until("the search to end", || search.finished());
    assert!(search.cancel.load(Ordering::Relaxed));
}

#[test]
fn shutting_down_stops_every_search() {
    let (_folder, root) = tree();
    let finder = Finder::default();
    let (_, gate, log) = held(&finder, &root);

    finder.shut_down();
    ends_cancelled(&gate, &log);
}

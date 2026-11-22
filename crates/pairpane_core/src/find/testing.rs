//! Helpers shared by the tests of a search: trees on disk and a runner that cannot hang the test run.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;

use crate::model::{FindCriteria, FindEvent, FindHit, FindQuery};

use super::{Limits, compile, run_limited};

/// How long a search may take in a test before it counts as hung.
const WATCHDOG: Duration = Duration::from_secs(60);

pub(super) fn temp() -> TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

/// Writes `bytes` to `rel` (names joined with `/`) below `root`, making the folders.
pub(super) fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);

    fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    fs::write(path, bytes).expect("write");
}

pub(super) fn folder(root: &Path, rel: &str) {
    fs::create_dir_all(root.join(rel)).expect("folder");
}

/// Criteria that look for names fitting `names`.
pub(super) fn named(names: &str) -> FindCriteria {
    FindCriteria {
        names: names.to_owned(),
        ..FindCriteria::default()
    }
}

/// What a search delivered.
pub(super) struct Found {
    pub(super) events: Vec<FindEvent>,
}

impl Found {
    /// Every result of the `found` events, ordered by path then inner path.
    pub(super) fn hits(&self) -> Vec<FindHit> {
        let mut hits: Vec<FindHit> = self
            .events
            .iter()
            .flat_map(|event| match event {
                FindEvent::Found { hits } => hits.clone(),
                _ => Vec::new(),
            })
            .collect();
        hits.sort_by(|a, b| (&a.path, &a.inner).cmp(&(&b.path, &b.inner)));

        hits
    }

    /// The paths of the results, sorted; a result inside an archive is `archive>inner`.
    pub(super) fn paths(&self) -> Vec<String> {
        self.hits().iter().map(label).collect()
    }

    /// The groups of duplicates as sorted paths, the groups sorted too.
    pub(super) fn groups(&self) -> Vec<Vec<String>> {
        let mut groups: Vec<Vec<String>> = self
            .events
            .iter()
            .filter_map(|event| match event {
                FindEvent::Group { hits, .. } => {
                    let mut paths: Vec<String> = hits.iter().map(label).collect();
                    paths.sort();

                    Some(paths)
                }
                _ => None,
            })
            .collect();
        groups.sort();

        groups
    }

    /// `(found, skipped, truncated, cancelled)` of the `done` event, which must be the last and the only one.
    pub(super) fn done(&self) -> (u64, u64, bool, bool) {
        let Some(FindEvent::Done {
            found,
            skipped,
            truncated,
            cancelled,
        }) = self.events.last()
        else {
            panic!("the search ends with done: {:?}", self.events.last());
        };
        let dones = self
            .events
            .iter()
            .filter(|event| matches!(event, FindEvent::Done { .. }))
            .count();
        assert_eq!(dones, 1, "exactly one done");

        (*found, *skipped, *truncated, *cancelled)
    }

    /// The bytes of the last `progress` event.
    pub(super) fn bytes(&self) -> u64 {
        self.events
            .iter()
            .rev()
            .find_map(|event| match event {
                FindEvent::Progress { bytes, .. } => Some(*bytes),
                _ => None,
            })
            .unwrap_or(0)
    }
}

fn label(hit: &FindHit) -> String {
    match &hit.inner {
        Some(inner) => format!("{}>{inner}", hit.path),
        None => hit.path.clone(),
    }
}

/// Searches `root` for `criteria` and collects the events.
pub(super) fn search(root: &Path, criteria: FindCriteria) -> Found {
    try_search(root, criteria).expect("the search runs")
}

pub(super) fn try_search(root: &Path, criteria: FindCriteria) -> std::io::Result<Found> {
    watched(
        root,
        criteria,
        Limits::DEFAULT,
        &Arc::new(AtomicBool::new(false)),
        |_| false,
    )
}

/// [`search`] with other limits.
pub(super) fn search_limited(root: &Path, criteria: FindCriteria, limits: Limits) -> Found {
    watched(root, criteria, limits, &Arc::new(AtomicBool::new(false)), |_| false).expect("the search runs")
}

/// [`search`] that raises the cancel flag at the first event `stop` says yes to (or before it starts, when `early`).
pub(super) fn search_stopped(root: &Path, criteria: FindCriteria, early: bool, stop: fn(&FindEvent) -> bool) -> Found {
    let cancel = Arc::new(AtomicBool::new(early));

    watched(root, criteria, Limits::DEFAULT, &cancel, stop).expect("the search runs")
}

/// Runs the search on a thread of its own and fails the test when it does not end in time.
fn watched(
    root: &Path,
    criteria: FindCriteria,
    limits: Limits,
    cancel: &Arc<AtomicBool>,
    stop: impl Fn(&FindEvent) -> bool + Send + 'static,
) -> std::io::Result<Found> {
    let query = FindQuery {
        root: root.to_string_lossy().into_owned(),
        criteria,
    };
    let root = root.to_owned();
    let cancel = Arc::clone(cancel);
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let outcome = compile(&query).and_then(|compiled| {
            let mut events = Vec::new();
            let raise = Arc::clone(&cancel);

            run_limited(&root, &compiled, &cancel, limits, &mut |event| {
                if stop(&event) {
                    raise.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                events.push(event);
            })
            .map(|()| Found { events })
        });

        sender.send(outcome).ok();
    });

    receiver.recv_timeout(WATCHDOG).expect("the search ended in time")
}

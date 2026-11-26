//! What the download tests share: a server with a connected site, an engine whose events land in a channel, and the
//! folder downloads go to.

use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

use pairpane_core::model::{ControlAction, ItemError, JobId, OpKind, OpsEvent, Outcome, SiteId};
use pairpane_core::ops::{Engine, Request};
use pairpane_core::vfs::RemoteFs;
use tempfile::TempDir;

use super::transfer_fs::wait_for_open;
use super::{Harness, path, quick};
use crate::test_server::Settings;
use crate::{AgentSource, Timeouts};

const PASSWORD: &str = "correct horse";

/// How long a test waits for something before it gives up (a hang then fails the test instead of freezing it).
const PATIENCE: Duration = Duration::from_secs(30);

/// How a job ended.
#[derive(Debug)]
pub(super) struct Ended {
    pub(super) outcome: Outcome,
    pub(super) skipped: u32,
    pub(super) errors: Vec<ItemError>,
}

impl Ended {
    /// The paths of the items that failed.
    pub(super) fn failed(&self) -> Vec<&str> {
        self.errors.iter().map(|item| item.path.as_str()).collect()
    }

    /// The message of a job that failed as a whole.
    pub(super) fn failure(&self) -> &str {
        match &self.outcome {
            Outcome::Failed { error } => &error.message,
            other => panic!("the job did not fail: {other:?}"),
        }
    }
}

/// A server with a connected site, an engine whose events land in a channel, and the folder downloads go to.
pub(super) struct Rig {
    pub(super) h: Harness,
    pub(super) site: SiteId,
    pub(super) fs: Arc<dyn RemoteFs>,
    pub(super) engine: Engine,
    pub(super) local: TempDir,
    events: Receiver<OpsEvent>,
    seen: Vec<OpsEvent>,
}

impl Rig {
    pub(super) fn new(settings: Settings) -> Self {
        Self::with(Harness::new(settings))
    }

    /// The same, where an idle connection is closed after `idle`.
    pub(super) fn idling(settings: Settings, idle: Duration) -> Self {
        Self::with(Harness::with(settings, Timeouts { idle, ..quick() }, AgentSource::None))
    }

    fn with(h: Harness) -> Self {
        h.trust();

        let site = h.site(Some(PASSWORD));
        let fs = h.pool.fs(site).expect("a site");
        let engine = Engine::new();
        let (sender, events) = mpsc::channel();

        engine.set_sink(Arc::new(move |event| {
            let _ = sender.send(event);
        }));

        Self {
            h,
            site,
            fs,
            engine,
            local: tempfile::tempdir().expect("temp dir"),
            events,
            seen: Vec::new(),
        }
    }

    /// A download of `names` of the server folder `dir` into `to`.
    pub(super) fn request_to(&self, kind: OpKind, dir: &str, names: &[&str], to: &Path) -> Request {
        Request::download(
            kind,
            Arc::clone(&self.fs),
            path(dir),
            names.iter().map(|name| (*name).to_owned()).collect(),
            to.to_owned(),
            false,
        )
    }

    /// A download into the rig's local folder.
    pub(super) fn request(&self, kind: OpKind, dir: &str, names: &[&str]) -> Request {
        self.request_to(kind, dir, names, self.local.path())
    }

    pub(super) fn start(&self, request: Request) -> JobId {
        self.engine.submit(request).expect("accepted")
    }

    /// Runs `request` to its end.
    pub(super) fn run(&mut self, request: Request) -> Ended {
        let job = self.start(request);

        self.ended(job)
    }

    /// Downloads `names` of the server folder `dir` into the rig's local folder.
    pub(super) fn download(&mut self, kind: OpKind, dir: &str, names: &[&str]) -> Ended {
        let request = self.request(kind, dir, names);

        self.run(request)
    }

    pub(super) fn wait(&mut self, wanted: impl Fn(&OpsEvent) -> bool) -> OpsEvent {
        loop {
            if let Some(found) = self.seen.iter().position(&wanted) {
                return self.seen.remove(found);
            }

            let event = self
                .events
                .recv_timeout(PATIENCE)
                .unwrap_or_else(|_| panic!("no wanted event came; seen so far: {:#?}", self.seen));

            self.seen.push(event);
        }
    }

    pub(super) fn ended(&mut self, job: JobId) -> Ended {
        match self.wait(|event| matches!(event, OpsEvent::Ended { id, .. } if *id == job)) {
            OpsEvent::Ended {
                outcome,
                skipped,
                errors,
                ..
            } => Ended {
                outcome,
                skipped,
                errors,
            },
            other => unreachable!("{other:?}"),
        }
    }

    /// Whether the job has ended, as far as the events that came show.
    pub(super) fn has_ended(&mut self, job: JobId) -> bool {
        while let Ok(event) = self.events.try_recv() {
            self.seen.push(event);
        }

        self.seen
            .iter()
            .any(|event| matches!(event, OpsEvent::Ended { id, .. } if *id == job))
    }

    pub(super) fn control(&self, job: JobId, action: ControlAction) {
        self.engine.control(job, action).expect("a known job");
    }

    /// Waits until the server has received `count` READ requests (all connections).
    pub(super) fn wait_for_reads(&self, count: usize) {
        wait_until("the READ requests", || {
            self.h.server.counts.reads.load(Ordering::Relaxed) >= count
        });
    }

    /// The files the local folder holds, and that no handle of the server is left open.
    pub(super) fn local_tree(&self) -> Vec<String> {
        tree(self.local.path())
    }

    pub(super) fn no_handles_left(&self) {
        wait_for_open(&self.h, 0);
    }

    pub(super) fn on_server(&self, name: &str) -> PathBuf {
        self.h.server.root.path().join(name)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // The jobs end (and clean up) before the pool and the server are dropped.
        self.engine.shutdown(Duration::from_secs(2));
    }
}

/// Polls `condition` until it holds, for at most [`PATIENCE`].
pub(super) fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let started = Instant::now();

    while !condition() {
        assert!(started.elapsed() < PATIENCE, "{what} did not come");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Every path below `dir`, relative and sorted, folders with a trailing `/`.
pub(super) fn tree(dir: &Path) -> Vec<String> {
    let mut paths = below(dir, "");

    paths.sort();

    paths
}

fn below(dir: &Path, prefix: &str) -> Vec<String> {
    fs::read_dir(dir)
        .expect("folder")
        .flat_map(|entry| {
            let entry = entry.expect("entry");
            let shown = format!("{prefix}{}", entry.file_name().to_string_lossy());

            if entry.path().is_dir() {
                let inside = below(&entry.path(), &format!("{shown}/"));

                std::iter::once(format!("{shown}/")).chain(inside).collect()
            } else {
                vec![shown]
            }
        })
        .collect()
}

/// Seconds after 1970 of a time a test sets on the server's files (whole seconds are what SFTP carries).
pub(super) const LONG_AGO: u64 = 1_000_000_000;

pub(super) fn long_ago() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(LONG_AGO)
}

pub(super) fn set_times(path: &Path) {
    let times = FileTimes::new().set_modified(long_ago()).set_accessed(long_ago());
    let file = if path.is_dir() {
        File::open(path)
    } else {
        File::options().write(true).open(path)
    };

    file.expect("opened").set_times(times).expect("times");
}

pub(super) fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).expect("metadata").modified().expect("modified")
}

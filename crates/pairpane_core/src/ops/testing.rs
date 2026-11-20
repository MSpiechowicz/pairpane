//! Helpers shared by the tests of the engine: an engine whose events land in a channel, and small file helpers.

use std::collections::HashSet;
use std::fs::{self, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use crate::model::{
    Answer, ConflictInfo, ControlAction, ItemError, JobId, JobPhase, JobView, OpKind, OpsEvent, Outcome,
};

use super::hooks::Hooks;
use super::open;
use super::volume::VolumeKey;
use super::{Engine, Request};

/// How long a test waits for an event before it gives up (a hang then fails the test instead of freezing it).
const PATIENCE: Duration = Duration::from_secs(30);

/// How a job ended.
#[derive(Debug)]
pub(super) struct Ended {
    pub(super) outcome: Outcome,
    pub(super) skipped: u32,
    pub(super) errors: Vec<ItemError>,
    pub(super) more_errors: u32,
}

/// An engine, and the events its jobs produce, in order.
pub(super) struct Rig {
    pub(super) engine: Engine,
    events: Receiver<OpsEvent>,
    /// Every event taken from the channel so far.
    pub(super) seen: Vec<OpsEvent>,
    /// Which of `seen` an earlier `wait` returned already.
    claimed: HashSet<usize>,
}

impl Rig {
    pub(super) fn new() -> Self {
        Self::with(Hooks::default(), |_, _| {})
    }

    /// An engine with `hooks`, whose sink first calls `observer` (on the engine's thread, like any sink).
    pub(super) fn with(hooks: Hooks, observer: impl Fn(&Engine, &OpsEvent) + Send + Sync + 'static) -> Self {
        let engine = Engine::with_hooks(hooks);
        let (sender, events) = mpsc::channel();
        let inside = engine.clone();

        engine.set_sink(Arc::new(move |event| {
            observer(&inside, &event);
            let _ = sender.send(event);
        }));

        Self {
            engine,
            events,
            seen: Vec::new(),
            claimed: HashSet::new(),
        }
    }

    /// Returns the first event, seen or still to come, that satisfies `wanted` and that no earlier call returned.
    pub(super) fn wait(&mut self, wanted: impl Fn(&OpsEvent) -> bool) -> OpsEvent {
        loop {
            let found = (0..self.seen.len()).find(|index| !self.claimed.contains(index) && wanted(&self.seen[*index]));

            if let Some(index) = found {
                self.claimed.insert(index);

                return self.seen[index].clone();
            }

            let event = self
                .events
                .recv_timeout(PATIENCE)
                .unwrap_or_else(|_| panic!("no wanted event came; seen so far: {:#?}", self.seen));
            self.seen.push(event);
        }
    }

    pub(super) fn wait_ended(&mut self, job: JobId) -> Ended {
        match self.wait(|event| matches!(event, OpsEvent::Ended { id, .. } if *id == job)) {
            OpsEvent::Ended {
                outcome,
                skipped,
                errors,
                more_errors,
                ..
            } => Ended {
                outcome,
                skipped,
                errors,
                more_errors,
            },
            other => unreachable!("{other:?}"),
        }
    }

    pub(super) fn wait_conflict(&mut self, job: JobId) -> ConflictInfo {
        match self.wait(|event| matches!(event, OpsEvent::Conflict { conflict } if conflict.job == job)) {
            OpsEvent::Conflict { conflict } => conflict,
            other => unreachable!("{other:?}"),
        }
    }

    /// A rig whose hooks `set` changes from their defaults.
    pub(super) fn hooked(set: impl FnOnce(&mut Hooks)) -> Self {
        let mut hooks = Hooks::default();
        set(&mut hooks);

        Self::with(hooks, |_, _| {})
    }

    /// A rig whose listener pauses every job as it starts, before any byte can move: no race with the worker.
    pub(super) fn paused_at_start() -> Self {
        Self::with(Hooks::default(), |engine, event| {
            if let OpsEvent::Phase {
                id,
                phase: JobPhase::Preparing,
            } = event
            {
                engine.control(*id, ControlAction::Pause).expect("known job");
            }
        })
    }

    /// Runs a copy that verifies each file, from `root/src` into `root/dst`, to its end.
    pub(super) fn run_verified(&mut self, root: &Path, names: &[&str]) -> Ended {
        let mut request = copy(&root.join("src"), names, &root.join("dst"));
        request.verify = true;

        self.run(request)
    }

    /// Starts a copy of `names` from `root/src` into `root/dst`.
    pub(super) fn copy_in(&self, root: &Path, names: &[&str]) -> JobId {
        self.start(copy(&root.join("src"), names, &root.join("dst")))
    }

    /// Starts a move of `names` from `root/src` into `root/dst`.
    pub(super) fn move_in(&self, root: &Path, names: &[&str]) -> JobId {
        self.start(relocate(&root.join("src"), names, &root.join("dst")))
    }

    /// Runs a copy of `names` from `root/src` into `root/dst` to its end.
    pub(super) fn run_copy(&mut self, root: &Path, names: &[&str]) -> Ended {
        let job = self.copy_in(root, names);

        self.wait_ended(job)
    }

    /// Runs a move of `names` from `root/src` into `root/dst` to its end.
    pub(super) fn run_move(&mut self, root: &Path, names: &[&str]) -> Ended {
        let job = self.move_in(root, names);

        self.wait_ended(job)
    }

    /// Waits for the next question of `job` and answers it; returns the question.
    pub(super) fn answer_next(&mut self, job: JobId, answer: Answer, all: bool) -> ConflictInfo {
        let question = self.wait_conflict(job);
        self.engine
            .answer(job, question.question, answer, all)
            .expect("an open question");

        question
    }

    /// Submits a job and returns its id.
    pub(super) fn start(&self, request: Request) -> JobId {
        self.engine.submit(request).expect("accepted")
    }

    /// Runs a job to its end and returns how it ended.
    pub(super) fn run(&mut self, request: Request) -> Ended {
        let job = self.start(request);

        self.wait_ended(job)
    }

    /// The `Ended` events seen for `job`.
    pub(super) fn ended_count(&self, job: JobId) -> usize {
        self.seen
            .iter()
            .filter(|event| matches!(event, OpsEvent::Ended { id, .. } if *id == job))
            .count()
    }

    /// Takes whatever else arrives within a moment (for events that must not come).
    pub(super) fn drain(&mut self) {
        while let Ok(event) = self.events.recv_timeout(Duration::from_millis(100)) {
            self.seen.push(event);
        }
    }
}

pub(super) fn request(kind: OpKind, from: &Path, names: &[&str], to: &Path) -> Request {
    Request {
        kind,
        from: from.to_owned(),
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        to: to.to_owned(),
        verify: false,
    }
}

pub(super) fn copy(from: &Path, names: &[&str], to: &Path) -> Request {
    request(OpKind::Copy, from, names, to)
}

pub(super) fn relocate(from: &Path, names: &[&str], to: &Path) -> Request {
    request(OpKind::Move, from, names, to)
}

pub(super) fn temp() -> TempDir {
    TempDir::new().expect("temp dir")
}

/// Writes `bytes` to `path`, making the folders above it.
pub(super) fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    fs::write(path, bytes).expect("file");
}

pub(super) fn read(path: &Path) -> Vec<u8> {
    fs::read(path).expect("file")
}

/// Every path below `dir`, relative and sorted, folders with a trailing `/`; links are not followed.
pub(super) fn tree(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    walk(dir, "", &mut found);
    found.sort();

    found
}

fn walk(dir: &Path, prefix: &str, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("folder") {
        let entry = entry.expect("entry");
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());

        if entry.file_type().expect("type").is_dir() {
            found.push(format!("{name}/"));
            walk(&entry.path(), &format!("{name}/"), found);
        } else {
            found.push(name);
        }
    }
}

/// Whether no temporary file of a copy is left below `dir`.
pub(super) fn has_no_partial_files(dir: &Path) -> bool {
    !tree(dir).iter().any(|path| path.ends_with(".part"))
}

/// A time long before any file here was made.
pub(super) fn long_ago() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
}

/// Sets the modification and access time of the file or folder `path` to `when`.
pub(super) fn set_times(path: &Path, when: SystemTime) {
    let times = FileTimes::new().set_modified(when).set_accessed(when);

    if fs::symlink_metadata(path).expect("metadata").is_dir() {
        open::folder(path)
            .expect("open folder")
            .set_times(times)
            .expect("times");
    } else {
        fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_times(times)
            .expect("times");
    }
}

pub(super) fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).expect("metadata").modified().expect("modified")
}

/// Bytes that differ from place to place, so a copy that shifts or repeats data cannot match by luck.
pub(super) fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| (index % 251) as u8 ^ (index / 4093) as u8)
        .collect()
}

/// The phases the job went through, as announced.
pub(super) fn phases(rig: &Rig) -> Vec<JobPhase> {
    rig.seen
        .iter()
        .filter_map(|event| match event {
            OpsEvent::Phase { phase, .. } => Some(*phase),
            _ => None,
        })
        .collect()
}

/// Puts every target folder on the volume named by the part of its name before the first `-`.
pub(super) fn volume_by_name(target: &Path) -> VolumeKey {
    let name = target.file_name().expect("a name").to_string_lossy();

    VolumeKey::Named(name.split('-').next().unwrap_or_default().to_owned())
}

pub(super) fn by_name() -> Hooks {
    Hooks {
        keyer: Some(Arc::new(volume_by_name)),
        ..Hooks::default()
    }
}

/// A job that stops on a conflict: its target folder `to` holds `f.txt` already.
pub(super) fn blocked(rig: &Rig, src: &Path, to: &Path) -> JobId {
    write(&to.join("f.txt"), b"old");

    rig.start(copy(src, &["f.txt"], to))
}

pub(super) fn view_of(engine: &Engine, job: JobId) -> JobView {
    let OpsEvent::Snapshot { jobs, .. } = engine.snapshot() else {
        unreachable!("a snapshot");
    };

    jobs.into_iter().find(|view| view.id == job).expect("the job is listed")
}

impl Rig {
    /// A rig whose volumes come from the target folders' names (see [`volume_by_name`]).
    pub(super) fn new_by_name() -> Self {
        Self::with(by_name(), |_, _| {})
    }
}

/// Writes `bytes` to `root/rel`, making the folders above it.
pub(super) fn put(root: &Path, rel: &str, bytes: &[u8]) {
    write(&root.join(rel), bytes);
}

/// A folder with `src/f.txt`, for jobs that stop on a conflict (see [`blocked`]); and the `src` folder.
pub(super) fn blocked_scene() -> (TempDir, PathBuf) {
    let root = temp();
    put(root.path(), "src/f.txt", b"new");
    let src = root.path().join("src");

    (root, src)
}

/// Another spelling of the existing `path`: its canonical form, with the `\\?\` prefix taken off. `None` when it
/// reads the same (ignoring case). On a Windows machine whose temporary folder is given by its 8.3 names (the CI
/// runners', `RUNNER~1`) a folder made there has two spellings, which no comparison of names sees through.
#[cfg(windows)]
pub(super) fn other_spelling(path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    let text = canonical.to_string_lossy();
    let plain = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));

    (plain.to_string_lossy().to_lowercase() != path.to_string_lossy().to_lowercase()).then_some(plain)
}

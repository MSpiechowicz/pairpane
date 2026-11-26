//! A server kept in a folder of the disk, for the tests of a download. It answers like a real one (whole seconds for
//! times, links as links), and a test makes it misbehave with [`Fake::switch`]: short reads, a read that hangs, a file
//! that grows or loses its time, a call that fails.

use std::collections::VecDeque;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use tempfile::TempDir;

use crate::model::{Answer, ConflictInfo, OpKind, Outcome, SiteId};
use crate::vfs::{RemoteEntry, RemoteFs, RemoteKind, RemoteMeta, RemotePath, RemoteReader};

use super::super::Request;
use super::super::testing::{Ended, Rig};

/// What a move says of an item it kept because its folder is not where the walk went.
pub(super) const FOLDER_CHANGED: &str = "Kept the original: the folder on the server changed while it was moved";

/// What a move says of a removal whose folder turned out not to be where the removal was asked.
pub(super) const REMOVED_ELSEWHERE: &str =
    "the folder on the server changed during the removal; something in another folder may have been removed";

/// When a file that a reader serves grows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Grow {
    #[default]
    Never,
    /// Once the reader has served all it had (the read that says "the end" still does).
    AtEnd,
    /// When the reader is dropped.
    OnClose,
}

/// How the server misbehaves; nothing by default.
#[derive(Default)]
pub(super) struct Switches {
    /// The most bytes one read returns.
    pub(super) short_reads: Option<usize>,
    /// Every read waits for a cancel.
    pub(super) hang_reads: bool,
    /// A reader fails (with `fail_kind`) once it served this many bytes.
    pub(super) fail_after: Option<u64>,
    pub(super) fail_kind: Option<io::ErrorKind>,
    pub(super) grow: Grow,
    /// No file has a time.
    pub(super) no_mtime: bool,
    pub(super) fail_remove: bool,
    /// A removal finds nothing there: the item was removed by someone else in the meantime.
    pub(super) remove_finds_nothing: bool,
    pub(super) panic_reads: bool,
    /// What the server says of the size of a file differs by this much from what it sends.
    pub(super) size_skew: i64,
    /// The failures of the next opens, one each (`None`: that open works).
    pub(super) open_errors: VecDeque<Option<io::ErrorKind>>,
    /// Items a listing holds besides the ones in the folder.
    pub(super) extra_entries: Vec<(String, RemoteKind)>,
    /// The server cannot say where a folder is (no `realpath`).
    pub(super) no_realpath: bool,
    /// A listing shows files as items of a kind that is neither a file nor a folder.
    pub(super) files_as_other: bool,
    /// Run once, by the first read of a file.
    pub(super) at_first_read: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The server only cleans a path when asked where it is (as some do), and follows no link.
    pub(super) lexical_realpath: bool,
    /// Run once, right after the server said where the folder named here is.
    pub(super) after_realpath: Option<(String, Arc<dyn Fn() + Send + Sync>)>,
}

#[derive(Default)]
struct Shared {
    switches: Mutex<Switches>,
    handles: AtomicUsize,
    biggest_read: AtomicUsize,
}

impl Shared {
    fn switches(&self) -> MutexGuard<'_, Switches> {
        self.switches.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The server: the folder `root` is its `/`.
pub(super) struct Fake {
    root: PathBuf,
    _own: Option<TempDir>,
    shared: Arc<Shared>,
}

impl Fake {
    /// A server of its own, empty.
    pub(super) fn new() -> Arc<Self> {
        let own = TempDir::new().expect("temp dir");

        Arc::new(Self {
            root: own.path().to_owned(),
            _own: Some(own),
            shared: Arc::default(),
        })
    }

    /// A server whose `/` is the existing folder `root`: it holds what is there, as a server on this very computer does.
    pub(super) fn over(root: &Path) -> Arc<Self> {
        Arc::new(Self {
            root: root.to_owned(),
            _own: None,
            shared: Arc::default(),
        })
    }

    /// The file or folder `rel` of the server, on this computer's disk.
    pub(super) fn at(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Puts the file `rel` on the server.
    pub(super) fn put(&self, rel: &str, bytes: &[u8]) {
        let path = self.at(rel);

        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(path, bytes).expect("file");
    }

    pub(super) fn switch(&self, set: impl FnOnce(&mut Switches)) {
        set(&mut self.shared.switches());
    }

    /// Files open for reading now.
    pub(super) fn handles(&self) -> usize {
        self.shared.handles.load(Ordering::SeqCst)
    }

    /// The most bytes a read was asked for.
    pub(super) fn biggest_read(&self) -> usize {
        self.shared.biggest_read.load(Ordering::SeqCst)
    }

    fn refuse_removal(&self) -> io::Result<()> {
        let switches = self.shared.switches();

        if switches.fail_remove {
            return Err(io::Error::other("the server refused to remove it"));
        }
        if switches.remove_finds_nothing {
            return Err(io::ErrorKind::NotFound.into());
        }

        Ok(())
    }

    fn on_disk(&self, path: &RemotePath) -> PathBuf {
        path.components().fold(self.root.clone(), |disk, name| disk.join(name))
    }

    fn describe(&self, meta: &Metadata) -> RemoteMeta {
        describe(meta, &self.shared.switches())
    }

    /// What the server answers for an item that was looked at (`None` when it is not there).
    fn seen(&self, looked: io::Result<Metadata>) -> io::Result<Option<RemoteMeta>> {
        match looked {
            Ok(meta) => Ok(Some(self.describe(&meta))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn describe(meta: &Metadata, switches: &Switches) -> RemoteMeta {
    let kind = if meta.is_symlink() {
        RemoteKind::Link
    } else if meta.is_dir() {
        RemoteKind::Dir
    } else if meta.is_file() {
        RemoteKind::File
    } else {
        RemoteKind::Other
    };
    let size = if kind == RemoteKind::File {
        u64::try_from(i64::try_from(meta.len()).expect("small") + switches.size_skew).unwrap_or(0)
    } else {
        0
    };
    let seconds = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok());

    RemoteMeta {
        kind,
        size,
        modified_ms: seconds
            .filter(|_| !switches.no_mtime)
            .map(|time| i64::try_from(time.as_secs()).expect("a time") * 1000),
        mode: mode_of(meta),
    }
}

#[cfg(unix)]
fn mode_of(meta: &Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_: &Metadata) -> Option<u32> {
    None
}

impl RemoteFs for Fake {
    fn site(&self) -> SiteId {
        SiteId(41)
    }

    fn label(&self) -> &str {
        "tester@fake"
    }

    fn read_dir(&self, dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(self.on_disk(dir))? {
            let entry = entry?;

            let mut meta = self.describe(&fs::symlink_metadata(entry.path())?);

            if meta.kind == RemoteKind::File && self.shared.switches().files_as_other {
                meta.kind = RemoteKind::Other;
            }

            entries.push(RemoteEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                meta,
            });
        }

        for (name, kind) in &self.shared.switches().extra_entries {
            let meta = RemoteMeta {
                kind: *kind,
                size: 1,
                modified_ms: Some(0),
                mode: Some(0o644),
            };

            entries.push(RemoteEntry {
                name: name.clone(),
                meta,
            });
        }

        Ok(entries)
    }

    fn stat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.seen(fs::metadata(self.on_disk(path)))
    }

    fn read_link(&self, path: &RemotePath) -> io::Result<String> {
        Ok(fs::read_link(self.on_disk(path))?.to_string_lossy().into_owned())
    }

    fn lstat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.seen(fs::symlink_metadata(self.on_disk(path)))
    }

    fn realpath(&self, path: &RemotePath) -> io::Result<RemotePath> {
        let (unsupported, lexical) = {
            let switches = self.shared.switches();

            (switches.no_realpath, switches.lexical_realpath)
        };

        if unsupported {
            return Err(io::ErrorKind::Unsupported.into());
        }

        let resolved = if lexical { Ok(path.clone()) } else { self.resolved(path) };
        let hook = {
            let mut switches = self.shared.switches();

            match &switches.after_realpath {
                Some((folder, _)) if folder == path.as_str() => switches.after_realpath.take(),
                _ => None,
            }
        };

        if let Some((_, hook)) = hook {
            hook();
        }

        resolved
    }

    fn open_read(&self, path: &RemotePath) -> io::Result<Box<dyn RemoteReader>> {
        if let Some(Some(kind)) = self.shared.switches().open_errors.pop_front() {
            return Err(kind.into());
        }

        let disk = self.on_disk(path);
        let file = File::open(&disk)?;
        self.shared.handles.fetch_add(1, Ordering::SeqCst);

        Ok(Box::new(Reader {
            file,
            disk,
            shared: Arc::clone(&self.shared),
            served: 0,
            started: false,
            grown: false,
        }))
    }

    fn remove_file(&self, path: &RemotePath) -> io::Result<()> {
        self.refuse_removal()?;

        fs::remove_file(self.on_disk(path))
    }

    fn remove_dir(&self, path: &RemotePath) -> io::Result<()> {
        self.refuse_removal()?;

        fs::remove_dir(self.on_disk(path))
    }
}

impl Fake {
    /// Where `path` is, with every link followed.
    fn resolved(&self, path: &RemotePath) -> io::Result<RemotePath> {
        let top = fs::canonicalize(&self.root)?;
        let found = fs::canonicalize(self.on_disk(path))?;
        let inside = found
            .strip_prefix(&top)
            .map_err(|_| io::Error::other("the path leads out of the server"))?;

        inside.components().try_fold(RemotePath::root(), |resolved, name| {
            resolved.join(&name.as_os_str().to_string_lossy())
        })
    }
}

/// A file of the server open for reading.
struct Reader {
    file: File,
    disk: PathBuf,
    shared: Arc<Shared>,
    served: u64,
    started: bool,
    grown: bool,
}

impl Reader {
    fn grow(&self) {
        let mut file = OpenOptions::new().append(true).open(&self.disk).expect("the file");
        file.write_all(b"more").expect("append");
    }
}

impl RemoteReader for Reader {
    fn read(&mut self, buf: &mut [u8], cancel: &AtomicBool) -> io::Result<usize> {
        self.shared.biggest_read.fetch_max(buf.len(), Ordering::SeqCst);

        let hook = {
            let mut switches = self.shared.switches();
            assert!(!switches.panic_reads, "the server broke");

            if self.started {
                None
            } else {
                switches.at_first_read.take()
            }
        };
        self.started = true;
        if let Some(hook) = hook {
            hook();
        }

        let (hang, short, fail, kind, grow) = {
            let switches = self.shared.switches();

            (
                switches.hang_reads,
                switches.short_reads,
                switches.fail_after,
                switches.fail_kind.unwrap_or(io::ErrorKind::ConnectionAborted),
                switches.grow,
            )
        };

        if hang {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                thread::sleep(Duration::from_millis(5));
            }
        }

        if fail.is_some_and(|limit| self.served >= limit) {
            return Err(kind.into());
        }

        let want = short.map_or(buf.len(), |most| most.min(buf.len()));
        let count = self.file.read(&mut buf[..want])?;
        self.served += count as u64;

        if count == 0 && grow == Grow::AtEnd && !self.grown {
            self.grown = true;
            self.grow();
        }

        Ok(count)
    }

    fn meta(&mut self) -> io::Result<RemoteMeta> {
        Ok(describe(&self.file.metadata()?, &self.shared.switches()))
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.shared.switches().grow == Grow::OnClose {
            self.grow();
        }

        self.shared.handles.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A download of `names` from the server folder `dir` into `to`.
pub(super) fn download(kind: OpKind, server: &Arc<Fake>, dir: &str, names: &[&str], to: &Path) -> Request {
    Request::download(
        kind,
        Arc::<Fake>::clone(server),
        RemotePath::new(dir).expect("path"),
        names.iter().map(|name| (*name).to_owned()).collect(),
        to.to_owned(),
        false,
    )
}

/// Downloads `names` from the root of the server into `to` and waits for the end of the job.
pub(super) fn fetch(rig: &mut Rig, kind: OpKind, server: &Arc<Fake>, names: &[&str], to: &Path) -> Ended {
    rig.run(download(kind, server, "/", names, to))
}

/// What the items of a job that failed say, in order.
pub(super) fn said(ended: &Ended) -> Vec<String> {
    ended.errors.iter().map(|item| item.error.message.clone()).collect()
}

/// The message a job that failed as a whole gives.
pub(super) fn failure(ended: &Ended) -> String {
    match &ended.outcome {
        Outcome::Failed { error } => error.message.clone(),
        other => panic!("the job did not fail: {other:?}"),
    }
}

/// Runs `request`, which asks one question, and answers it with `answer`. Returns the question and how the job ended.
pub(super) fn answered(rig: &mut Rig, request: Request, answer: Answer) -> (ConflictInfo, Ended) {
    let job = rig.start(request);
    let question = rig.answer_next(job, answer, false);

    (question, rig.wait_ended(job))
}

/// Makes the first read of a file from the server put `content` at `target` on this computer, as another program may.
pub(super) fn claim_meanwhile(server: &Fake, target: &Path, content: &'static [u8]) {
    let target = target.to_owned();

    server.switch(|switches| {
        switches.at_first_read = Some(Arc::new(move || fs::write(&target, content).expect("file")));
    });
}

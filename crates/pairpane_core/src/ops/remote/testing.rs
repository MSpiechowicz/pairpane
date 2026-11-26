//! A server kept in a folder of the disk, for the tests of a download. It answers like a real one (whole seconds for
//! times, links as links), and a test makes it misbehave with [`Fake::switch`]: short reads, a read that hangs, a file
//! that grows or loses its time, a call that fails.

use std::collections::VecDeque;
use std::fs::{self, File, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::UNIX_EPOCH;

use tempfile::TempDir;

use crate::model::{Answer, ConflictInfo, OpKind, Outcome, SiteId};
use crate::vfs::{RemoteEntry, RemoteFs, RemoteKind, RemoteMeta, RemotePath, RemoteReader};

use self::reader::Reader;

use super::super::Request;
use super::super::testing::{Ended, Rig};

mod reader;

/// What a move says of an item it kept because its folder is not where the walk went.
#[cfg(unix)]
pub(super) const FOLDER_CHANGED: &str = "Kept the original: the folder on the server changed while it was moved";

/// What a move says of a removal whose folder turned out not to be where the removal was asked.
#[cfg(unix)]
pub(super) const REMOVED_ELSEWHERE: &str =
    "the folder on the server changed during the removal; something in another folder may have been removed";

/// What a move says of a folder whose contents it could not list after the copy, because it is a link by then.
#[cfg(unix)]
pub(super) const NOT_LISTED: &str = "The folder was copied, but what is left in it could not be listed, so it stays on the server: it changed while it was being copied";

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
    /// The server cannot say where a folder is (no `realpath`), or where a link points (no `read_link`).
    pub(super) no_realpath: bool,
    pub(super) no_read_link: bool,
    /// Every listing from the n-th on fails (the first is number 1).
    pub(super) fail_listings_from: Option<usize>,
    /// Run once, when the n-th listing is asked for (the first is number 1), before the listing looks at a cancel.
    pub(super) at_listing: Option<(usize, Arc<dyn Fn() + Send + Sync>)>,
    /// Run once, right after the server removed a file.
    pub(super) after_remove: Option<Arc<dyn Fn() + Send + Sync>>,
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
    listings: AtomicUsize,
    looks: AtomicUsize,
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

    /// Calls a caller waited for, one after the other: a batch of looks at once counts as one.
    pub(super) fn round_trips(&self) -> usize {
        self.shared.looks.load(Ordering::SeqCst)
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

    /// What the server says of `path` itself, not counted as a call.
    fn look(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.seen(fs::symlink_metadata(self.on_disk(path)))
    }

    fn on_disk(&self, path: &RemotePath) -> PathBuf {
        // In one piece: a path of a thousand names is not built name by name for each of a million looks.
        self.root.join(path.as_str().trim_start_matches('/'))
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
        self.shared.looks.fetch_add(1, Ordering::SeqCst);
        let listing = self.shared.listings.fetch_add(1, Ordering::SeqCst) + 1;
        let hook = {
            let mut switches = self.shared.switches();

            match &switches.at_listing {
                Some((number, _)) if *number == listing => switches.at_listing.take(),
                _ => None,
            }
        };

        if let Some((_, hook)) = hook {
            hook();
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self
            .shared
            .switches()
            .fail_listings_from
            .is_some_and(|from| listing >= from)
        {
            return Err(io::Error::other("the server could not list the folder"));
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
        self.shared.looks.fetch_add(1, Ordering::SeqCst);

        if self.shared.switches().no_read_link {
            return Err(io::ErrorKind::Unsupported.into());
        }

        Ok(fs::read_link(self.on_disk(path))?.to_string_lossy().into_owned())
    }

    fn lstat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        self.shared.looks.fetch_add(1, Ordering::SeqCst);

        self.look(path)
    }

    /// Asked in parts at once, as a server that has several requests in flight answers a way of folders.
    fn lstat_many(&self, paths: &[RemotePath], cancel: &AtomicBool) -> Vec<io::Result<Option<RemoteMeta>>> {
        self.shared.looks.fetch_add(1, Ordering::SeqCst);

        let parts = thread::available_parallelism().map_or(1, usize::from).min(8);
        let each = paths.len().div_ceil(parts).max(32);

        thread::scope(|scope| {
            let asked: Vec<_> = paths
                .chunks(each)
                .map(|part| {
                    scope.spawn(|| {
                        part.iter()
                            .map(|path| {
                                if cancel.load(Ordering::Relaxed) {
                                    return Err(io::ErrorKind::Interrupted.into());
                                }

                                self.look(path)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();

            asked
                .into_iter()
                .flat_map(|part| part.join().expect("a thread of the server"))
                .collect()
        })
    }

    fn realpath(&self, path: &RemotePath) -> io::Result<RemotePath> {
        self.shared.looks.fetch_add(1, Ordering::SeqCst);

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

        Ok(Box::new(Reader::new(file, disk, Arc::clone(&self.shared))))
    }

    fn remove_file(&self, path: &RemotePath) -> io::Result<()> {
        self.refuse_removal()?;

        let removed = fs::remove_file(self.on_disk(path));
        let hook = self.shared.switches().after_remove.take();

        if let Some(hook) = hook {
            hook();
        }

        removed
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

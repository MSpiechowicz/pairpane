//! Packing files into a new archive as a job: the items of the source folder (folders with all below them) are written
//! into a zip, a tar.zst or a 7z in the target folder.
//!
//! The tree is walked first (see `tree`: no link is followed, names in order, a folder before what is in it) and the
//! entries are planned: a file's size and mode and time, and how to open it again. A link or a special file is not packed
//! but counted as skipped, an empty folder is kept, and what the archive readers would refuse (a name that is not text or
//! not a normalized relative path) fails that item and the job goes on. A tree with more entries or more bytes of names
//! than the readers index fails the job before anything is written, and so does one that holds nothing to pack. A file
//! that cannot be opened is an item that fails too; opening it again later, to write it, is the race the
//! writers report (a file that changed size fails the job).
//!
//! Then the target is decided: when the archive's name is taken the user is asked (see `conflict`) before the first byte is
//! written. The archive is written to a temporary file next to its target (the disk keeps its reserve, see
//! `extract_write`), flushed to the disk and put in place with the no-replace rename (or the replacing one when the answer
//! was to overwrite); a cancel or any failure removes the temporary file.
//!
//! The progress counts the bytes of the files as the writer reads them, through a reader that also stops for a pause and
//! raises the writer's cancel flag on a cancel.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::archive::write::{Entry, Lazy, Options, Packer};
use crate::archive::{MAX_INDEX_ENTRIES, MAX_INDEX_NAME_BYTES, normalize};
use crate::model::{FileFacts, JobPhase};

use super::PackJob;
use super::conflict::Place;
use super::extract_write::CHECK_EVERY;
use super::held::{Held, Node};
use super::partial::{Partial, run_token};
use super::report::refusal;
use super::task::{as_text, create_plain};
use super::transfer::{Run, Stop};
use super::tree::{Reopen, Spot, Visitor};

/// Bytes the output is gathered up to before it goes to the file.
pub(super) const OUTPUT_BUFFER: usize = 256 * 1024;

/// What an entry of the archive is made from.
pub(super) enum Source {
    Folder,
    File { size: u64, open: Reopen },
}

/// One entry to write: its name in the archive, its mode and time, and its source.
pub(super) struct Planned {
    pub(super) name: String,
    pub(super) mode: u32,
    pub(super) modified: Option<i64>,
    pub(super) source: Source,
}

/// The walk that plans the entries (see [`Run::walk_items`]).
pub(super) struct Plan {
    root: Arc<Held>,
    pub(super) entries: Vec<Planned>,
    /// The bytes of the files.
    pub(super) bytes: u64,
    /// The bytes of the names.
    names: usize,
    /// The most entries, and the most bytes of names, the archive takes.
    limits: (usize, usize),
    /// What the names of this job's own temporary files start with.
    own: String,
    /// What every name in the archive starts with: the folder the items are put in (`a/b/`), or nothing.
    prefix: String,
    /// The limits were passed: that ends the job, not the item.
    refused: bool,
}

impl Plan {
    /// A plan of the items of the folder `root` for the job `run`, whose entries are put under `prefix` (`a/b/` or
    /// nothing) and which takes at most `limits` entries and bytes of names.
    pub(super) fn new(run: &Run<'_>, root: &Arc<Held>, prefix: String, limits: (usize, usize)) -> Self {
        Self {
            root: Arc::clone(root),
            entries: Vec::new(),
            bytes: 0,
            names: 0,
            limits,
            own: format!(".pairpane-{}-{}-", run_token(), run.job.id.0),
            prefix,
            refused: false,
        }
    }

    /// The name in the archive of the node `at`.
    pub(super) fn name_of(&self, at: &Spot<'_>) -> String {
        format!("{}{}", self.prefix, at.rel)
    }

    /// Takes the node `at` as an entry, or fails: its name must be text, a plain relative path of the kind an archive can
    /// be read back from, and one more entry must still fit. Gives its name in the archive.
    fn admit(&mut self, at: &Spot<'_>) -> Result<String, Stop> {
        as_text(&at.node.name)?;

        let name = self.name_of(at);
        if !matches!(normalize(&name), Ok(Some(ref plain)) if *plain == name) {
            return Err(refusal(io::ErrorKind::InvalidInput, "the name cannot be stored in an archive").into());
        }

        let (most_entries, most_names) = self.limits;
        if self.entries.len() >= most_entries || self.names.saturating_add(name.len()) > most_names {
            self.refused = true;

            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "there are too many files to pack in one archive",
            )
            .into());
        }

        self.names += name.len();

        Ok(name)
    }

    fn push(&mut self, name: String, at: &Spot<'_>, source: Source) {
        self.entries.push(Planned {
            name,
            mode: entry_mode(at.node.mode, matches!(source, Source::Folder)),
            modified: at.node.modified,
            source,
        });
    }
}

/// The permission bits an entry is stored with: those of the item, or the usual ones where the system has none.
fn entry_mode(mode: u32, folder: bool) -> u32 {
    match (cfg!(windows), folder) {
        (false, _) => mode & 0o777,
        (true, true) => 0o755,
        (true, false) => 0o644,
    }
}

impl Visitor for Plan {
    fn file(&mut self, run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop> {
        // The temporary file of this very job, when the archive is made inside the folder that is packed.
        if at.node.name.to_string_lossy().starts_with(&self.own) {
            return Ok(());
        }

        let name = self.admit(at)?;
        // A file that cannot be read fails here, as an item, and not in the middle of the archive.
        at.parent.open_file(at.node)?;

        run.found(at.node.size);
        self.bytes = self.bytes.saturating_add(at.node.size);
        self.push(
            name,
            at,
            Source::File {
                size: at.node.size,
                open: Reopen::new(&self.root, at),
            },
        );

        Ok(())
    }

    fn folder(&mut self, _run: &mut Run<'_>, at: &Spot<'_>) -> Result<(), Stop> {
        let name = self.admit(at)?;
        self.push(name, at, Source::Folder);

        Ok(())
    }

    fn fatal(&self) -> bool {
        self.refused
    }
}

/// The job, shared between what the writer reads from and writes to (which the job cannot hold both at once), so that
/// each can report the progress and look at a pause or a cancel. The writer calls one at a time, never one inside another.
pub(super) type Shared<'r, 'a, 'b> = &'r RefCell<&'a mut Run<'b>>;

/// A file as the writer reads it: counted in the progress, and stopped by a pause or a cancel (which also raises the
/// writer's flag, so what the writer does between two reads stops too).
struct Counted<'r, 'a, 'b, R> {
    run: Shared<'r, 'a, 'b>,
    cancel: &'r AtomicBool,
    name: &'r str,
    size: u64,
    /// Bytes read so far.
    seen: u64,
    from: R,
    begun: bool,
    ended: bool,
}

impl<R: Read> Read for Counted<'_, '_, '_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut run = self.run.borrow_mut();

        if run.checkpoint().is_err() {
            self.cancel.store(true, Ordering::Relaxed);

            return Err(refusal(io::ErrorKind::Other, "the job was cancelled"));
        }

        if !self.begun {
            self.begun = true;
            run.begin_file(self.name, self.size);
        }

        let count = loop {
            match self.from.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                done => break done?,
            }
        };

        // The file is over when its declared size is read (a tar never asks for more) or when it ends.
        if !self.ended {
            self.seen += count as u64;
            run.add_bytes(count as u64);

            if count == 0 || self.seen >= self.size {
                self.ended = true;
                run.end_file();
            }
        }

        Ok(count)
    }
}

/// The temporary file as the writer writes it: the disk keeps its reserve (see `extract_write`).
pub(super) struct Output<'r, 'a, 'b> {
    file: File,
    run: Shared<'r, 'a, 'b>,
    folder: &'r Path,
    /// Bytes written since the free space was looked at.
    unchecked: u64,
}

impl<'r, 'a, 'b> Output<'r, 'a, 'b> {
    /// The new file `file`, which is in `folder`, written for the job `run`.
    pub(super) fn new(file: File, run: Shared<'r, 'a, 'b>, folder: &'r Path) -> Self {
        Self {
            file,
            run,
            folder,
            // The free space is looked at before the first write.
            unchecked: CHECK_EVERY,
        }
    }
}

/// Ends the archive written through `output`: what is buffered goes to the file, which is flushed to the disk.
pub(super) fn sync_output(output: BufWriter<Output<'_, '_, '_>>) -> io::Result<()> {
    let output = output.into_inner().map_err(io::IntoInnerError::into_error)?;

    output.file.sync_all()
}

/// The entry that writes `planned`: its file is read as the writer asks, counted in the progress of `run`, and a cancel
/// also raises `cancel`, which the writer looks at.
pub(super) fn entry_of<'r>(run: Shared<'r, '_, '_>, cancel: &'r AtomicBool, planned: &'r Planned) -> Entry<'r> {
    match &planned.source {
        Source::Folder => Entry::folder(&planned.name, planned.mode, planned.modified),
        Source::File { size, open } => {
            let from = Counted {
                run,
                cancel,
                name: &planned.name,
                size: *size,
                seen: 0,
                from: Lazy::new(|| open.open()),
                begun: false,
                ended: false,
            };

            Entry::file(&planned.name, planned.mode, planned.modified, *size, from)
        }
    }
}

/// An empty file is never read, so it is finished as soon as it is added.
pub(super) fn count_empty(run: Shared<'_, '_, '_>, planned: &Planned) {
    if matches!(planned.source, Source::File { size: 0, .. }) {
        let mut run = run.borrow_mut();

        run.begin_file(&planned.name, 0);
        run.end_file();
    }
}

impl Write for Output<'_, '_, '_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.run
            .borrow_mut()
            .keep_reserve(&mut self.unchecked, self.folder, bytes.len() as u64)?;

        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Output<'_, '_, '_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.file.seek(to)
    }
}

/// The time now, as what a new archive is modified at.
fn now_millis() -> Option<i64> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;

    i64::try_from(since.as_millis()).ok()
}

impl Run<'_> {
    /// Runs the pack job: the archive `name` in the request's target folder, written as `options` say (see the module's
    /// text).
    pub(super) fn pack(&mut self, name: &str, options: &PackJob) -> Result<(), Stop> {
        let job = self.job;
        let request = &job.request;

        let (root, items) = self.sorted_items()?;
        let to = self.target_folder(&request.to)?;

        self.phase(JobPhase::Scanning);
        let plan = self.plan_entries(&root, &items)?;

        // A job whose every item failed has nothing to write: the items' errors say why.
        if plan.entries.is_empty() {
            return Err(refusal(io::ErrorKind::InvalidInput, "there is nothing to pack").into());
        }

        let facts = FileFacts {
            size: plan.bytes,
            modified: now_millis(),
            dir: false,
        };
        let mut place = self.place_entry(name, &facts, &to.join(name))?;
        if place == Place::Skip {
            self.skipped += 1;

            return Ok(());
        }

        self.phase(JobPhase::Packing);
        let (temp, file) = self.make_temp(&to, create_plain)?;
        let mut guard = Partial::new(temp.clone());

        self.write_archive(&plan, options, file, &to)?;
        self.checkpoint()?;

        if self.commit_with(&temp, &mut place, |run, again| run.place_entry(name, &facts, again))? {
            guard.committed = true;
        } else {
            self.skipped += 1;
        }

        Ok(())
    }

    /// Walks the `items` of the folder `root` and plans the entries.
    fn plan_entries(&mut self, root: &Arc<Held>, items: &[Node]) -> Result<Plan, Stop> {
        let mut plan = Plan::new(self, root, String::new(), self.pack_limits());

        self.walk_items(root, items, &mut plan)?;

        Ok(plan)
    }

    /// The folder of the job's items, held, and the items in it in name order: the order of the entries of an archive does
    /// not depend on the order the items were selected in.
    pub(super) fn sorted_items(&mut self) -> Result<(Arc<Held>, Vec<Node>), Stop> {
        let request = &self.job.request;

        let from = self.open_folders(&[&request.from, &request.to])?;
        let root = Arc::new(Held::root(&from)?);
        let mut items = self.look_up_items(&root);
        items.sort_by(|a, b| a.name.cmp(&b.name));

        Ok((root, items))
    }

    /// The most entries, and the most bytes of names, an archive takes.
    pub(super) fn pack_limits(&self) -> (usize, usize) {
        self.inner
            .hooks
            .pack_limits
            .unwrap_or((MAX_INDEX_ENTRIES, MAX_INDEX_NAME_BYTES))
    }

    /// Writes the planned entries into `file`, which is in the folder `folder`, and flushes it to the disk.
    fn write_archive(&mut self, plan: &Plan, options: &PackJob, file: File, folder: &Path) -> Result<(), Stop> {
        let cancel = AtomicBool::new(false);
        let writing = Options {
            level: options.level,
            password: options.password.as_deref().cloned(),
            encrypt_names: options.encrypt_names,
        };

        let run = RefCell::new(&mut *self);
        let stop = |error: io::Error| run.borrow().stopped(error);

        let output = Output::new(file, &run, folder);
        let mut packer = Packer::new(
            options.format,
            BufWriter::with_capacity(OUTPUT_BUFFER, output),
            &writing,
            &cancel,
        )
        .map_err(stop)?;

        for planned in &plan.entries {
            run.borrow_mut().checkpoint()?;

            packer.add(entry_of(&run, &cancel, planned)).map_err(stop)?;
            count_empty(&run, planned);
        }

        sync_output(packer.finish().map_err(stop)?).map_err(stop)?;

        Ok(())
    }
}

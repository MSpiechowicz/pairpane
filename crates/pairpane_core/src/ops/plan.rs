//! Looking before copying: checking the paths, listing a folder without following links, refusing a folder
//! copied into itself, and counting what there is to copy.

use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::listing::platform;

use super::Task;
use super::identity::FileId;
use super::report::refusal;
use super::transfer::{MAX_DEPTH, Run, Stop};
use super::validate::is_item_path;
use super::volume;

/// What an entry of a folder is. A link is a link whatever it points at: nothing here follows one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Dir,
    File,
    Link,
    /// A pipe, a socket or a device: never opened (a pipe could block the worker for good).
    Other,
}

/// One entry of a source folder, as the walk saw it.
#[derive(Debug)]
pub(crate) struct Child {
    pub(crate) name: OsString,
    pub(crate) path: PathBuf,
    pub(crate) kind: Kind,
    /// Read without following links.
    pub(crate) meta: Metadata,
}

impl Child {
    fn new(path: PathBuf, name: OsString, meta: Metadata) -> Self {
        let file_type = meta.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Link
        } else if file_type.is_dir() {
            Kind::Dir
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };

        Self { name, path, kind, meta }
    }

    /// The top-level item `name` of the source folder `from`; fails when it is gone.
    pub(crate) fn lookup(from: &Path, name: &str) -> io::Result<Self> {
        let path = from.join(name);
        let meta = fs::symlink_metadata(&path)?;

        Ok(Self::new(path, OsString::from(name), meta))
    }

    /// The name as the job reports it (for a top-level item, its path relative to the source folder).
    pub(super) fn rel(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }
}

/// The entries of one folder.
pub(crate) struct Listing {
    pub(crate) children: Vec<Child>,
    /// Entries that could not be read.
    pub(crate) failed: Vec<io::Error>,
    /// Entries left out because Windows would read their name as something else.
    pub(crate) skipped: u32,
}

/// Lists the folder `dir` without following any link. The folder is closed again before the caller goes into
/// its subfolders, so a deep tree does not hold a descriptor per level.
pub(crate) fn read_children(dir: &Path) -> io::Result<Listing> {
    let mut listing = Listing {
        children: Vec::new(),
        failed: Vec::new(),
        skipped: 0,
    };

    for entry in fs::read_dir(dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                listing.failed.push(error);
                continue;
            }
        };
        let name = entry.file_name();

        // Windows opens `x.` as `x`, which may be a link to a network share (the rule of the listing too).
        if platform::is_unsafe_name(&name.to_string_lossy()) {
            listing.skipped += 1;
            continue;
        }

        match entry.metadata() {
            Ok(meta) => listing.children.push(Child::new(entry.path(), name, meta)),
            Err(error) => listing.failed.push(error),
        }
    }

    Ok(listing)
}

/// Whether `inner` is `outer` or below it, comparing names whole, and ignoring case where the platform does.
pub(crate) fn is_inside(inner: &Path, outer: &Path) -> bool {
    is_inside_with(inner, outer, platform::CASE_INSENSITIVE)
}

pub(super) fn is_inside_with(inner: &Path, outer: &Path, ignore_case: bool) -> bool {
    let key = |component: Component<'_>| {
        let text = component.as_os_str().to_string_lossy();
        let text = if cfg!(windows) {
            text.trim_start_matches(r"\\?\")
        } else {
            &text
        };

        if ignore_case {
            text.to_lowercase()
        } else {
            text.to_owned()
        }
    };

    let mut inner = inner.components().map(key);

    outer
        .components()
        .map(key)
        .all(|part| inner.next().is_some_and(|other| other == part))
}

/// The path with its links resolved, for comparing where two paths lead. On Windows `existing` was resolved
/// already, link by link, with the checks that keep it off network paths.
pub(crate) fn real(existing: &Path) -> io::Result<PathBuf> {
    if cfg!(windows) {
        Ok(existing.to_owned())
    } else {
        fs::canonicalize(existing)
    }
}

/// Whether the folder `item`, copied into `existing/missing` (the part of the target that is there, and the folders
/// to make below it), would land on itself or inside itself: its place in the target is `item`, or the target
/// folder or one above it is `item`. Compared by identity, so another spelling of the same folder does not
/// get past (`existing` has its links resolved). The folders still to make cannot be `item`. `source` is the
/// identity of `item`.
pub(super) fn lands_in_itself(item: &Child, source: &FileId, existing: &Path, missing: &Path) -> bool {
    let is_item = |folder: &Path| FileId::of_path(folder).as_ref() == Some(source);

    is_item(&existing.join(missing).join(&item.name)) || existing.ancestors().any(is_item)
}

impl Run<'_> {
    /// Checks everything that can be checked before the first write, and makes the target folder. Returns the
    /// target folder and the items that are there to copy. An item that is gone is reported and left out.
    ///
    /// # Errors
    ///
    /// Fails the job when a path is not acceptable, the source is not a folder, the target is not (or cannot
    /// be made into) one, or a folder would be copied into itself.
    pub(super) fn prepare(&mut self) -> Result<(PathBuf, Vec<Child>), Stop> {
        let request = &self.job.request;
        let (_, items) = self.open_items(&[&request.from, &request.to])?;

        let (existing, missing) = volume::existing_part(&request.to)?
            .ok_or_else(|| refusal(io::ErrorKind::NotFound, "the target folder cannot be reached"))?;
        let real_existing = real(&existing)?;
        let real_target = real_existing.join(&missing);

        // The folder lands on itself, or in itself, when its place in the target is itself or below itself: by
        // identity first (a name can be spelled many ways on Windows), then by name.
        for item in items.iter().filter(|item| item.kind == Kind::Dir) {
            // Without its identity the folder cannot be told from the target, so nothing is copied.
            let Some(source) = self.id_of_meta(&item.path, &item.meta) else {
                return Err(refusal(
                    io::ErrorKind::PermissionDenied,
                    "could not tell whether the folder would be copied or moved into itself",
                )
                .into());
            };

            if lands_in_itself(item, &source, &real_existing, &missing)
                || is_inside(&real_target.join(&item.name), &real(&item.path)?)
            {
                return Err(refusal(
                    io::ErrorKind::InvalidInput,
                    "a folder cannot be copied or moved into itself",
                )
                .into());
            }
        }

        let to = existing.join(&missing);
        self.create_folders(&existing, &missing)?;

        let target = fs::metadata(&to)?;
        if !target.is_dir() {
            return Err(refusal(io::ErrorKind::InvalidInput, "the target is not a folder").into());
        }
        self.target_id = FileId::of_meta(&to, &target);

        Ok((to, items))
    }

    /// Checks the paths a job starts with (`folders` must pass `check_folder`, and the first one is the source),
    /// resolves the source folder and looks up the items in it. An item that is gone is reported and left out.
    /// Returns the resolved source folder and the items.
    pub(super) fn open_items(&mut self, folders: &[&Path]) -> Result<(PathBuf, Vec<Child>), Stop> {
        let from = self.open_folders(folders)?;
        let items = self.look_up_children(&from);

        Ok((from, items))
    }

    /// Checks the paths a job starts with (`folders` must pass `check_folder`) and the names of its items, and resolves
    /// the source folder (the job's `from`), which must be a folder.
    pub(super) fn open_folders(&mut self, folders: &[&Path]) -> Result<PathBuf, Stop> {
        let request = &self.job.request;

        for folder in folders {
            platform::check_folder(folder).map_err(|reason| refusal(io::ErrorKind::InvalidInput, reason))?;
        }
        let spelled = |name: &String| match request.task {
            Task::Pack { .. } | Task::Sync { .. } => is_item_path(name),
            _ => platform::is_plain_name(name),
        };
        if !request.names.iter().all(spelled) {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "an item has a name that cannot be used here",
            )
            .into());
        }

        let from = platform::local_path(&request.from)?;
        if !fs::metadata(&from)?.is_dir() {
            return Err(refusal(io::ErrorKind::InvalidInput, "the source is not a folder").into());
        }

        Ok(from)
    }

    /// Looks up the items of the job in `from` by path; one that is gone is reported and left out.
    fn look_up_children(&mut self, from: &Path) -> Vec<Child> {
        let job = self.job;
        let mut items = Vec::with_capacity(job.request.names.len());

        for name in &job.request.names {
            match Child::lookup(from, name) {
                Ok(child) => items.push(child),
                Err(error) => self.fail(name, &error),
            }
        }

        items
    }

    /// Lists the folder `node` for a walk: what could not be read is reported under `rel` and what was left out is
    /// counted as skipped; the entries are returned.
    pub(super) fn children_of(&mut self, node: &Child, rel: &str) -> Result<Vec<Child>, Stop> {
        let listing = read_children(&node.path)?;

        self.skipped += listing.skipped;
        for error in &listing.failed {
            self.fail(rel, error);
        }

        Ok(listing.children)
    }

    /// Settles the walk of the item at `rel`, which came to `done`: a failure is reported against it and the walk goes
    /// on; a cancel ends the walk.
    pub(super) fn settle<T>(&mut self, rel: &str, done: Result<T, Stop>) -> Result<(), Stop> {
        match done {
            Ok(_) => Ok(()),
            Err(Stop::Failed(error)) => {
                self.fail(rel, &error);

                Ok(())
            }
            Err(Stop::Cancelled) => Err(Stop::Cancelled),
        }
    }

    /// Makes the folders of `missing`, one below the other, under `existing`.
    pub(super) fn create_folders(&mut self, existing: &Path, missing: &Path) -> io::Result<()> {
        let mut path = existing.to_owned();

        for part in missing.components() {
            path.push(part);
            self.make_folder(&path, false)?;
        }

        Ok(())
    }

    /// Counts the files, links and bytes that the copy will move, so the progress has its total.
    pub(super) fn scan(&mut self, items: &[Child]) -> Result<(), Stop> {
        self.visited.clear();

        for item in items {
            self.checkpoint()?;
            self.scan_node(item, 0)?;
        }

        self.visited.clear();

        Ok(())
    }

    fn scan_node(&mut self, node: &Child, depth: usize) -> Result<(), Stop> {
        match node.kind {
            Kind::File => self.found(node.meta.len()),
            Kind::Link => self.found(0),
            Kind::Other => {}
            Kind::Dir => {
                if depth >= MAX_DEPTH || !self.first_visit(node) {
                    return Ok(());
                }

                // What cannot be read now is reported by the copy, which reads it again.
                let Ok(listing) = read_children(&node.path) else {
                    return Ok(());
                };

                for child in &listing.children {
                    self.checkpoint()?;
                    self.scan_node(child, depth + 1)?;
                }
            }
        }

        Ok(())
    }

    /// Whether the walk may enter the folder `node`: not the target folder (it could be reached through a bind
    /// mount, and copying it into itself never ends), and not one walked already.
    pub(super) fn first_visit(&mut self, node: &Child) -> bool {
        let Some(id) = FileId::of_meta(&node.path, &node.meta) else {
            return true;
        };

        if self.target_id.as_ref() == Some(&id) {
            return false;
        }

        // Windows keeps no set: an identity there is an open handle, one per folder would pile up, and it has no
        // mount tricks that show one folder at two places.
        cfg!(windows) || self.visited.insert(id)
    }
}

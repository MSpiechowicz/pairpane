//! The removals of a synchronization that mirrors: items only the side being written to has, removed so that it matches
//! the other side.
//!
//! Like a copy, a removal trusts nothing the scan saw. Right before an item is removed it is looked at again and must still
//! be what the scan saw: the same kind, and for a file the same size and time to the millisecond. For a folder its whole
//! subtree is walked again, depth first (all of it, hidden entries too, no link followed): it must hold the same number of
//! files and bytes. Nothing, the item and all in it, may have been modified or changed after the start of the scan (less
//! [`TIME_SLACK`], for file systems with coarse times; on Unix the `ctime` is looked at too, which a tool that puts a time
//! back cannot hide), which catches a file edited in place, an entry added, removed or renamed, and a time in the future.
//! An item whose name the source folder has by now is kept as well (an archive source is not looked at again: nothing of
//! it is read for a removal), and so is one that is the archive being compared or holds it (by identity on Unix, by path
//! elsewhere). What is kept is counted as skipped.
//!
//! The item goes to the Trash (the Recycle Bin on Windows) unless the job was asked to delete for good. A Trash that refuses
//! the item fails the item: it is never deleted for good instead. A folder that is the top of a mounted drive, or holds
//! one (a folder on another device, or a mount root on Linux), is refused whole before anything of it goes, to the Trash
//! too.
//!
//! On Unix every step goes by handle, as the writes of a copy do (see `sync_write`): the side's folder is opened once for the
//! job (its path, resolved, must name what was opened), each folder on the way is opened from the one above with no link
//! followed, and the item is looked at, walked and removed through the handle of its folder (`unlinkat`), or renamed into
//! the Trash from that handle (freedesktop), so a folder swapped for a link after the look leads nowhere outside the
//! compared folders. A folder deleted for good is emptied through handles too; it stops, with what it removed gone, at a
//! mount that appears meanwhile, at an entry changed after the scan, and when the job is cancelled. macOS takes a path for
//! the Trash: the path must lead to the held folder right before the move. Windows goes by path throughout (the system's
//! removal never enters a mounted volume or a link): a folder swapped in between is the race the other Windows jobs
//! document.
//!
//! The side's folder itself is found by its path when the removals start, as it is for the copies: a folder swapped for a
//! link to another one since the scan is entered (what is in it must still match the scan item by item). The last look and
//! the removal are two steps: an item swapped for another inside its folder in between still goes.

use std::io;
use std::path::{Path, PathBuf};

use crate::model::{EntryKind, JobPhase, SyncFacts};

use super::SyncRemove;
use super::held::{Held, Node};
use super::hooks::{Hooks, OpenHook};
use super::plan::{Kind, is_inside, real};
use super::report::refusal;
use super::sync::{lookup_below, same_facts, split_item};
use super::transfer::{MAX_DEPTH, Run, Stop};
use super::trash;

/// What the refusal of a folder that holds a mounted drive says. It holds no path.
const HOLDS_A_MOUNT: &str = "contains a mounted drive or volume; unmount or delete it separately";

/// One removal of the job: the item's path, what the scan saw of it, the folder of its side, and the folder of the other
/// side (the source of the mirror) when that is a folder on disk.
pub(super) struct Removal<'a> {
    pub(super) rel: &'a str,
    pub(super) removal: &'a SyncRemove,
    pub(super) root: &'a Path,
    pub(super) source: Option<&'a Path>,
}

/// How much earlier than the scan's start a time may be and still count as changed after it: file systems keep times as
/// coarse as 2 s (FAT), so a change just after the start can carry a time just before it.
const TIME_SLACK: i64 = 2_000;

/// The folder of a side, held open for the whole job, and where it really is.
struct Root {
    path: PathBuf,
    real: PathBuf,
    held: Held,
}

impl Root {
    /// Opens the side's folder `path`; on Unix the path, resolved, must name the folder that was opened.
    fn open(path: &Path) -> io::Result<Self> {
        let root = Self {
            path: path.to_owned(),
            real: real(path)?,
            held: Held::root(path)?,
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let (named, held) = (std::fs::metadata(&root.real)?, root.held.dir.metadata()?);
            if (named.dev(), named.ino()) != (held.dev(), held.ino()) {
                return Err(refusal(
                    io::ErrorKind::Other,
                    "the folder changed while it was being walked",
                ));
            }
        }

        Ok(root)
    }
}

/// The archive of the other side, when it is one: where it is, and (Unix) what it is.
struct Archive {
    real: PathBuf,
    #[cfg(unix)]
    id: Option<(u64, u64)>,
}

impl Archive {
    fn of(path: &Path) -> io::Result<Self> {
        Ok(Self {
            real: real(path)?,
            #[cfg(unix)]
            id: {
                use std::os::unix::fs::MetadataExt;

                std::fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
            },
        })
    }

    /// Whether `node` is this archive (Unix: by identity; elsewhere the path tells).
    fn is(&self, node: &Node) -> bool {
        #[cfg(unix)]
        {
            node.kind == Kind::File && self.id.is_some() && node.id == self.id
        }

        #[cfg(not(unix))]
        {
            let _ = node;

            false
        }
    }
}

impl Run<'_> {
    /// Removes each of `removals` that is still as the scan saw it: see the module's text. `archive` is the archive file
    /// of the other side, when that side is one.
    pub(super) fn sync_removals(
        &mut self,
        removals: &[Removal<'_>],
        permanent: bool,
        archive: Option<&Path>,
    ) -> Result<(), Stop> {
        if removals.is_empty() {
            return Ok(());
        }

        self.phase(JobPhase::Removing);
        let archive = archive.map(Archive::of).transpose()?;
        let mut roots: Vec<Root> = Vec::new();

        for removal in removals {
            self.checkpoint()?;

            let index = match roots.iter().position(|root| root.path == removal.root) {
                Some(index) => index,
                None => match Root::open(removal.root) {
                    Ok(root) => {
                        roots.push(root);
                        roots.len() - 1
                    }
                    Err(error) => {
                        self.fail(removal.rel, &error);
                        continue;
                    }
                },
            };

            let done = self.remove_item(&roots[index], removal, permanent, archive.as_ref());
            self.settle(removal.rel, done)?;
        }

        Ok(())
    }

    /// Removes the item of `removal` below `root` when it may go (see the module's text); counts it as skipped when not.
    fn remove_item(
        &mut self,
        root: &Root,
        removal: &Removal<'_>,
        permanent: bool,
        archive: Option<&Archive>,
    ) -> Result<(), Stop> {
        let Removal {
            rel,
            removal: seen,
            source,
            ..
        } = *removal;
        let since = seen.scanned.saturating_sub(TIME_SLACK);

        if let Some(source) = source
            && lookup_below(source, rel)?.is_some()
        {
            return self.changed();
        }
        if archive.is_some_and(|archive| is_inside(&archive.real, &root.real.join(rel))) {
            return self.changed();
        }

        let (parents, name) = split_item(rel);
        let Some((entered, path)) = enter(root, parents, self.inner.hooks.before_open.as_ref())? else {
            return self.changed();
        };
        let folder = entered.as_ref().unwrap_or(&root.held);
        let node = match folder.look(name) {
            Ok(node) => node,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return self.changed(),
            Err(error) => return Err(error.into()),
        };
        if !same_facts(Some(&facts_of(&node)), Some(&seen.seen))
            || is_newer(&node, since)
            || archive.is_some_and(|archive| archive.is(&node))
        {
            return self.changed();
        }

        if node.kind == Kind::Dir {
            let held = folder.subfolder(&node)?;
            // The item itself may be the top of a mounted drive: what is on it is never removed or moved to the Trash.
            if is_mount(&held, &node, (device_of(folder)?, &self.inner.hooks))? {
                return Err(refusal(io::ErrorKind::InvalidInput, HOLDS_A_MOUNT).into());
            }

            let mut found = Survey::default();
            let depth = rel.split('/').count();
            if survey(
                &held,
                (node.device, &self.inner.hooks),
                depth,
                (since, archive),
                &mut found,
            )?
            .is_none()
                || (found.files, found.bytes) != (seen.files, seen.bytes)
            {
                return self.changed();
            }
            if found.mount {
                return Err(refusal(io::ErrorKind::InvalidInput, HOLDS_A_MOUNT).into());
            }
        }

        if permanent {
            self.remove_held(folder, &node, since)?;
        } else if self.inner.hooks.refuse_trash {
            return Err(refusal(io::ErrorKind::Unsupported, "the system could not move it to the Trash").into());
        } else {
            trash::trash_held(&path, folder, name)?;
        }

        self.renamed();

        Ok(())
    }
}

/// Enters the folders `parents` below `root`, each from the one above with no link followed: the last of them (`None` for
/// the root itself) and its path. `Ok(None)` when one is gone or is not a folder now. `before` is a test's, run between
/// the look at a folder and its open.
fn enter(root: &Root, parents: &str, before: Option<&OpenHook>) -> io::Result<Option<(Option<Held>, PathBuf)>> {
    let mut held: Option<Held> = None;
    let mut path = root.path.clone();
    let mut walked = String::new();

    for part in parents.split('/').filter(|part| !part.is_empty()) {
        let above = held.as_ref().unwrap_or(&root.held);
        let node = match above.look(part) {
            Ok(node) if node.kind == Kind::Dir => node,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };

        if !walked.is_empty() {
            walked.push('/');
        }
        walked.push_str(part);
        if let Some(before) = before {
            before(&walked);
        }

        held = Some(above.subfolder(&node)?);
        path.push(part);
    }

    Ok(Some((held, path)))
}

/// What a removal states of `node`, as a plan states it. A link is `other`, which no removal holds.
fn facts_of(node: &Node) -> SyncFacts {
    let kind = match node.kind {
        Kind::Dir => EntryKind::Dir,
        Kind::File => EntryKind::File,
        Kind::Link | Kind::Other => EntryKind::Other,
    };

    SyncFacts {
        kind,
        size: if kind == EntryKind::File { node.size } else { 0 },
        modified: node.modified,
    }
}

/// Whether `node` was modified or changed (Unix `ctime`) after `since`, or has no time to tell.
fn is_newer(node: &Node, since: i64) -> bool {
    node.modified.is_none_or(|time| time > since) || node.changed.is_some_and(|time| time > since)
}

/// The device of the folder `held` (Unix); `None` on Windows, where it is not looked at.
fn device_of(held: &Held) -> io::Result<Option<u64>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Ok(Some(held.dir.metadata()?.dev()))
    }

    #[cfg(not(unix))]
    {
        let _ = held;

        Ok(None)
    }
}

/// What a folder holds, all of it.
#[derive(Default)]
struct Survey {
    files: u64,
    bytes: u64,
    /// A folder in it is the top of another mounted drive.
    mount: bool,
}

/// Walks the folder `held` (on `device`, `depth` levels down the compared folders) by handle, depth first, holding only the
/// folders above the one it reads: adds to `found` its files and their bytes as the scan counts them (every entry, a link
/// counted as nothing and never followed), and whether it holds a mount. `Ok(None)` when it is not as the scan saw it:
/// deeper than the scan goes, an entry left out of a listing, anything modified or changed after `since`, or the archive
/// compared (`archive`) in it. A folder that cannot be read fails the walk.
fn survey(
    held: &Held,
    (device, hooks): (Option<u64>, &Hooks),
    depth: usize,
    (since, archive): (i64, Option<&Archive>),
    found: &mut Survey,
) -> io::Result<Option<()>> {
    let entries = held.entries()?;
    if let Some(error) = entries.failed.into_iter().next() {
        return Err(error);
    }
    if entries.skipped > 0 {
        return Ok(None);
    }

    for entry in entries.nodes {
        if is_newer(&entry, since) || archive.is_some_and(|archive| archive.is(&entry)) {
            return Ok(None);
        }

        match entry.kind {
            Kind::File => {
                found.files += 1;
                found.bytes = found.bytes.saturating_add(entry.size);
            }
            Kind::Dir if depth + 1 >= MAX_DEPTH => return Ok(None),
            Kind::Dir => {
                let below = held.subfolder(&entry)?;
                found.mount |= is_mount(&below, &entry, (device, hooks))?;
                if survey(&below, (entry.device, hooks), depth + 1, (since, archive), found)?.is_none() {
                    return Ok(None);
                }
            }
            Kind::Link | Kind::Other => {}
        }
    }

    Ok(Some(()))
}

/// Whether the folder `node`, opened as `held`, is the top of another mounted drive: on another device than the folder that
/// holds it, or (Linux 5.8 and later) a mount root on the same one, as a bind mount is. Windows: never (see the module's
/// text).
fn is_mount(held: &Held, node: &Node, (outer, hooks): (Option<u64>, &Hooks)) -> io::Result<bool> {
    if hooks.mount_names.iter().any(|name| node.name == name.as_str()) {
        return Ok(true);
    }
    if cfg!(windows) {
        return Ok(false);
    }
    if node.device != outer {
        return Ok(true);
    }

    is_mount_root(held)
}

#[cfg(target_os = "linux")]
fn is_mount_root(held: &Held) -> io::Result<bool> {
    use rustix::fs::{AtFlags, StatxFlags, statx};

    super::delete::is_mount_root_stat(statx(&held.dir, "", AtFlags::EMPTY_PATH, StatxFlags::empty()))
}

#[cfg(not(target_os = "linux"))]
fn is_mount_root(_held: &Held) -> io::Result<bool> {
    Ok(false)
}

/// What stops a removal for good that is under way: an entry changed after the look.
#[cfg(unix)]
fn changed_meanwhile() -> io::Error {
    refusal(io::ErrorKind::Other, "it changed while the folder was being walked")
}

#[cfg(unix)]
impl Run<'_> {
    /// Removes `node` of `folder` for good, through handles: a file or a link alone (when it is still that entry), a folder
    /// with all that is in it, entered from its folder with no link followed (see [`Run::empty_held`]).
    fn remove_held(&mut self, folder: &Held, node: &Node, since: i64) -> Result<(), Stop> {
        use rustix::fs::{AtFlags, statat, unlinkat};

        if node.kind == Kind::Dir {
            let held = folder.subfolder(node)?;
            self.empty_held(&held, node.device, (0, since))?;

            return Ok(unlinkat(&folder.dir, &node.name, AtFlags::REMOVEDIR).map_err(io::Error::from)?);
        }

        let now = statat(&folder.dir, &node.name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
        if !node.is(&now) {
            return Err(changed_meanwhile().into());
        }

        Ok(unlinkat(&folder.dir, &node.name, AtFlags::empty()).map_err(io::Error::from)?)
    }

    /// Removes all that the folder `held` (on `device`, `depth` below the item) holds, through its handle. It stops, with
    /// what it removed gone, at a folder on another drive, at an entry modified or changed after `since`, and when the job
    /// is cancelled (looked at once for each folder).
    fn empty_held(&mut self, held: &Held, device: Option<u64>, (depth, since): (usize, i64)) -> Result<(), Stop> {
        use rustix::fs::{AtFlags, unlinkat};

        self.checkpoint()?;
        let entries = held.entries()?;
        if let Some(error) = entries.failed.into_iter().next() {
            return Err(error.into());
        }

        for entry in entries.nodes {
            if is_newer(&entry, since) {
                return Err(changed_meanwhile().into());
            }

            if entry.kind == Kind::Dir {
                if depth + 1 >= MAX_DEPTH {
                    return Err(refusal(io::ErrorKind::InvalidInput, "the folders go too deep").into());
                }

                let below = held.subfolder(&entry)?;
                if is_mount(&below, &entry, (device, &self.inner.hooks))? {
                    return Err(refusal(io::ErrorKind::InvalidInput, HOLDS_A_MOUNT).into());
                }
                self.empty_held(&below, entry.device, (depth + 1, since))?;
                unlinkat(&held.dir, &entry.name, AtFlags::REMOVEDIR).map_err(io::Error::from)?;
            } else {
                unlinkat(&held.dir, &entry.name, AtFlags::empty()).map_err(io::Error::from)?;
            }
        }

        Ok(())
    }
}

#[cfg(windows)]
impl Run<'_> {
    /// Removes `node` for good by its path: the system's removal never enters a mounted volume or a link (see `delete`).
    fn remove_held(&mut self, _folder: &Held, node: &Node, _since: i64) -> Result<(), Stop> {
        Ok(super::delete::remove(&node.path)?)
    }
}

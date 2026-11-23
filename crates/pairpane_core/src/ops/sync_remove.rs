//! The removals of a synchronization that mirrors: items only the side being written to has, removed so that it matches
//! the other side.
//!
//! Like a copy, a removal trusts nothing the scan saw. Right before an item is removed it is looked at again and must still
//! be what the scan saw: the same kind, and for a file the same size and time to the millisecond. For a folder its whole
//! subtree is walked again (all of it, hidden entries too, no link followed): it must hold the same number of files and
//! bytes, and nothing in it, the folder itself included, may have a time later than the start of the scan (which catches a
//! file edited in place, an entry added, removed or renamed, and a time in the future too). An item whose name the source
//! folder has by now is kept as well (an archive source is not looked at again: nothing of it is read for a removal), and
//! so is one that is the archive being compared, or holds it. What is kept is counted as skipped.
//!
//! The item goes to the Trash (the Recycle Bin on Windows) unless the job was asked to delete for good. A Trash that refuses
//! the item fails the item: it is never deleted for good instead.
//!
//! On Unix every step goes by handle, as the writes of a copy do (see `sync_write`): the side's folder is opened once for the
//! job, each folder on the way is opened from the one above with no link followed, and the item is looked at, walked and
//! removed through the handle of its folder (`unlinkat`), or renamed into the Trash from that handle (freedesktop), so a
//! folder swapped for a link after the look leads nowhere outside the compared folders. A folder deleted for good is
//! emptied through handles too, and refused whole, before anything of it goes, when it holds a mounted drive (a folder on
//! another device, or a mount root on Linux); a mount that appears while it is emptied stops it there. macOS takes a path
//! for the Trash: the path must lead to the held folder right before the move. Windows goes by path throughout (the
//! system's removal never enters a mounted volume or a link): a folder swapped in between is the race the other Windows
//! jobs document.
//!
//! The last look and the removal are two steps: an item swapped for another inside its folder in between still goes,
//! which is as narrow as a removal can make it.

use std::io;
use std::path::{Path, PathBuf};

use crate::model::{EntryKind, JobPhase, SyncFacts};

use super::SyncRemove;
use super::held::{Held, Node};
use super::hooks::OpenHook;
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

/// The folder of a side, held open for the whole job, and where it really is.
struct Root {
    path: PathBuf,
    real: PathBuf,
    held: Held,
}

impl Root {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            real: real(path)?,
            held: Held::root(path)?,
        })
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
        let archive = archive.map(real).transpose()?;
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

            let done = self.remove_item(&roots[index], removal, permanent, archive.as_deref());
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
        archive: Option<&Path>,
    ) -> Result<(), Stop> {
        let Removal {
            rel,
            removal: seen,
            source,
            ..
        } = *removal;

        if let Some(source) = source
            && lookup_below(source, rel)?.is_some()
        {
            return self.changed();
        }
        if archive.is_some_and(|archive| is_inside(archive, &root.real.join(rel))) {
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
        if !same_facts(Some(&facts_of(&node)), Some(&seen.seen)) {
            return self.changed();
        }

        if node.kind == Kind::Dir {
            let depth = rel.split('/').count();
            let Some(survey) = survey(folder, &node, depth)? else {
                return self.changed();
            };
            if (survey.files, survey.bytes) != (seen.files, seen.bytes)
                || survey.newest.is_none_or(|newest| newest > seen.scanned)
            {
                return self.changed();
            }
            if permanent && survey.mount {
                return Err(refusal(io::ErrorKind::InvalidInput, HOLDS_A_MOUNT).into());
            }
        }

        if permanent {
            remove_held(folder, &node)?;
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

/// What a folder holds, all of it.
struct Survey {
    files: u64,
    bytes: u64,
    /// The latest time of anything in it, the folder itself included; `None` when one has none.
    newest: Option<i64>,
    /// A folder in it is the top of another mounted drive.
    mount: bool,
}

/// Walks the folder `node` of `folder` by handle, which is `depth` levels down the compared folders: the files in it and
/// their bytes as the scan counts them (every entry, a link counted as nothing and never followed), the latest time in it,
/// and whether it holds a mount. `None` when it goes deeper than the scan goes (the scan counted less), or when an entry
/// is left out of a listing. A folder that cannot be read fails the walk.
fn survey(folder: &Held, node: &Node, depth: usize) -> io::Result<Option<Survey>> {
    let mut found = Survey {
        files: 0,
        bytes: 0,
        newest: node.modified,
        mount: false,
    };
    let mut pending = vec![(folder.subfolder(node)?, node.device, depth)];

    while let Some((held, device, depth)) = pending.pop() {
        let entries = held.entries()?;
        if let Some(error) = entries.failed.into_iter().next() {
            return Err(error);
        }
        if entries.skipped > 0 {
            return Ok(None);
        }

        for entry in entries.nodes {
            found.newest = found.newest.zip(entry.modified).map(|(one, other)| one.max(other));

            match entry.kind {
                Kind::File => {
                    found.files += 1;
                    found.bytes = found.bytes.saturating_add(entry.size);
                }
                Kind::Dir if depth + 1 >= MAX_DEPTH => return Ok(None),
                Kind::Dir => {
                    let below = held.subfolder(&entry)?;
                    found.mount |= is_mount(&below, &entry, device)?;
                    pending.push((below, entry.device, depth + 1));
                }
                Kind::Link | Kind::Other => {}
            }
        }
    }

    Ok(Some(found))
}

/// Whether the folder `node`, opened as `held`, is the top of another mounted drive: on another device than the folder that
/// holds it, or (Linux 5.8 and later) a mount root on the same one, as a bind mount is. Windows: never (see the module's
/// text).
fn is_mount(held: &Held, node: &Node, outer: Option<u64>) -> io::Result<bool> {
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

/// Removes `node` of `folder` for good, through handles: a file or a link alone (when it is still that entry), a folder
/// with all that is in it, entered from its folder with no link followed. A folder in it on another drive stops it.
#[cfg(unix)]
fn remove_held(folder: &Held, node: &Node) -> io::Result<()> {
    use rustix::fs::{AtFlags, statat, unlinkat};

    if node.kind == Kind::Dir {
        let held = folder.subfolder(node)?;
        empty_held(&held, node.device, 0)?;

        return unlinkat(&folder.dir, &node.name, AtFlags::REMOVEDIR).map_err(io::Error::from);
    }

    let now = statat(&folder.dir, &node.name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
    if !node.is(&now) {
        return Err(refusal(
            io::ErrorKind::Other,
            "it changed while the folder was being walked",
        ));
    }

    unlinkat(&folder.dir, &node.name, AtFlags::empty()).map_err(io::Error::from)
}

/// Removes all that the folder `held` (on `device`) holds, through its handle.
#[cfg(unix)]
fn empty_held(held: &Held, device: Option<u64>, depth: usize) -> io::Result<()> {
    use rustix::fs::{AtFlags, unlinkat};

    let entries = held.entries()?;
    if let Some(error) = entries.failed.into_iter().next() {
        return Err(error);
    }

    for entry in entries.nodes {
        if entry.kind == Kind::Dir {
            if depth + 1 >= MAX_DEPTH {
                return Err(refusal(io::ErrorKind::InvalidInput, "the folders go too deep"));
            }

            let below = held.subfolder(&entry)?;
            if is_mount(&below, &entry, device)? {
                return Err(refusal(io::ErrorKind::InvalidInput, HOLDS_A_MOUNT));
            }
            empty_held(&below, entry.device, depth + 1)?;
            unlinkat(&held.dir, &entry.name, AtFlags::REMOVEDIR).map_err(io::Error::from)?;
        } else {
            unlinkat(&held.dir, &entry.name, AtFlags::empty()).map_err(io::Error::from)?;
        }
    }

    Ok(())
}

/// Removes `node` for good by its path: the system's removal never enters a mounted volume or a link (see `delete`).
#[cfg(windows)]
fn remove_held(_folder: &Held, node: &Node) -> io::Result<()> {
    super::delete::remove(&node.path)
}

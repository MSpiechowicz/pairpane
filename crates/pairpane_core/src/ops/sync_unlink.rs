//! Deleting an item of a mirror for good (see `sync_remove`): on Unix through folder handles, a folder emptied entry by entry
//! with no link followed; on Windows by path, through the system's removal, which never enters a mounted volume or a link.

#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::io;

use super::held::{Held, Node};
#[cfg(unix)]
use super::plan::Kind;
#[cfg(unix)]
use super::report::refusal;
use super::sync_remove::Since;
#[cfg(unix)]
use super::sync_remove::{HOLDS_A_MOUNT, is_mount};
#[cfg(unix)]
use super::transfer::MAX_DEPTH;
use super::transfer::{Run, Stop};

/// What stops a removal for good that is under way: an entry changed after the look.
#[cfg(unix)]
fn changed_meanwhile() -> io::Error {
    refusal(io::ErrorKind::Other, "it changed while the folder was being walked")
}

#[cfg(unix)]
impl Run<'_> {
    /// Removes `node` of `folder` for good, through handles: a file or a link alone (when it is still that entry), a folder
    /// with all that is in it, entered from its folder with no link followed (see [`Run::empty_held`]).
    pub(super) fn remove_held(&mut self, folder: &Held, node: &Node, since: Since) -> Result<(), Stop> {
        use rustix::fs::{AtFlags, statat, unlinkat};

        if node.kind == Kind::Dir {
            let held = folder.subfolder(node)?;
            self.empty_held(&held, node.device, (0, since), &mut HashSet::new())?;

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
    /// is cancelled (looked at once for each folder). `unlinked` are the files this walk unlinked: another link to one of
    /// them had its change time moved by the walk itself, so for it only the modification time tells.
    fn empty_held(
        &mut self,
        held: &Held,
        device: Option<u64>,
        (depth, since): (usize, Since),
        unlinked: &mut HashSet<(u64, u64)>,
    ) -> Result<(), Stop> {
        use rustix::fs::{AtFlags, unlinkat};

        self.checkpoint()?;
        let entries = held.entries()?;
        if let Some(error) = entries.failed.into_iter().next() {
            return Err(error.into());
        }

        for entry in entries.nodes {
            let ours = entry.id.is_some_and(|id| unlinked.contains(&id));
            let newer = if ours {
                entry.modified.is_none_or(|time| time > since.time)
            } else {
                since.sees(&entry)
            };
            if newer {
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
                self.empty_held(&below, entry.device, (depth + 1, since), unlinked)?;
                unlinkat(&held.dir, &entry.name, AtFlags::REMOVEDIR).map_err(io::Error::from)?;
            } else {
                unlinkat(&held.dir, &entry.name, AtFlags::empty()).map_err(io::Error::from)?;
                unlinked.extend(entry.id);
            }
        }

        Ok(())
    }
}

#[cfg(windows)]
impl Run<'_> {
    /// Removes `node` for good by its path: the system's removal never enters a mounted volume or a link (see `delete`).
    pub(super) fn remove_held(&mut self, _folder: &Held, node: &Node, _since: Since) -> Result<(), Stop> {
        Ok(super::delete::remove(&node.path)?)
    }
}

//! The holding folders of a synchronization (see `sync_aside`): made by the job in a target folder, pinned by their identity,
//! and only ever reached from that target folder held open, so a folder put at their name meanwhile is never used or
//! removed.
//!
//! On Unix a holding folder is made with no access for anyone but its owner (`mkdirat` 0700); it must still be the job's
//! user's, with no access for the group or others, when it is opened right after. What is kept in it is named by the log,
//! each with the identity of the file it is: putting one back checks it, and removing the folder unlinks only those names
//! (each a file) and then the folder, which must be empty by then; anything else in it is left, and the folder with it. On
//! Windows the steps go by path, the folder and each file checked by identity first.

use std::io;
use std::path::{Path, PathBuf};

use super::held::{Held, Node};
use super::identity::FolderId;
use super::plan::Kind;

/// One holding folder: where it is, what it is, and what the job kept in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Holding {
    /// The target folder it is in, as the job named it.
    pub(super) root: PathBuf,
    pub(super) name: String,
    pub(super) id: Option<FolderId>,
    /// The old files in it: each name and the identity of the file.
    pub(super) kept: Vec<Kept>,
}

/// An old file kept in a holding folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Kept {
    pub(super) name: String,
    pub(super) id: Option<FolderId>,
}

/// The identity of `node`, as the log keeps it.
#[cfg(unix)]
pub(super) fn identity_of(node: &Node) -> Option<FolderId> {
    node.id.map(FolderId::of_ids)
}

#[cfg(windows)]
pub(super) fn identity_of(node: &Node) -> Option<FolderId> {
    FolderId::of_path(&node.path)
}

/// Makes the holding folder `name` in `top` (the target folder held open) and opens it; gives it with its identity.
pub(super) fn make(top: &Held, name: &str) -> io::Result<(Held, Option<FolderId>)> {
    make_folder(top, name)?;
    let node = top.look(name)?;
    if node.kind != Kind::Dir {
        return Err(refused());
    }

    let held = top.subfolder(&node)?;
    check_private(&held)?;

    Ok((held, identity_of(&node)))
}

/// Opens `holding` from `top`, the target folder it is in held open: it must be the folder it was made as.
pub(super) fn open(top: &Held, holding: &Holding) -> io::Result<Held> {
    let node = top.look(&holding.name)?;
    if node.kind != Kind::Dir || holding.id.is_none() || identity_of(&node) != holding.id {
        return Err(refused());
    }

    top.subfolder(&node)
}

/// The kept file `kept` of the holding folder `folder`, when it is still that file.
pub(super) fn kept_file(folder: &Held, kept: &Kept) -> io::Result<Node> {
    let node = folder.look(&kept.name)?;
    if node.kind != Kind::File || kept.id.is_none() || identity_of(&node) != kept.id {
        return Err(refused());
    }

    Ok(node)
}

/// Removes the kept file `kept` from `holding`, when both are still what they were.
pub(super) fn forget(holding: &Holding, kept: &Kept) {
    let removed = Held::root(&holding.root).and_then(|top| {
        let folder = open(&top, holding)?;
        let node = kept_file(&folder, kept)?;

        unlink(&folder, &node, false)
    });
    if let Err(error) = removed {
        tracing::debug!(?error, "an old file kept for an undo was left in its holding folder");
    }
}

/// Removes `holding` with the old files the job kept in it (see the module's text).
pub(super) fn drop_holding(holding: &Holding) {
    let dropped = Held::root(&holding.root).and_then(|top| {
        let folder = open(&top, holding)?;

        for kept in &holding.kept {
            if let Ok(node) = kept_file(&folder, kept) {
                unlink(&folder, &node, false)?;
            }
        }

        let node = top.look(&holding.name)?;
        unlink(&top, &node, true)
    });
    if let Err(error) = dropped {
        tracing::warn!(?error, "a holding folder of a synchronization was left behind");
    }
}

fn refused() -> io::Error {
    io::Error::other("the holding folder or a file in it is not what the synchronization made")
}

#[cfg(unix)]
fn make_folder(top: &Held, name: &str) -> io::Result<()> {
    use rustix::fs::{Mode, mkdirat};

    mkdirat(&top.dir, name, Mode::from_bits_truncate(0o700)).map_err(io::Error::from)
}

#[cfg(windows)]
fn make_folder(top: &Held, name: &str) -> io::Result<()> {
    std::fs::create_dir(top.path.join(name))
}

/// Fails unless the folder `held` is the job's user's, with no access for the group or others (Unix).
#[cfg(unix)]
fn check_private(held: &Held) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = held.dir.metadata()?;
    if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(refused());
    }

    Ok(())
}

#[cfg(windows)]
fn check_private(_held: &Held) -> io::Result<()> {
    Ok(())
}

/// Removes `node` of `folder` (a folder when `folder_too`, which must be empty).
#[cfg(unix)]
fn unlink(folder: &Held, node: &Node, folder_too: bool) -> io::Result<()> {
    use rustix::fs::{AtFlags, unlinkat};

    let flags = if folder_too {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };

    unlinkat(&folder.dir, &node.name, flags).map_err(io::Error::from)
}

#[cfg(windows)]
fn unlink(_folder: &Held, node: &Node, folder_too: bool) -> io::Result<()> {
    if folder_too {
        std::fs::remove_dir(&node.path)
    } else {
        std::fs::remove_file(&node.path)
    }
}

/// The path of `holding`.
pub(super) fn path_of(holding: &Holding) -> PathBuf {
    holding.root.join(&holding.name)
}

/// Whether `holding` is in the target folder `root`.
pub(super) fn is_in(holding: &Holding, root: &Path) -> bool {
    holding.root == root
}

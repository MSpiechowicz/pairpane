//! The object folders of a repository, which gix goes through to read an object: the repository's own and every alternate it
//! names (each with its own `info/alternates`, up to a depth). What is looked at, before anything is read:
//!
//! - **Where they are.** The lists are parsed by gix's own parser (`gix::odb::alternate::parse`: comments, quoted entries, and
//!   `\r` is kept as part of the name), and every relative entry of every list, at any depth, is joined to the objects folder
//!   of the repository itself, as gix joins it (`gix_odb::alternate::resolve`), and not to the folder of the list it was read
//!   from. An entry that is no local place, a list that is too long, an entry with a `..` straight after a link, a folder
//!   that is too broad (see [`super::layout::is_too_broad`]) and a store that belongs to another user is a repository not to read.
//!   A link on the way is followed, hop by hop, to the folder it leads to (`locality::alternate_target`), and that folder is what
//!   is looked at, once however many lines and chains name it (stores are told apart by the folder they lead to).
//! - **What is in them.** The folder itself, `info`, `pack` and each of the 256 folders of loose objects that is there must be a
//!   real folder, no link or reparse point; every entry of a folder of loose objects, of `pack`, of `pack/multi-pack-index.d`
//!   (an incremental multi-pack index) and of `info/commit-graphs` must be a regular file, as the listing of the folder says of
//!   it (`DirEntry::file_type`: no call is made for a file, and a link or a reparse point is not one), and the folders of `pack`,
//!   `pack/multi-pack-index.d` and `info/commit-graphs` are not on a file system of the kernel's own (`files::on_pseudo_path`, one `statfs` each). A link there would lead gix to a
//!   place that was not looked at (a loose object that is a link to `\\host\share\x` makes the system connect to the host). The
//!   entries of all these folders and of the git folder add up, for a whole repository and every store it names, to
//!   [`ENTRIES`](super::inspect::ENTRIES) at most (a repository with more is too big to read here), and each is counted with a
//!   look at the exit flag and the time limit (`gate`), so that no list of stores can make the look last.
//! - **What the helper may read.** Of an alternate only the `info`, `pack` and `xx` folders that were looked at, and never the
//!   folder of the store itself ([`alternates`]): a store whose folder is a broad place (the root, `/tmp`) gives nothing of it
//!   to the helper but those.

use std::path::{Path, PathBuf};

use super::files::{self, Seen, is_plain_entry, is_plain_folder_entry};
use super::gate::Gate;
use super::inspect::{Looking, Refusal, fine, listed};
use super::layout::is_too_broad;
use super::links::leads_nowhere;
use super::locality::alternate_target;

/// The most the list of alternate object folders may hold, and how many lines (over all the lists of a repository) and how deep a
/// chain of them is followed.
const ALTERNATES_BYTES: u64 = 64 * 1024;
const ALTERNATES_LINES: usize = 32;
const ALTERNATES_DEPTH: usize = 5;

/// Looks at every object folder of the repository whose common folder is `common_dir`, and gives the folders of its alternates'
/// stores that the helper may read (see the module). The entries of all of them count in the one budget of `looking`.
pub(super) fn alternates(common_dir: &Path, looking: &mut Looking<'_>) -> Result<Vec<PathBuf>, Refusal> {
    let stores = object_folders(common_dir, looking.gate())?;
    let mut granted = Vec::new();

    for (position, store) in stores.iter().enumerate() {
        let folders = object_folder(store, looking)?;

        // The repository's own objects folder (the first) is below the common folder, which is read as a whole.
        if position > 0 {
            granted.extend(folders);
        }
    }

    Ok(granted)
}

/// The object folders: the repository's own, first, and the alternates it names, each as the folder it leads to and each once (a
/// store that several lines, or a chain, name is looked at once). See the module for how they are read.
fn object_folders(common_dir: &Path, gate: Gate<'_>) -> Result<Vec<PathBuf>, Refusal> {
    let own = common_dir.join("objects");
    let mut folders = vec![own.clone()];
    let mut pending = vec![(own.clone(), 0)];
    let mut lines = 0;

    while let Some((folder, depth)) = pending.pop() {
        gate.check()?;
        let bytes = match files::read_below(&folder, "info/alternates", ALTERNATES_BYTES) {
            Seen::Missing => continue,
            Seen::Plain(bytes) => bytes,
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        };
        let entries = gix::odb::alternate::parse(&bytes).map_err(|_| Refusal::Odd)?;

        for entry in entries {
            gate.check()?;
            if depth + 1 > ALTERNATES_DEPTH || lines + 1 >= ALTERNATES_LINES {
                return Err(Refusal::Odd);
            }
            lines += 1;

            // Not a local place: the repository is not read, and nothing at the other end is touched.
            let target = alternate_target(&own, &entry).ok_or(Refusal::Odd)?;
            if folders.contains(&target) {
                continue;
            }
            if is_too_broad(&target) {
                return Err(Refusal::Broad);
            }
            is_the_users(&target)?;

            folders.push(target.clone());
            pending.push((target, depth + 1));
        }
    }

    Ok(folders)
}

/// A store that is another user's is not read, as a repository that is (see `locate`): what is in it may be anything that user,
/// or anyone who shares a folder with them, chose to leave there. One that is not there has nothing to read.
fn is_the_users(store: &Path) -> Result<(), Refusal> {
    match files::owned_by_user(store) {
        Ok(true) => Ok(()),
        Ok(false) => Err(Refusal::Owner),
        Err(error) if leads_nowhere(&error) => Ok(()),
        Err(_) => Err(Refusal::Odd),
    }
}

/// The structure of one object folder (see the module), and the folders of it that are there: `info`, `pack` and the `xx`
/// folders. The entries listed count in `looking`.
fn object_folder(objects: &Path, looking: &mut Looking<'_>) -> Result<Vec<PathBuf>, Refusal> {
    let fanned_out = (0..=u8::MAX).map(|byte| objects.join(format!("{byte:02x}")));
    let others = [objects.join("info"), objects.join("pack")];
    let mut present = Vec::new();

    if let Seen::Odd | Seen::Large = files::folder(objects) {
        return Err(Refusal::Odd);
    }
    for folder in others.into_iter().chain(fanned_out) {
        looking.gate().check()?;

        match files::folder(&folder) {
            Seen::Plain(()) => present.push(folder),
            Seen::Missing => {}
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        }
    }

    for folder in &present {
        match folder.file_name().and_then(|name| name.to_str()) {
            Some("pack") => pack(folder, looking)?,
            Some("info") => info(folder, looking)?,
            _ => loose_objects(folder, looking)?,
        }
    }

    Ok(present)
}

/// The loose objects of the folder `xx`: plain files, every one of them.
fn loose_objects(folder: &Path, looking: &mut Looking<'_>) -> Result<(), Refusal> {
    listed(folder, looking, plain)
}

/// The packs, their indexes and the rest of the files of `pack`, which gix maps or opens when an object is in one: plain files, and
/// an incremental multi-pack index's own folder with plain files in it.
fn pack(folder: &Path, looking: &mut Looking<'_>) -> Result<(), Refusal> {
    not_made_up(folder)?;

    let midx = folder.join("multi-pack-index.d");
    let mut has_midx = false;
    listed(folder, looking, |entry| {
        if entry.file_name() == "multi-pack-index.d" && is_plain_folder_entry(entry) {
            has_midx = true;

            return Ok(());
        }

        plain(entry)
    })?;

    if has_midx { plain_files(&midx, looking) } else { Ok(()) }
}

/// `info` of an object folder: the commit graph (a file, or a folder `commit-graphs` of files, which is a chain of them) that gix
/// reads next to the objects. The rest of `info` is not read by anything of a status.
fn info(folder: &Path, looking: &mut Looking<'_>) -> Result<(), Refusal> {
    fine(files::seen(&folder.join("commit-graph"), u64::MAX))?;

    match files::folder(&folder.join("commit-graphs")) {
        Seen::Plain(()) => plain_files(&folder.join("commit-graphs"), looking),
        Seen::Missing => Ok(()),
        Seen::Odd | Seen::Large => Err(Refusal::Odd),
    }
}

/// Every entry of `folder` is a plain file (a regular file that is no link), as the listing says of it. The files are not opened:
/// gix maps a pack and reads a commit graph through a map, so what the file system would make up for a file of its own
/// (`/proc`, `/sys`) is the one thing that a look at each file would add, and that is asked once for the folder ([`not_made_up`]).
fn plain_files(folder: &Path, looking: &mut Looking<'_>) -> Result<(), Refusal> {
    not_made_up(folder)?;

    listed(folder, looking, plain)
}

fn plain(entry: &std::fs::DirEntry) -> Result<(), Refusal> {
    if is_plain_entry(entry) {
        Ok(())
    } else {
        Err(Refusal::Odd)
    }
}

/// The folder is not on a file system of the kernel's own, which makes its files up (see `files::on_pseudo_path`).
fn not_made_up(folder: &Path) -> Result<(), Refusal> {
    if files::on_pseudo_path(folder) {
        Err(Refusal::Odd)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "objects_budget_tests.rs"]
mod budget_tests;

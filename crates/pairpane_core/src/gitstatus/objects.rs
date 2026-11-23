//! The object folders of a repository, which gix goes through to read an object: the repository's own and every alternate it
//! names (each with its own `info/alternates`, up to a depth). What is looked at, before anything is read:
//!
//! - **Where they are.** The lists are parsed by gix's own parser (`gix::odb::alternate::parse`: comments, quoted entries, and
//!   `\r` is kept as part of the name), and every relative entry of every list, at any depth, is joined to the objects folder
//!   of the repository itself, as gix joins it (`gix_odb::alternate::resolve`), and not to the folder of the list it was read
//!   from. An entry that is no local place, a list that is too long, an entry with a `..` straight after a link, and a folder
//!   that is too broad (see [`super::layout::is_too_broad`]) is a repository not to read. A link on the way is followed, hop by
//!   hop, to the folder it leads to (`locality::alternate_target`), and that folder is what is looked at.
//! - **What is in them.** The folder itself, `info`, `pack` and each of the 256 folders of loose objects that is there must be a
//!   real folder, no link or reparse point; every entry of a folder of loose objects, of `pack`, of `pack/multi-pack-index.d`
//!   (an incremental multi-pack index) and of `info/commit-graphs` must be a regular file; and the files of `pack` and of
//!   `info/commit-graphs` are looked at as any file of the repository is (`files::seen`). A link there would lead gix to a place
//!   that was not looked at (a loose object that is a link to `\\host\share\x` makes the system connect to the host), and the
//!   total of loose objects listed is capped at [`LOOSE_OBJECTS`] (a repository with more is too big to read here).
//! - **What the helper may read.** Of an alternate only the `info`, `pack` and `xx` folders that were looked at, and never the
//!   folder of the store itself ([`alternates`]): a store whose folder is a broad place (the root, `/tmp`) gives nothing of it
//!   to the helper but those.

use std::path::{Path, PathBuf};

use super::files::{self, Seen};
use super::inspect::{Refusal, fine, listed};
use super::layout::is_too_broad;
use super::locality::alternate_target;

/// The most the list of alternate object folders may hold, and how many lines and how deep a chain of them is followed.
const ALTERNATES_BYTES: u64 = 64 * 1024;
const ALTERNATES_LINES: usize = 32;
const ALTERNATES_DEPTH: usize = 5;

/// The most loose objects that are listed, over all the object folders of a repository: more is a repository that is too big to
/// read here.
pub(super) const LOOSE_OBJECTS: usize = 65_536;

/// Looks at every object folder of the repository whose common folder is `common_dir`, and gives the folders of its alternates'
/// stores that the helper may read (see the module).
pub(super) fn alternates(common_dir: &Path) -> Result<Vec<PathBuf>, Refusal> {
    let stores = object_folders(common_dir)?;
    let mut loose = 0;
    let mut granted = Vec::new();

    for (position, store) in stores.iter().enumerate() {
        let folders = object_folder(store, &mut loose)?;

        // The repository's own objects folder (the first) is below the common folder, which is read as a whole.
        if position > 0 {
            granted.extend(folders);
        }
    }

    Ok(granted)
}

/// The object folders: the repository's own, first, and the alternates it names, each as the folder it leads to. See the module
/// for how they are read.
fn object_folders(common_dir: &Path) -> Result<Vec<PathBuf>, Refusal> {
    let own = common_dir.join("objects");
    let mut folders = vec![own.clone()];
    let mut pending = vec![(own.clone(), 0)];

    while let Some((folder, depth)) = pending.pop() {
        let bytes = match files::read_below(&folder, "info/alternates", ALTERNATES_BYTES) {
            Seen::Missing => continue,
            Seen::Plain(bytes) => bytes,
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        };
        let entries = gix::odb::alternate::parse(&bytes).map_err(|_| Refusal::Odd)?;

        for entry in entries {
            if depth + 1 > ALTERNATES_DEPTH || folders.len() >= ALTERNATES_LINES {
                return Err(Refusal::Odd);
            }

            // Not a local place: the repository is not read, and nothing at the other end is touched.
            let target = alternate_target(&own, &entry).ok_or(Refusal::Odd)?;
            if is_too_broad(&target) {
                return Err(Refusal::Broad);
            }

            folders.push(target.clone());
            pending.push((target, depth + 1));
        }
    }

    Ok(folders)
}

/// The structure of one object folder (see the module), and the folders of it that are there: `info`, `pack` and the `xx`
/// folders. `loose` is the count of loose objects listed so far, to which this adds.
fn object_folder(objects: &Path, loose: &mut usize) -> Result<Vec<PathBuf>, Refusal> {
    let fanned_out = (0..=u8::MAX).map(|byte| objects.join(format!("{byte:02x}")));
    let others = [objects.join("info"), objects.join("pack")];
    let mut present = Vec::new();

    if let Seen::Odd | Seen::Large = files::folder(objects) {
        return Err(Refusal::Odd);
    }
    for folder in others.into_iter().chain(fanned_out) {
        match files::folder(&folder) {
            Seen::Plain(()) => present.push(folder),
            Seen::Missing => {}
            Seen::Odd | Seen::Large => return Err(Refusal::Odd),
        }
    }

    for folder in &present {
        match folder.file_name().and_then(|name| name.to_str()) {
            Some("pack") => pack(folder)?,
            Some("info") => info(folder)?,
            _ => loose_objects(folder, loose)?,
        }
    }

    Ok(present)
}

/// The loose objects of the folder `xx`: plain files, every one of them.
fn loose_objects(folder: &Path, loose: &mut usize) -> Result<(), Refusal> {
    listed(folder, |entry| {
        *loose += 1;
        if *loose > LOOSE_OBJECTS {
            return Err(Refusal::TooBig);
        }

        if files::is_plain_entry(entry) {
            Ok(())
        } else {
            Err(Refusal::Odd)
        }
    })
}

/// The packs, their indexes and the rest of the files of `pack`, which gix opens when an object is in one: plain files, and an
/// incremental multi-pack index's own folder with plain files in it.
fn pack(folder: &Path) -> Result<(), Refusal> {
    listed(folder, |entry| {
        if entry.file_name() == "multi-pack-index.d" && files::is_plain_folder_entry(entry) {
            return plain_files(&entry.path());
        }

        fine(files::seen(&entry.path(), u64::MAX))
    })
}

/// `info` of an object folder: the commit graph (a file, or a folder `commit-graphs` of files, which is a chain of them) that gix
/// reads next to the objects. The rest of `info` is not read by anything of a status.
fn info(folder: &Path) -> Result<(), Refusal> {
    fine(files::seen(&folder.join("commit-graph"), u64::MAX))?;

    match files::folder(&folder.join("commit-graphs")) {
        Seen::Plain(()) => plain_files(&folder.join("commit-graphs")),
        Seen::Missing => Ok(()),
        Seen::Odd | Seen::Large => Err(Refusal::Odd),
    }
}

/// Every entry of `folder` is a plain file (a regular file that is no link).
fn plain_files(folder: &Path) -> Result<(), Refusal> {
    listed(folder, |entry| fine(files::seen(&entry.path(), u64::MAX)))
}

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;

//! What an extraction writes: the folders (made one by one, never through a link), each file (to a temporary file next
//! to its target, given its mode and time, then put in place like a copy does), and the guards around that: the data is
//! cut off at the size the entry declares, and the disk keeps a reserve of free space.
//!
//! Only the permission bits `0o755` of an entry are kept (no write for the group and others; setuid, setgid and sticky
//! never are) and the owner always keeps read and write (a folder also entering), so a `0o000` entry is still readable.
//! A folder gets its mode and time last, once what is in it is written.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::archive::{self, EntryType, IndexEntry, MAX_INDEX_SCAN, MIN_FREE_RESERVE};
use crate::listing::platform;
use crate::model::FileFacts;

use super::conflict::Place;
use super::meta;
use super::name_fits;
use super::open;
use super::origin::Origin;
use super::partial::Partial;
use super::report::refusal;
use super::transfer::{Run, Stop};

/// Bytes written between two looks at the free space of the disk.
const CHECK_EVERY: u64 = 64 * 1024 * 1024;

/// The permission bits of a file whose archive states none.
const DEFAULT_FILE_MODE: u32 = 0o644;

/// A folder that this job made and that gets its mode and time at the end.
struct Folder {
    path: PathBuf,
    rel: String,
    mode: Option<u32>,
    modified: Option<i64>,
}

/// Where an extraction writes and what it has done there so far.
pub(super) struct Landing {
    target: PathBuf,
    /// The folders below the target that are known to be there, and whether this job made each.
    folders: HashMap<PathBuf, bool>,
    finish: Vec<Folder>,
    /// Bytes written since the free space was looked at.
    unchecked: u64,
    /// The marker of the archive's origin, put on each file written while it is still the temporary file.
    origin: Origin,
}

impl Landing {
    pub(super) fn new(target: PathBuf, origin: Origin) -> Self {
        Self {
            target,
            folders: HashMap::new(),
            finish: Vec::new(),
            unchecked: CHECK_EVERY,
            origin,
        }
    }
}

/// Fails unless `part` is a name that can be a file or folder name here.
fn plain(part: &str) -> io::Result<()> {
    if platform::is_plain_name(part) && name_fits(part) {
        return Ok(());
    }

    Err(refusal(
        io::ErrorKind::InvalidInput,
        "an entry has a name that cannot be used here",
    ))
}

impl Run<'_> {
    /// Writes the entry `entry`, whose path below the target is `rel`, reading its data from `data`: a folder is made, a
    /// file is written and put in place.
    pub(super) fn write_item(
        &mut self,
        landing: &mut Landing,
        entry: &IndexEntry,
        rel: &str,
        data: &mut dyn Read,
    ) -> Result<(), Stop> {
        if entry.kind == EntryType::Dir {
            return self.write_folder(landing, entry, rel);
        }

        self.begin_file(rel, entry.size);
        let done = self.write_file(landing, entry, rel, data);
        self.end_file();

        done
    }

    /// Makes the folders of `relative` below the target, the ones that are there already being used as they are.
    /// Returns the last. Nothing is made through a link: a link or a file in the way is an error.
    fn folder_for(&mut self, landing: &mut Landing, relative: &str) -> Result<PathBuf, Stop> {
        let mut path = landing.target.clone();

        for part in relative.split('/').filter(|part| !part.is_empty()) {
            plain(part)?;
            path.push(part);

            if !landing.folders.contains_key(&path) {
                let made = self.make_folder(&path, false)?;
                landing.folders.insert(path.clone(), made);
            }
        }

        Ok(path)
    }

    fn write_folder(&mut self, landing: &mut Landing, entry: &IndexEntry, rel: &str) -> Result<(), Stop> {
        let path = self.folder_for(landing, rel)?;

        if landing.folders.get(&path) == Some(&true) {
            landing.finish.push(Folder {
                path,
                rel: rel.to_owned(),
                mode: entry.mode,
                modified: entry.modified,
            });
        }

        Ok(())
    }

    fn write_file(
        &mut self,
        landing: &mut Landing,
        entry: &IndexEntry,
        rel: &str,
        data: &mut dyn Read,
    ) -> Result<(), Stop> {
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        plain(name)?;

        let folder = self.folder_for(landing, parent)?;
        let target = folder.join(name);
        let facts = FileFacts {
            size: entry.size,
            modified: entry.modified,
            dir: false,
        };

        let mut place = self.place_entry(rel, &facts, &target)?;
        if place == Place::Skip {
            self.skipped += 1;

            return Ok(());
        }

        let (temp, mut copy) = self.make_temp(&folder, open::create_new)?;
        let mut guard = Partial::new(temp.clone());

        self.write_data(landing, data, &mut copy, &folder, entry)?;
        self.checkpoint()?;
        // The marker goes on before the mode and time (a read-only file takes no stream, and writing one may change the
        // time) and before the rename: the file is never at its final name without it.
        landing.origin.apply(&copy, &temp);
        meta::apply_entry(&copy, Some(entry.mode.unwrap_or(DEFAULT_FILE_MODE)), entry.modified)?;
        drop(copy);

        if self.commit_with(&temp, &mut place, |run, again| run.place_entry(rel, &facts, again))? {
            guard.committed = true;
        } else {
            self.skipped += 1;
        }

        Ok(())
    }

    /// Copies the data of `entry` into `copy`, which is in `folder`, and fails when it is more than the entry declares
    /// (the one byte past the end is read to find out). The free space is looked at before the first byte and then every
    /// [`CHECK_EVERY`] bytes.
    fn write_data(
        &mut self,
        landing: &mut Landing,
        data: &mut dyn Read,
        copy: &mut File,
        folder: &Path,
        entry: &IndexEntry,
    ) -> Result<(), Stop> {
        // A gzip file declares only the length modulo 4 GiB: what it inflates to is bounded by the archive limit.
        let cap = if matches!(entry.place, archive::Place::Gz) {
            MAX_INDEX_SCAN
        } else {
            entry.size
        };
        let limit = cap.saturating_add(1);
        let mut crc = crc32fast::Hasher::new();
        let mut written = 0_u64;

        while written < limit {
            let slice = CHECK_EVERY.min(limit - written);

            self.keep_reserve(landing, folder, slice)?;
            let count = self.pump(data, copy, slice, &mut crc)?;
            written += count;

            if count < slice {
                break;
            }
        }

        if written > cap {
            return Err(refusal(io::ErrorKind::InvalidData, "an entry holds more data than it declares").into());
        }

        Ok(())
    }

    /// Before `next` more bytes are written into `folder`: fails with "disk full" when the disk would be left with less
    /// than the reserve. Only looks every [`CHECK_EVERY`] bytes (and where the system cannot tell the free space, never).
    fn keep_reserve(&mut self, landing: &mut Landing, folder: &Path, next: u64) -> io::Result<()> {
        if landing.unchecked >= CHECK_EVERY {
            landing.unchecked = 0;
            let reserve = self.inner.hooks.free_reserve.unwrap_or(MIN_FREE_RESERVE);

            if platform::free_space(folder).is_some_and(|free| free < reserve.saturating_add(next)) {
                return Err(refusal(
                    io::ErrorKind::StorageFull,
                    "there is not enough free space left on the disk",
                ));
            }
        }

        landing.unchecked = landing.unchecked.saturating_add(next);

        Ok(())
    }

    /// Gives the folders this job made their modes and times, the last made first. One that fails is reported.
    pub(super) fn finish_folders(&mut self, landing: Landing) {
        for folder in landing.finish.iter().rev() {
            // Windows keeps no permission bits on a folder.
            let mode = if cfg!(windows) { None } else { folder.mode };

            if let Err(error) = meta::apply_entry_folder(&folder.path, mode, folder.modified) {
                self.fail(&folder.rel, &error);
            }
        }
    }
}

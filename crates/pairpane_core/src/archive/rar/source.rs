//! The [`EntrySource`] of a RAR archive: `unrar` unpacks the wanted files into a private staging folder, and each one is
//! then handed to the visitor as a reader, in the order of the ordinals, so the caller writes it with the rules it has
//! for every format (names, conflicts, caps, permissions) and nothing `unrar` made is ever moved into place as it is.
//!
//! Only regular files whose paths the index holds are read from the staging folder; whatever else `unrar` made there (a
//! link, a file nobody asked for, a file a hard link copied) is never read, and the folder is removed when the call ends.
//! A wanted file that is not there (damaged, `unrar` named it differently) gives a reader that fails with
//! [`unreadable`], so its entry fails and the others go on. A folder entry is visited with an empty reader.
//!
//! Disk: the staging folder holds the unpacked files until they are read, so unpacking needs their size in free space on
//! the volume of the staging folder, beside the space of the destination. By default that is the system's temporary
//! folder; [`RarSource::staging_in`] puts it elsewhere (the destination volume, to be sure of one volume). Before `unrar`
//! starts the declared sizes are checked against the free space, minus [`MIN_FREE_RESERVE`]: every entry `unrar` may write
//! for a wanted name counts (names are matched without regard to case, and a name can come twice or as a link), and an
//! unpacking of everything counts every entry that is not a folder. While it runs, `unrar` is stopped when the volume is
//! left with less than the reserve (see [`Unrar::extract`]). Each file is deleted from the staging folder as soon as it is
//! read.
//!
//! The viewer's single entry goes through the same path (a one-entry `wanted`); `unrar p` is not used.

use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::archive::MIN_FREE_RESERVE;
use crate::archive::entries::{Declared, EntrySource, EntryType, Flow, Visit};
use crate::archive::error::{check, unreadable};
use crate::archive::index::{ArchiveIndex, IndexEntry};
use crate::listing::platform;

use super::run::Unrar;
use super::stage::Staging;
use super::{full, protected};

/// The most names that are handed to `unrar` on its command line; more (or longer) and everything is unpacked and the
/// wanted files are picked from it, which also avoids the system's limit on a command line.
const MAX_NAMES: usize = 512;
const MAX_NAME_BYTES: usize = 16 * 1024;

/// Extracts from a RAR archive by running `unrar`.
pub struct RarSource<'a> {
    index: &'a ArchiveIndex,
    archive: PathBuf,
    unrar: Unrar,
    cancel: &'a AtomicBool,
    staging_parent: Option<PathBuf>,
}

impl<'a> RarSource<'a> {
    /// A source for the archive `archive` of `index`, unpacked by `unrar`.
    pub fn new(index: &'a ArchiveIndex, archive: &Path, unrar: Unrar, cancel: &'a AtomicBool) -> Self {
        Self {
            index,
            archive: archive.to_owned(),
            unrar,
            cancel,
            staging_parent: None,
        }
    }

    /// Makes the staging folder inside `parent` (an absolute folder that exists) instead of the temporary folder.
    #[must_use]
    pub fn staging_in(mut self, parent: PathBuf) -> Self {
        self.staging_parent = Some(parent);

        self
    }

    /// The names `unrar` is asked for, or `None` to unpack everything and pick: names with a wildcard cannot be given as
    /// they are (`unrar` reads them as masks), and a long list does not fit a command line.
    fn names<'e>(files: &[&'e IndexEntry]) -> Option<Vec<&'e str>> {
        let bytes: usize = files.iter().map(|entry| entry.path.len() + 1).sum();
        let wildcard = files.iter().any(|entry| entry.path.contains(['*', '?']));

        if files.len() > MAX_NAMES || bytes > MAX_NAME_BYTES || wildcard {
            return None;
        }

        Some(files.iter().map(|entry| entry.path.as_str()).collect())
    }

    /// Fails when the staging volume cannot hold `needed` bytes and the reserve.
    fn check_space(parent: &Path, needed: u64) -> io::Result<()> {
        match platform::free_space(parent) {
            Some(free) if needed > free.saturating_sub(MIN_FREE_RESERVE) => Err(full()),
            _ => Ok(()),
        }
    }

    /// The bytes `unrar` may write when asked for `names` (everything when `None`): all the entries of the archive that
    /// are not folders and, for names, whose name is one of them without regard to case.
    fn staged_size(&self, names: Option<&[&str]>) -> u64 {
        let wanted: Option<HashSet<String>> = names.map(|names| names.iter().map(|name| name.to_lowercase()).collect());

        self.index
            .entries()
            .iter()
            .filter(|entry| entry.kind != EntryType::Dir)
            .filter(|entry| {
                wanted
                    .as_ref()
                    .is_none_or(|wanted| wanted.contains(&entry.path.to_lowercase()))
            })
            .map(|entry| entry.size)
            .fold(0, u64::saturating_add)
    }
}

impl EntrySource for RarSource<'_> {
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()> {
        check(self.cancel)?;

        let entries = wanted
            .iter()
            .map(|ordinal| self.index.entries().get(*ordinal as usize).ok_or_else(unreadable))
            .collect::<io::Result<Vec<_>>>()?;

        if entries.iter().any(|entry| !entry.writable()) {
            return Err(unreadable());
        }

        let files: Vec<&IndexEntry> = entries
            .iter()
            .copied()
            .filter(|entry| entry.kind == EntryType::File)
            .collect();

        // Passwords are not supported: an encrypted file cannot be unpacked.
        if files.iter().any(|entry| entry.encrypted) {
            return Err(protected());
        }

        let staging = if files.is_empty() {
            None
        } else {
            Some(self.unpack(&files)?)
        };

        for entry in entries {
            check(self.cancel)?;

            let flow = match (&staging, entry.kind) {
                (Some(staging), EntryType::File) => visit_file(staging, entry, visit)?,
                _ => visit(entry, &mut io::empty())?,
            };

            if flow == Flow::Stop {
                break;
            }
        }

        Ok(())
    }
}

impl RarSource<'_> {
    /// Unpacks `files` (or all) into a new staging folder.
    fn unpack(&self, files: &[&IndexEntry]) -> io::Result<Staging> {
        let names = Self::names(files);
        let needed = self.staged_size(names.as_deref());

        let parent = self.staging_parent.clone().unwrap_or_else(std::env::temp_dir);
        Self::check_space(&parent, needed)?;

        let staging = Staging::create(&parent)?;
        self.unrar
            .extract(&self.archive, names.as_deref(), staging.path(), self.cancel)?;

        Ok(staging)
    }
}

/// Hands the staged file of `entry` to `visit`, then deletes it.
fn visit_file(staging: &Staging, entry: &IndexEntry, visit: &mut Visit<'_>) -> io::Result<Flow> {
    let Some(file) = staging.open(&entry.path) else {
        return visit(entry, &mut Absent);
    };

    let flow = visit(entry, &mut Declared::new(file, entry.size))?;
    staging.discard(&entry.path);

    Ok(flow)
}

/// The reader of a file that `unrar` did not unpack.
struct Absent;

impl Read for Absent {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(unreadable())
    }
}

//! Moving an item into the Trash: the record is made first, then the item is renamed into `files`.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Mode, OFlags, openat, unlinkat};
use rustix::io::Errno;

use super::Placed;
use super::folders::{self, Chosen, Context, Kind};
use super::handle::{self, Facts, birth_of, open_folder_exact, rename_noreplace, unusable};
use super::info;
use crate::ops::report::refusal;

/// The most names tried for one item (`name`, `name.2`, `name.3`, ...) before the Trash is taken for full of them.
const MOST_NAMES: u32 = 1000;

/// The longest name a file system takes, in bytes.
const NAME_MAX: usize = 255;

/// What is added to the name of an item in `files` to make the name of its record in `info`.
const RECORD_SUFFIX: &str = ".trashinfo";

/// The folder the items come from, open, and the Trash they go to. The Trash is chosen (and made if it is missing) for
/// the first item that is there to be moved, and kept for the others.
pub(super) struct Session {
    canonical: PathBuf,
    folder: OwnedFd,
    here: Facts,
    chosen: Option<Chosen>,
}

impl Session {
    /// Opens `folder`. The folder is resolved first and opened as it was resolved, without following a link at the
    /// end. Nothing is made yet.
    pub(super) fn open(folder: &Path) -> io::Result<Self> {
        let canonical = std::fs::canonicalize(folder)?;
        let handle = open_folder_exact(&canonical)?;
        let here = Facts::of_handle(&handle)?;

        Ok(Self {
            canonical,
            folder: handle,
            here,
            chosen: None,
        })
    }

    /// Takes the folder `held`, open already, which `folder` names: the item is renamed from that very handle. The path is
    /// still resolved for the record and to choose the Trash, and it must lead to the same folder as the handle now.
    pub(super) fn of_held(folder: &Path, held: OwnedFd) -> io::Result<Self> {
        let canonical = std::fs::canonicalize(folder)?;
        let here = Facts::of_handle(&held)?;
        let named = Facts::of_handle(&open_folder_exact(&canonical)?)?;

        if !named.is_same_place(&here) {
            return Err(refusal(
                io::ErrorKind::Other,
                "the folder changed while it was being walked",
            ));
        }

        Ok(Self {
            canonical,
            folder: held,
            here,
            chosen: None,
        })
    }

    /// Moves the item `name` of the folder to the Trash. A link goes as the link, never its target.
    ///
    /// An item that is not there is reported before anything is chosen or made: its absence leaves no trace in any
    /// Trash.
    pub(super) fn trash(&mut self, context: &Context, name: &str) -> io::Result<Placed> {
        let name = OsStr::new(name);

        if !handle::exists_in(&self.folder, name)? {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }

        // The date can be refused (a clock that is out of range); that is known before a Trash folder is made.
        let date = info::deletion_date()?;

        let chosen = match self.chosen.take() {
            Some(chosen) => chosen,
            None => folders::choose(context, &self.canonical, &self.folder, self.here.device)?,
        };
        let placed = self.place(&chosen, name, &date);
        self.chosen = Some(chosen);

        placed
    }

    /// Moves the item `name` into the Trash `chosen`, with a record that says it was deleted at `date`.
    fn place(&self, chosen: &Chosen, name: &OsStr, date: &str) -> io::Result<Placed> {
        let recorded = self.recorded(chosen, name)?;
        let record = info::render(&recorded, date);

        for number in 1..=MOST_NAMES {
            let stem = stem(name, number);

            if handle::exists_in(&chosen.folders.files, &stem)? {
                continue;
            }

            let Some(record_name) = make_record(chosen, &stem, &record)? else {
                continue;
            };

            match rename_noreplace(&self.folder, name, &chosen.folders.files, &stem) {
                Ok(()) => return Ok(self.placed(chosen, name, recorded, stem)),
                Err(error) => {
                    remove_record(chosen, &record_name);

                    // Someone took the name in between: the next one is tried.
                    if error.kind() != io::ErrorKind::AlreadyExists {
                        return Err(error);
                    }
                }
            }
        }

        // Not `AlreadyExists`: that is told to the user as a clash of names in their folder, and this is not one.
        Err(refusal(
            io::ErrorKind::Other,
            "the Trash holds too many items with this name",
        ))
    }

    /// The path the record gives for the item: absolute in the home Trash, else relative to the top folder of the
    /// drive.
    fn recorded(&self, chosen: &Chosen, name: &OsStr) -> io::Result<PathBuf> {
        let item = self.canonical.join(name);

        match chosen.kind {
            Kind::Home => Ok(item),
            Kind::Shared | Kind::Own => item
                .strip_prefix(&chosen.base)
                .map(Path::to_path_buf)
                .map_err(|_| unusable("the item is not below the top folder of its drive")),
        }
    }

    fn placed(&self, chosen: &Chosen, name: &OsStr, recorded: PathBuf, stem: OsString) -> Placed {
        Placed {
            name: name.to_string_lossy().into_owned(),
            kind: chosen.kind,
            base: chosen.base.clone(),
            recorded,
            stem,
            folder_device: self.here.device,
            folder_inode: self.here.inode,
            folder_owner: self.here.owner,
            folder_birth: birth_of(&self.folder),
        }
    }
}

/// Makes `info/<stem>.trashinfo` of the Trash `chosen` with `text`, and returns its name; `None` when one is there
/// already. A record that cannot be written is removed.
fn make_record(chosen: &Chosen, stem: &OsStr, text: &str) -> io::Result<Option<OsString>> {
    let mut record_name = stem.to_os_string();
    record_name.push(RECORD_SUFFIX);

    let created = openat(
        &chosen.folders.info,
        &record_name,
        OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    );
    let fd = match created {
        Ok(fd) => fd,
        Err(Errno::EXIST) => return Ok(None),
        Err(errno) => return Err(io::Error::from(errno)),
    };

    if let Err(error) = File::from(fd).write_all(text.as_bytes()) {
        remove_record(chosen, &record_name);

        return Err(error);
    }

    Ok(Some(record_name))
}

/// Removes a record that no item stands behind. One that cannot be removed is left, and does no harm.
fn remove_record(chosen: &Chosen, record_name: &OsStr) {
    if let Err(errno) = unlinkat(&chosen.folders.info, record_name, AtFlags::empty()) {
        tracing::warn!(%errno, "cannot remove a record of the Trash");
    }
}

/// The name an item has in the Trash on its `number`th try: its own name, then `name.2`, `name.3`, ...
///
/// The name of the record is this and `.trashinfo`, and must fit in a name of the file system too, so a long name is
/// cut to make room (at a character where the name is text, else at a byte). Two names cut to the same stem are told
/// apart by the number, as any two items of one name are: the stem that is taken is skipped.
fn stem(name: &OsStr, number: u32) -> OsString {
    let number_part = if number > 1 {
        format!(".{number}")
    } else {
        String::new()
    };
    let room = NAME_MAX - RECORD_SUFFIX.len() - number_part.len();

    let bytes = name.as_bytes();
    let mut cut = bytes.len().min(room);

    // A cut in the middle of a character would leave a name that is not text.
    if let Some(text) = name.to_str() {
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
    }

    let mut stem = bytes[..cut].to_vec();
    stem.extend_from_slice(number_part.as_bytes());

    OsString::from_vec(stem)
}

#[cfg(test)]
mod tests;

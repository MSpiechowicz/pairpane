//! Which Trash folder an item goes to, opened as handles that can be trusted.
//!
//! The home Trash (`$XDG_DATA_HOME/Trash`) takes the items of the drive it is on; every other drive has its own Trash
//! in its top folder: `.Trash/$uid` when the drive has a sticky `.Trash` with a folder of the user in it that can be
//! used, else `.Trash-$uid` (made when it is missing). A drive that has none the user can use has no Trash: the item
//! is refused, never copied to another disk.
//!
//! The home is looked up only when an item is moved, and a home that cannot be reached (a folder on the way that the
//! user may not search) costs only the items of the drive it is on: those have no usable Trash, and the items of
//! other drives still go to theirs.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use super::handle::{Facts, Rule, Step, open_folder, step, step_or_make, unusable};
use super::top::{Top, top_of};

/// The system facts the choice depends on, so that tests can give another user and another home.
#[derive(Debug, Clone)]
pub(super) struct Context {
    pub uid: u32,
    /// The data folder that holds the home Trash: `$XDG_DATA_HOME`, else `$HOME/.local/share`. `None` when the
    /// environment names none.
    pub data_home: Option<PathBuf>,
}

impl Context {
    pub(super) fn system() -> Self {
        Self {
            uid: rustix::process::getuid().as_raw(),
            data_home: data_home(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME")),
        }
    }
}

/// The data folder of the home Trash by the Trash specification: `$XDG_DATA_HOME` when it is an absolute path, else
/// `$HOME/.local/share` (an absolute `$HOME`). An empty variable counts as not set.
pub(super) fn data_home(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |value: OsString| Some(PathBuf::from(value)).filter(|path| path.is_absolute());

    xdg.and_then(absolute)
        .or_else(|| home.and_then(absolute).map(|home| home.join(".local/share")))
}

/// Which of the Trash folders of a drive it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// `$XDG_DATA_HOME/Trash`; the base is the data folder.
    Home,
    /// `$top/.Trash/$uid`; the base is the top folder of the drive.
    Shared,
    /// `$top/.Trash-$uid`; the base is the top folder of the drive.
    Own,
}

/// The two folders of a Trash an item is moved into, opened.
#[derive(Debug)]
pub(super) struct Folders {
    pub files: OwnedFd,
    pub info: OwnedFd,
}

/// The Trash folder that an item goes to.
#[derive(Debug)]
pub(super) struct Chosen {
    pub folders: Folders,
    pub kind: Kind,
    /// The folder the Trash is found from again: the data folder (home Trash) or the top folder of the drive.
    pub base: PathBuf,
}

/// The nearest folder that exists on the way to a path, and the parts below it that do not exist yet.
struct Anchor {
    fd: OwnedFd,
    missing: Vec<OsString>,
}

impl Anchor {
    /// Opens `path`, or its nearest parent that exists (links on the way are followed: the path is the user's).
    fn find(path: &Path) -> io::Result<Self> {
        let mut missing = Vec::new();
        let mut at = path;

        loop {
            match open_folder(at) {
                Ok(fd) => return Ok(Self { fd, missing }),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let (Some(part), Some(parent)) = (at.file_name(), at.parent()) else {
                        return Err(unusable("the data folder cannot be found"));
                    };

                    missing.push(part.to_owned());
                    at = parent;
                }
                Err(error) => {
                    tracing::debug!(?error, "the data folder cannot be opened");

                    return Err(unusable("the data folder cannot be opened"));
                }
            }
        }
    }

    /// The data folder, with the parts that were missing made (permissions of the owner only).
    fn complete(self, uid: u32) -> io::Result<OwnedFd> {
        let mut fd = self.fd;

        for part in self.missing.iter().rev() {
            fd = step_or_make(&fd, part, Rule::Own, uid).or_refuse("a part of the data folder cannot be made")?;
        }

        Ok(fd)
    }
}

/// Chooses the Trash folder for an item in the folder `folder` (opened as a handle, its path resolved being
/// `canonical`), made when it is missing. Everything of it must be on `device`, the device of the folder.
///
/// # Errors
///
/// A refusal with [`super::handle::NO_TRASH`] when there is no usable Trash folder, or an error of the system.
pub(super) fn choose(context: &Context, canonical: &Path, folder: &OwnedFd, device: u64) -> io::Result<Chosen> {
    let uid = context.uid;
    let Some(data_home) = context.data_home.as_deref() else {
        return Err(unusable("the environment names no home"));
    };

    let top = top_of(folder)?;

    let located = Anchor::find(data_home).and_then(|anchor| top_of(&anchor.fd).map(|top| (anchor, top)));
    let (anchor, home_top) = match located {
        Ok(found) => found,
        Err(error) => {
            tracing::debug!(?error, "the data folder of the home Trash cannot be reached");

            return other_drive_or_refuse(uid, canonical, &top, device, data_home);
        }
    };

    // The home Trash is the Trash of the drive the home is on: the same top folder, by identity.
    if Facts::of_handle(&top.fd)?.is_same_place(&Facts::of_handle(&home_top.fd)?) {
        let data = anchor.complete(uid)?;
        let trash =
            step_or_make(&data, OsStr::new("Trash"), Rule::Own, uid).or_refuse("the home Trash cannot be used")?;

        return Ok(Chosen {
            folders: made_folders(&trash, uid, device)?,
            kind: Kind::Home,
            base: data_home.to_path_buf(),
        });
    }

    volume(uid, canonical, &top, device)
}

/// The home cannot be reached, so which drive it is on is not known exactly. The nearest folder of its path that can
/// be opened stands for it: an item on another drive than that goes to the Trash of its own drive, and an item on the
/// same one is refused, as the home Trash it should go to cannot be used.
fn other_drive_or_refuse(uid: u32, canonical: &Path, top: &Top, device: u64, data_home: &Path) -> io::Result<Chosen> {
    let near_top = data_home
        .ancestors()
        .find_map(|path| open_folder(path).ok())
        .and_then(|fd| top_of(&fd).ok());

    let elsewhere = match near_top {
        Some(near) => !Facts::of_handle(&top.fd)?.is_same_place(&Facts::of_handle(&near.fd)?),
        None => false,
    };

    if elsewhere {
        return volume(uid, canonical, top, device);
    }

    Err(unusable("the home Trash cannot be reached"))
}

/// The Trash folder of the drive whose top folder is `top`.
fn volume(uid: u32, canonical: &Path, top: &Top, device: u64) -> io::Result<Chosen> {
    // The path of the top folder is what the Trash is found from again; it must lead to the folder that was found.
    let Some(base) = canonical.ancestors().nth(top.steps) else {
        return Err(unusable("the top folder of the drive has no path"));
    };
    let wanted = Facts::of_handle(&top.fd)?;
    if !open_folder(base)
        .and_then(|fd| Facts::of_handle(&fd))
        .is_ok_and(|found| found.is_same_place(&wanted))
    {
        return Err(unusable("the path of the top folder leads elsewhere"));
    }
    if !Rule::Guarded.allows(&wanted, uid) {
        return Err(unusable("others can rename what is in the top folder"));
    }

    // A folder of the user in `.Trash` that cannot be used (a link, or one of someone else, planted there) is passed
    // over like a missing one: the top folder is guarded, so the Trash of the user in it is theirs alone.
    let (trash, kind) = match shared(&top.fd, uid) {
        Step::Open(shared) => (shared, Kind::Shared),
        Step::Missing | Step::Refused => {
            let own = step_or_make(&top.fd, &own_name(uid), Rule::Own, uid)
                .or_refuse("the Trash of the drive cannot be used")?;

            (own, Kind::Own)
        }
    };

    Ok(Chosen {
        folders: made_folders(&trash, uid, device)?,
        kind,
        base: base.to_path_buf(),
    })
}

/// `$top/.Trash/$uid`: [`Step::Missing`] when there is no usable `.Trash` (guarded and sticky) or no folder of the user
/// in it, [`Step::Refused`] when the folder of the user is there and cannot be used.
fn shared(top: &OwnedFd, uid: u32) -> Step {
    let Step::Open(shared) = step(top, OsStr::new(".Trash"), Rule::Shared, uid) else {
        return Step::Missing;
    };

    step(&shared, OsStr::new(&uid.to_string()), Rule::Own, uid)
}

/// The name of the Trash folder of the user in the top folder of a drive.
fn own_name(uid: u32) -> OsString {
    OsString::from(format!(".Trash-{uid}"))
}

/// `files` and `info` of the Trash folder `trash`, made when they are missing, all on `device`.
fn made_folders(trash: &OwnedFd, uid: u32, device: u64) -> io::Result<Folders> {
    open_folders(trash, uid, device, true)?.ok_or_else(|| unusable("a folder of the Trash cannot be made"))
}

/// `files` and `info` of the Trash folder `trash`, all on `device`; made when `make` and missing, else `None` when
/// one is missing.
fn open_folders(trash: &OwnedFd, uid: u32, device: u64, make: bool) -> io::Result<Option<Folders>> {
    if Facts::of_handle(trash)?.device != device {
        return Err(unusable("the Trash is on another disk than the item"));
    }

    let mut opened = Vec::with_capacity(2);

    for name in ["files", "info"] {
        let name = OsStr::new(name);
        let found = if make {
            step_or_make(trash, name, Rule::Own, uid)
        } else {
            step(trash, name, Rule::Own, uid)
        };

        match found {
            Step::Open(fd) if Facts::of_handle(&fd)?.device == device => opened.push(fd),
            Step::Open(_) => return Err(unusable("a folder of the Trash is on another disk than the item")),
            Step::Missing => return Ok(None),
            Step::Refused => return Err(unusable("a folder of the Trash cannot be used")),
        }
    }

    let (Some(info), Some(files)) = (opened.pop(), opened.pop()) else {
        return Ok(None);
    };

    Ok(Some(Folders { files, info }))
}

/// The Trash folders an item was moved to, opened again for the undo. Nothing is made.
#[derive(Debug)]
pub(super) enum Reopened {
    /// The Trash folder, or one of its folders, is not there any more.
    Gone,
    Open(Folders),
}

/// Opens the Trash folders again, from the `base` and `kind` that were recorded, with the same rules as when they were
/// chosen.
///
/// # Errors
///
/// A refusal with [`super::handle::NO_TRASH`] when what is there cannot be used.
pub(super) fn reopen(uid: u32, kind: Kind, base: &Path, device: u64) -> io::Result<Reopened> {
    let base = match open_folder(base) {
        Ok(fd) => fd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Reopened::Gone),
        Err(_) => return Err(unusable("the folder the Trash is found from cannot be opened")),
    };

    if kind != Kind::Home && !Rule::Guarded.allows(&Facts::of_handle(&base)?, uid) {
        return Err(unusable("others can rename what is in the top folder"));
    }

    let trash = match kind {
        Kind::Home => step(&base, OsStr::new("Trash"), Rule::Own, uid),
        Kind::Own => step(&base, &own_name(uid), Rule::Own, uid),
        Kind::Shared => shared(&base, uid),
    };

    match trash {
        Step::Missing => Ok(Reopened::Gone),
        Step::Refused => Err(unusable("the Trash folder cannot be used")),
        Step::Open(trash) => Ok(open_folders(&trash, uid, device, false)?.map_or(Reopened::Gone, Reopened::Open)),
    }
}

#[cfg(test)]
mod tests;

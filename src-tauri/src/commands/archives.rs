//! What the archive commands share: the archive path check, the archive opened without following a link, the indexes
//! kept for the archives that were opened lately, the passwords the user gave and the cap on the threads that read
//! archives.
//!
//! An archive is only ever read: an index is built from its directory (never by extracting) on a blocking thread, which
//! holds a place of a [`ThreadCap`] until it ends, as a read on a hung mount never returns (see `limit.rs`). A password
//! lives in memory only, per archive path, and is never logged, sent in an event or saved; the commands that take one
//! (`archive_password`) put it in a [`Password`], which hides itself from `Debug`. A password fault of the archive code
//! reaches the page as `passwordRequired`, with "needs a password" or "the password is wrong" (the page tells the two
//! apart by the word "wrong").
//!
//! - `panes/archive.rs`: `archive_open`. `ops/extract.rs`: `ops_extract`. `ops/add.rs`: `ops_add_to_archive`.
//!   `view/archive_entry.rs`: `archive_view_open`. (`ops_pack` writes a new archive and reads none.)

use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::archive::{
    self, ArchiveIndex, Fingerprint, HEAD_BYTES, IndexCache, Input, MAX_NAME_BYTES, Password, PasswordFault, ReadAt,
};
use pairpane_core::listing::platform;
use pairpane_core::model::{ArchiveFormat, IpcError, IpcErrorCode};
use pairpane_core::ops::name_fits;
use pairpane_core::view::{is_refusal, refusal};
use tauri::State;

use super::items::{begin, blocking};
use super::limit::ThreadCap;
use super::ops::{check_name, failed, names_too_large};
use super::panes::resolve;
use crate::error::{CommandError, CommandResult};

#[cfg(test)]
pub(super) mod fixtures;
#[cfg(test)]
mod tests;

/// Threads, apart from the panes', that read archives at once (a password check, the checks before an extraction, a
/// file opened in the viewer). A stuck one is stepped around by the others.
const ARCHIVE_THREADS: usize = 3;

/// The most passwords kept; past it an arbitrary one is forgotten (it is asked for again).
const MAX_PASSWORDS: usize = 64;

/// The longest password accepted, in bytes.
const MAX_PASSWORD_BYTES: usize = 1024;

/// The indexes and passwords of the archives opened lately, the cap on the threads that read them and the flag that stops
/// them all when the app exits. Clones share all of it (a listing thread holds one).
#[derive(Clone, Default)]
pub struct Archives {
    cache: Arc<IndexCache>,
    passwords: Arc<Mutex<HashMap<PathBuf, Arc<Password>>>>,
    work: ThreadCap,
    /// Raised once, when the app exits: the reads that have no cancel flag of their own (a password check, the checks
    /// before an extraction, a file opened in the viewer) stop with it.
    shutdown: Arc<AtomicBool>,
}

/// A checked archive path: the folder that holds the archive, its plain name and the two joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) folder: PathBuf,
    pub(super) name: String,
    /// `folder` and `name` joined: the path the pane shows, the engine fingerprints and the passwords are kept by.
    pub(super) path: PathBuf,
}

impl Target {
    /// Checks the absolute `archive` path the page sent, as strictly as a folder is checked: no `..`, no device or
    /// network prefix, and a plain file name. Touches no disk.
    pub(super) fn parse(archive: String) -> CommandResult<Self> {
        let path = PathBuf::from(archive);
        let name = path.file_name().and_then(|name| name.to_str());
        let folder = path.parent().and_then(Path::to_str);
        let (Some(name), Some(folder)) = (name, folder) else {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "an archive must be a file with a plain name",
            ));
        };

        if !name_fits(name) {
            return Err(names_too_large());
        }
        let folder = resolve(Some(folder.to_owned()), None)?;
        check_name(&folder, name)?;

        Ok(Self {
            path: folder.join(name),
            name: name.to_owned(),
            folder,
        })
    }
}

/// An archive file that is open, and the state it was in when it was opened.
pub(super) struct Opened {
    pub(super) file: File,
    pub(super) fingerprint: Fingerprint,
    /// Where it is read at: the path that [`platform::local_path`] resolved (holding no link) for the folder, and the name.
    pub(super) at: PathBuf,
}

impl Opened {
    /// The archive as the readers take it.
    pub(super) fn input(&self) -> Input<'_> {
        Input {
            file: &self.file,
            path: Some(&self.at),
        }
    }
}

impl Archives {
    /// Takes a place of the cap on archive reading, or refuses as busy.
    pub(super) fn begin(&self) -> CommandResult<super::limit::Permit> {
        begin(&self.work, ARCHIVE_THREADS)
    }

    /// The password given for the archive, if any.
    pub(super) fn password(&self, archive: &Path) -> Option<Arc<Password>> {
        self.lock().get(archive).cloned()
    }

    /// Drops the password kept for the archive, so the page is asked for one again.
    pub(super) fn forget(&self, archive: &Path) {
        self.lock().remove(archive);
    }

    /// Drops the index kept for the archive, whatever state of the file it was made from, so the next open reads the
    /// archive again (it was replaced, or may have been).
    pub(super) fn forget_index(&self, archive: &Path) {
        self.cache.forget(archive);
    }

    /// [`Archives::forget`], but only when the password kept is `used` (the very one): a job that learns its password was
    /// wrong must not drop the one the user gave after it started.
    pub(super) fn forget_used(&self, archive: &Path, used: &Arc<Password>) {
        let mut passwords = self.lock();

        if passwords.get(archive).is_some_and(|kept| Arc::ptr_eq(kept, used)) {
            passwords.remove(archive);
        }
    }

    /// Stops every read that holds [`Archives::stop`]; called when the app exits, before the temporary folders are swept.
    pub fn shut_down(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// The flag [`Archives::shut_down`] raises: the cancel flag of the reads that no page cancels.
    pub(super) fn stop(&self) -> &AtomicBool {
        &self.shutdown
    }

    /// [`archive_error`] of `error`, met reading the archive `target` with the password kept for it. A password that
    /// the archive turned down is dropped: a check can accept a wrong one by chance, and it must not fail every later
    /// read, with the page unable to ask for another.
    pub(super) fn report(&self, target: &Target, error: &io::Error) -> IpcError {
        if archive::password_fault(error) == Some(PasswordFault::Wrong) {
            self.forget(&target.path);
        }

        archive_error(error)
    }

    pub(super) fn remember(&self, archive: PathBuf, password: Password) {
        let mut passwords = self.lock();

        if passwords.len() >= MAX_PASSWORDS
            && !passwords.contains_key(&archive)
            && let Some(old) = passwords.keys().next().cloned()
        {
            passwords.remove(&old);
        }
        passwords.insert(archive, Arc::new(password));
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, Arc<Password>>> {
        self.passwords.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the archive without following a link: it must be a regular file now (not a link, a folder or a pipe) and
    /// the file opened must be that one. Blocking.
    pub(super) fn open(&self, target: &Target) -> io::Result<Opened> {
        let at = platform::local_path(&target.folder)?.join(&target.name);
        let seen = fs::symlink_metadata(&at)?;
        if !seen.is_file() {
            return Err(not_an_archive());
        }

        let file = File::open(&at)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || platform::identity(&metadata) != platform::identity(&seen) {
            return Err(refusal(io::ErrorKind::Other, "the archive changed; open it again"));
        }

        Ok(Opened {
            fingerprint: Fingerprint::of(&target.path, &metadata),
            file,
            at,
        })
    }

    /// Which kind of archive the open file is.
    pub(super) fn format(&self, target: &Target, opened: &Opened) -> io::Result<ArchiveFormat> {
        let head = opened.file.read_chunk(0, HEAD_BYTES)?;

        archive::detect(&target.name, &head).ok_or_else(not_an_archive)
    }

    /// The index of the archive if it was kept for this state of the file.
    pub(super) fn cached(&self, opened: &Opened) -> Option<Arc<ArchiveIndex>> {
        self.cache.get(&opened.fingerprint)
    }

    /// The index of the archive: the one kept for this state of the file, or one built now (and kept). `candidate` is
    /// a password to try before the one the user gave earlier. Stops with `Interrupted` when `cancel` is raised.
    /// Blocking.
    pub(super) fn index(
        &self,
        target: &Target,
        opened: &Opened,
        cancel: &AtomicBool,
        candidate: Option<&Password>,
    ) -> io::Result<Arc<ArchiveIndex>> {
        if let Some(index) = self.cached(opened) {
            return Ok(index);
        }

        let format = self.format(target, opened)?;
        let stored = self.password(&target.path);
        let password = candidate.or(stored.as_deref());
        let index = Arc::new(archive::build_index(format, &opened.input(), cancel, password)?);
        self.cache.put(opened.fingerprint.clone(), Arc::clone(&index));

        Ok(index)
    }
}

/// The archive is not a file this build opens.
fn not_an_archive() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "this is not an archive that can be opened")
}

/// What the page is told when reading an archive fails: a password fault as `passwordRequired` and its fixed text,
/// our own refusals with their text (which holds no path), and for anything else only the kind of failure. Never a path,
/// never a password.
pub(super) fn archive_error(error: &io::Error) -> IpcError {
    if let Some(fault) = archive::password_fault(error) {
        return IpcError::new(IpcErrorCode::PasswordRequired, archive::fault_text(fault));
    }

    // Before the other "not found": it is the `unrar` program that is missing, not the archive.
    if archive::rar::is_missing(error) {
        return IpcError::new(IpcErrorCode::Unsupported, archive::rar::MISSING);
    }

    let own = is_refusal(error);
    let (code, message) = match error.kind() {
        io::ErrorKind::NotFound => (IpcErrorCode::NotFound, "This archive does not exist.".to_owned()),
        io::ErrorKind::Interrupted => (IpcErrorCode::Io, "the read was cancelled".to_owned()),
        io::ErrorKind::PermissionDenied => (
            IpcErrorCode::Io,
            "You do not have permission to open this archive.".to_owned(),
        ),
        io::ErrorKind::InvalidInput if own => (IpcErrorCode::InvalidInput, error.to_string()),
        io::ErrorKind::Unsupported if own => (IpcErrorCode::Unsupported, error.to_string()),
        _ if own => (IpcErrorCode::Io, error.to_string()),
        _ => {
            tracing::debug!(%error, "archive read failed");

            (IpcErrorCode::Io, "the archive cannot be read".to_owned())
        }
    };

    IpcError::new(code, message)
}

/// A folder inside an archive that the page sent: short enough to be looked up (it is never used as a path).
pub(super) fn check_inner(dir: &str) -> CommandResult<()> {
    if dir.len() > MAX_NAME_BYTES {
        return Err(names_too_large());
    }

    Ok(())
}

/// A password the page sent: between 1 and [`MAX_PASSWORD_BYTES`] bytes. It is wrapped at once, so it never appears in an
/// error, an event or a log.
pub(super) fn checked_password(password: String) -> CommandResult<Password> {
    let password = Password::new(password);

    if password.as_bytes().is_empty() || password.as_bytes().len() > MAX_PASSWORD_BYTES {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "a password must have between 1 and 1024 bytes",
        ));
    }

    Ok(password)
}

/// Whether `password` opens the archive at `archive`, and if it does, keeps it for the archive until the app exits. The
/// check is cheap (see [`archive::check_password`]): a format with nothing to check ahead accepts any password, and a
/// later failure asks again. A wrong password is not kept.
#[tauri::command]
pub async fn archive_password(archives: State<'_, Archives>, archive: String, password: String) -> CommandResult<bool> {
    archives.remember_if_right(archive, password).await
}

impl Archives {
    /// Checks the arguments of [`archive_password`] and the password, on a blocking thread.
    pub(super) async fn remember_if_right(&self, archive: String, password: String) -> CommandResult<bool> {
        let password = checked_password(password)?;
        let target = Target::parse(archive)?;
        let permit = self.begin()?;

        let shared = self.clone();
        let checked = blocking(permit, move || shared.check(&target, password)).await?;

        checked.map_err(|error| failed(archive_error(&error)))
    }

    /// The password check on a blocking thread.
    fn check(&self, target: &Target, password: Password) -> io::Result<bool> {
        let opened = self.open(target)?;

        let index = match self.index(target, &opened, self.stop(), Some(&password)) {
            Ok(index) => index,
            // A header that stays shut with this password is a wrong password, not a failure.
            Err(error) if archive::password_fault(&error).is_some() => return Ok(false),
            Err(error) => return Err(error),
        };

        if !archive::check_password(&index, &opened.file, &password)? {
            return Ok(false);
        }
        self.remember(target.path.clone(), password);

        Ok(true)
    }
}

//! `archive_view_open`: an entry of an archive opened in the viewer.
//!
//! The viewer reads a file at offsets, so the entry is extracted first, into a file of a private temporary folder
//! (`pairpane-view-<random>` in the system's temporary folder, mode 0700 on Unix, the file created new with mode 0600),
//! opened as a session like `view_open` does, and deleted at once: the session keeps the open handle, so nothing of the
//! entry stays on the disk once it is open. On Windows a file that cannot be deleted while it is open is left, and the
//! sweep ([`sweep`], at start-up and on exit) removes it later. An entry over [`MAX_VIEW_ENTRY`] is refused; the data
//! written is bounded by that, whatever the archive declares. Listing and extraction never go through here.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use pairpane_core::archive::{self, Flow, MAX_VIEW_ENTRY, PasswordFault};
use pairpane_core::listing::platform;
use pairpane_core::model::{IpcError, IpcErrorCode, ViewOpened};
use pairpane_core::ops::name_fits;
use pairpane_core::view::{self as files, Opened, refusal};
use tauri::State;

use super::Viewer;
use crate::commands::archives::{Archives, Target, archive_error, check_inner};
use crate::commands::items::blocking;
use crate::commands::ops::{failed, names_too_large};
use crate::error::{CommandError, CommandResult};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

#[cfg(test)]
mod tests;

/// The start of the name of every temporary folder this makes (and the sweep removes).
const PREFIX: &str = "pairpane-view-";

const TOO_BIG: &str = "This file is too big to view inside the archive \u{2014} extract it first (F5)";

/// Opens the file `name` of the folder `dir` (`""` is the root) of the archive `archive` for viewing and returns its
/// session, like `view_open`. A folder, a link, a file whose name the archive code refuses and an entry over 512 MiB are
/// refused; one that needs a password the app does not have fails with `passwordRequired`.
#[tauri::command]
pub async fn archive_view_open(
    viewer: State<'_, Viewer>,
    archives: State<'_, Archives>,
    archive: String,
    dir: String,
    name: String,
) -> CommandResult<ViewOpened> {
    viewer.open_archive_entry(&archives, archive, dir, name).await
}

impl Viewer {
    /// Checks the arguments, extracts the entry on a blocking thread and starts a session on it.
    pub(super) async fn open_archive_entry(
        &self,
        archives: &Archives,
        archive: String,
        dir: String,
        name: String,
    ) -> CommandResult<ViewOpened> {
        let target = Target::parse(archive)?;
        check_inner(&dir)?;
        if !name_fits(&name) {
            return Err(names_too_large());
        }
        if !platform::is_plain_name(&name) {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "an item name must be a single plain name",
            ));
        }

        let permit = archives.begin()?;
        let shared = archives.clone();
        let opened = blocking(permit, move || open_entry(&shared, &target, &dir, &name))
            .await?
            .map_err(failed)?;

        Ok(self.start(opened))
    }
}

/// Extracts the entry into a private temporary file, opens it for the viewer and deletes it.
fn open_entry(archives: &Archives, target: &Target, dir: &str, name: &str) -> Result<Opened, IpcError> {
    let opened = archives.open(target).map_err(|error| archive_error(&error))?;
    let never = AtomicBool::new(false);
    let index = archives
        .index(target, &opened, &never, None)
        .map_err(|error| archives.report(target, &error))?;

    let found = index
        .find(dir, name)
        .ok_or_else(|| IpcError::new(IpcErrorCode::NotFound, "the file is not in the archive"))?;
    let entry = match found.entry {
        Some(entry) if !found.dir => entry,
        _ => return Err(not_a_file(found.dir)),
    };
    if !entry.writable() || entry.kind != archive::EntryType::File {
        return Err(not_a_file(false));
    }

    let password = archives.password(&target.path);
    if entry.encrypted && password.is_none() {
        return Err(archive_error(&archive::password_error(PasswordFault::Needed)));
    }
    if entry.size > MAX_VIEW_ENTRY {
        return Err(IpcError::new(IpcErrorCode::InvalidInput, TOO_BIG));
    }

    let scratch = Scratch::create(name).map_err(|error| archive_error(&error))?;
    let mut file = scratch.open_file().map_err(|error| archive_error(&error))?;
    let mut source = archive::open_source(&index, &opened.input(), &never, password.as_deref())
        .map_err(|error| archives.report(target, &error))?;

    let mut written = false;
    source
        .extract(&[entry.ordinal], &mut |_, reader| {
            copy_capped(reader, &mut file)?;
            written = true;

            Ok(Flow::Stop)
        })
        .map_err(|error| archives.report(target, &error))?;
    if !written {
        return Err(archive_error(&archive::unreadable()));
    }
    drop(file);

    // Opened before it is deleted: the session holds the handle.
    files::open(&scratch.folder, name).map_err(|error| archive_error(&error))
}

/// Writes what `reader` yields to `file`, at most [`MAX_VIEW_ENTRY`] bytes: a byte more means the entry holds more than
/// it declares and fails it. A read loop of its own, not `io::copy`: that retries a read that failed with `Interrupted`,
/// which is how the archive code reports a raised cancel flag, and would never stop.
fn copy_capped(reader: &mut dyn Read, file: &mut File) -> io::Result<()> {
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }

        total += count as u64;
        if total > MAX_VIEW_ENTRY {
            return Err(refusal(io::ErrorKind::InvalidInput, TOO_BIG));
        }
        file.write_all(&buffer[..count])?;
    }
}

fn not_a_file(folder: bool) -> IpcError {
    let text = if folder {
        "the viewer opens files, not folders"
    } else {
        "the viewer opens regular files only"
    };

    IpcError::new(IpcErrorCode::InvalidInput, text)
}

/// A private temporary folder and the file in it, both removed when it is dropped (a failure to remove them, which can
/// happen on Windows while the file is open, is left to the sweep).
struct Scratch {
    folder: PathBuf,
    file: PathBuf,
}

impl Scratch {
    /// Makes the folder, which nobody else can enter, for a file called `name`.
    fn create(name: &str) -> io::Result<Self> {
        let folder = std::env::temp_dir().join(format!("{PREFIX}{}", random_hex()?));
        private_folder(&folder)?;

        Ok(Self {
            file: folder.join(name),
            folder,
        })
    }

    /// Creates the file, new and private, for writing.
    fn open_file(&self) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);

        options.open(&self.file)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let file = fs::remove_file(&self.file);
        let folder = fs::remove_dir(&self.folder);

        let gone = |result: &io::Result<()>| result.is_ok() || is_missing(result);
        if !(gone(&file) && gone(&folder)) {
            tracing::warn!("a temporary file of the viewer was left; it is removed at the next start");
        }
    }
}

fn is_missing(result: &io::Result<()>) -> bool {
    result
        .as_ref()
        .err()
        .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
}

/// 128 random bits as hex.
fn random_hex() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| refusal(io::ErrorKind::Other, "no temporary file can be made"))?;

    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Creates the folder `path`, which must not exist yet, for its owner alone.
fn private_folder(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);

    builder.create(path)
}

/// Removes the temporary folders of the viewer that an earlier run (or this one) left: the folders named
/// `pairpane-view-…` in the temporary folder that are real folders (never a link) and, on Unix, ours. Run at
/// start-up and on exit, off the UI's way.
pub fn sweep() {
    sweep_in(&std::env::temp_dir());
}

/// [`sweep`] of the folder `temp`.
fn sweep_in(temp: &Path) {
    let Ok(listing) = fs::read_dir(temp) else {
        return;
    };
    let owner = owner(temp);

    for entry in listing.flatten() {
        let named = entry.file_name().to_str().is_some_and(|name| name.starts_with(PREFIX));
        if !named {
            continue;
        }

        let Ok(seen) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if seen.is_dir() && is_ours(&seen, owner) {
            fs::remove_dir_all(entry.path()).ok();
        }
    }
}

/// Who we are, as the owner of a folder we make: the system tells no other way without `unsafe`.
#[cfg(unix)]
fn owner(temp: &Path) -> Option<u32> {
    let probe = temp.join(format!("pairpane-owner-{}", random_hex().ok()?));
    private_folder(&probe).ok()?;
    let uid = fs::symlink_metadata(&probe).ok().map(|seen| seen.uid());
    fs::remove_dir(&probe).ok();

    uid
}

#[cfg(unix)]
fn is_ours(seen: &fs::Metadata, owner: Option<u32>) -> bool {
    owner.is_some_and(|uid| seen.uid() == uid)
}

/// The temporary folder of a Windows user is that user's own.
#[cfg(not(unix))]
fn owner(_temp: &Path) -> Option<u32> {
    None
}

#[cfg(not(unix))]
fn is_ours(_seen: &fs::Metadata, _owner: Option<u32>) -> bool {
    true
}

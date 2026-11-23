//! `archive_view_open`: an entry of an archive opened in the viewer; and `archive_edit_open`: one read as text for a
//! comparison (Shift+F2), into memory only, at most what the editor takes, and read-only.
//!
//! The viewer reads a file at offsets, so the entry is extracted first, into a file of a private temporary folder
//! (`pairpane-view-<random>` in the system's temporary folder, mode 0700 on Unix, the file created new with mode 0600),
//! opened as a session like `view_open` does, and deleted at once: the session keeps the open handle, so nothing of the
//! entry stays on the disk once it is open. On Windows a file that cannot be deleted while it is open is left, and the
//! sweep ([`sweep`], at start-up and on exit) removes it later. An entry over [`MAX_VIEW_ENTRY`] is refused, and so is one that
//! would leave the temporary volume with less than the reserve ([`MIN_FREE_RESERVE`]); the data written is bounded by
//! [`MAX_VIEW_ENTRY`], whatever the archive declares. Listing and extraction never go through here.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use pairpane_core::archive::{self, Flow, MAX_VIEW_ENTRY, MIN_FREE_RESERVE, PasswordFault};
use pairpane_core::edit::{self, MAX_EDIT};
use pairpane_core::listing::platform;
use pairpane_core::model::{EditOpened, IpcError, IpcErrorCode, ViewOpened};
use pairpane_core::ops::name_fits;
use pairpane_core::view::{self as files, Opened, is_known_encoding, refusal};
use tauri::State;

use super::Viewer;
use crate::commands::archives::{Archives, Target, archive_error, check_inner};
use crate::commands::items::blocking;
use crate::commands::ops::{failed, names_too_large};
use crate::error::{CommandError, CommandResult};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod tests;

/// The start of the name of every temporary folder this makes (and the sweep removes).
const PREFIX: &str = "pairpane-view-";

const TOO_BIG: &str = "This file is too big to view inside the archive \u{2014} extract it first (F5)";

const NO_ROOM: &str = "Not enough free space to view this file \u{2014} extract it first (F5)";

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

/// Reads the file `name` of the folder `dir` of the archive `archive` as text for a comparison, in the encoding `encoding`
/// labels or the one found from it: read-only, as an archive is never written from the editor. Refused as
/// `archive_view_open` refuses; one bigger than the editor takes is `tooLarge`. Read into memory, never onto the disk.
#[tauri::command]
pub async fn archive_edit_open(
    archives: State<'_, Archives>,
    archive: String,
    dir: String,
    name: String,
    encoding: Option<String>,
) -> CommandResult<EditOpened> {
    edit_entry(&archives, (archive, dir, name), encoding, MAX_EDIT as u64).await
}

/// [`archive_edit_open`] for entries of at most `most` bytes.
async fn edit_entry(
    archives: &Archives,
    (archive, dir, name): (String, String, String),
    encoding: Option<String>,
    most: u64,
) -> CommandResult<EditOpened> {
    let target = checked(archive, &dir, &name)?;
    // An encoding that is not known is refused before anything is read, as `edit_open` does.
    if encoding.as_deref().is_some_and(|label| !is_known_encoding(label)) {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "the encoding is not known",
        ));
    }
    let permit = archives.begin()?;
    let shared = archives.clone();

    let read = blocking(permit, move || {
        read_entry(&shared, &target, (&dir, &name), most).map(|bytes| (bytes, name))
    })
    .await?
    .map_err(failed)?;

    match read {
        (Some(bytes), name) => {
            edit::open_bytes(&name, &bytes, encoding.as_deref()).map_err(|error| failed(archive_error(&error)))
        }
        (None, _) => Ok(EditOpened::TooLarge { size: most + 1 }),
    }
}

/// The archive of `archive`, when the folder `dir` and the name `name` in it pass the checks of a request.
fn checked(archive: String, dir: &str, name: &str) -> CommandResult<Target> {
    let target = Target::parse(archive)?;
    check_inner(dir)?;
    if !name_fits(name) {
        return Err(names_too_large());
    }
    if !platform::is_plain_name(name) {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "an item name must be a single plain name",
        ));
    }

    Ok(target)
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
        let target = checked(archive, &dir, &name)?;
        let permit = archives.begin()?;
        let shared = archives.clone();
        let opened = blocking(permit, move || open_entry(&shared, &target, &dir, &name))
            .await?
            .map_err(failed)?;

        Ok(self.start(opened))
    }
}

/// The file entry `name` of the folder `dir` of the archive, found in its index, with the archive opened and its password:
/// what both a session of the viewer and a text for a comparison read. A folder, a link, a name the archive code refuses
/// and an encrypted entry with no password are refused.
fn find_entry(archives: &Archives, target: &Target, dir: &str, name: &str) -> Result<Found, IpcError> {
    let opened = archives.open(target).map_err(|error| archive_error(&error))?;
    let index = archives
        .index(target, &opened, archives.stop(), None)
        .map_err(|error| archives.report(target, &error))?;

    let found = index
        .find(dir, name)
        .ok_or_else(|| IpcError::new(IpcErrorCode::NotFound, "the file is not in the archive"))?;
    let entry = match found.entry {
        Some(entry) if !found.dir => entry.clone(),
        _ => return Err(not_a_file(found.dir)),
    };
    if !entry.writable() || entry.kind != archive::EntryType::File {
        return Err(not_a_file(false));
    }

    let secret = archives.password(&target.path);
    if entry.encrypted && secret.is_none() {
        return Err(archive_error(&archive::password_error(PasswordFault::Needed)));
    }

    Ok(Found {
        opened,
        index,
        entry,
        secret,
    })
}

/// An entry found in its archive (see [`find_entry`]).
struct Found {
    opened: crate::commands::archives::Opened,
    index: std::sync::Arc<archive::ArchiveIndex>,
    entry: archive::IndexEntry,
    secret: Option<std::sync::Arc<archive::Password>>,
}

/// Hands the data of the entry `found` to `write`; fails when the archive yields none.
fn extract(
    archives: &Archives,
    target: &Target,
    found: &Found,
    write: &mut dyn FnMut(&mut dyn Read) -> io::Result<()>,
) -> Result<(), IpcError> {
    let mut source = archive::open_source(
        &found.index,
        &found.opened.input(),
        archives.stop(),
        found.secret.as_deref(),
    )
    .map_err(|error| archives.report(target, &error))?;

    let mut written = false;
    source
        .extract(&[found.entry.ordinal], &mut |_, reader| {
            write(reader)?;
            written = true;

            Ok(Flow::Stop)
        })
        .map_err(|error| archives.report(target, &error))?;
    if !written {
        return Err(archive_error(&archive::unreadable()));
    }

    Ok(())
}

/// Extracts the entry into a private temporary file, opens it for the viewer and deletes it.
fn open_entry(archives: &Archives, target: &Target, dir: &str, name: &str) -> Result<Opened, IpcError> {
    let found = find_entry(archives, target, dir, name)?;
    if found.entry.size > MAX_VIEW_ENTRY {
        return Err(IpcError::new(IpcErrorCode::InvalidInput, TOO_BIG));
    }

    if !has_room(platform::free_space(&std::env::temp_dir()), found.entry.size) {
        return Err(IpcError::new(IpcErrorCode::Io, NO_ROOM));
    }

    let scratch = Scratch::create(name).map_err(|error| archive_error(&error))?;
    let mut file = scratch.open_file().map_err(|error| archive_error(&error))?;
    extract(archives, target, &found, &mut |reader| copy_capped(reader, &mut file))?;
    drop(file);

    // Opened before it is deleted: the session holds the handle.
    files::open(&scratch.folder, name).map_err(|error| archive_error(&error))
}

/// The bytes of the file entry `name` of the folder `dir` of the archive, at most `most` of them; `Ok(None)` when it holds
/// more. Read into memory, never onto the disk.
fn read_entry(
    archives: &Archives,
    target: &Target,
    (dir, name): (&str, &str),
    most: u64,
) -> Result<Option<Vec<u8>>, IpcError> {
    let found = find_entry(archives, target, dir, name)?;
    if found.entry.size > most {
        return Ok(None);
    }

    let mut bytes = Vec::new();
    extract(archives, target, &found, &mut |reader| {
        bytes.clear();
        read_capped(reader, &mut bytes, most)
    })?;

    Ok((bytes.len() as u64 <= most).then_some(bytes))
}

/// Whether a temporary volume with `free` bytes (`None`: the system does not tell) can take `size` bytes and still keep
/// [`MIN_FREE_RESERVE`].
fn has_room(free: Option<u64>, size: u64) -> bool {
    free.is_none_or(|free| free >= size.saturating_add(MIN_FREE_RESERVE))
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

/// Reads what `reader` yields into `bytes`, at most `most` bytes and one more (which tells there was more). A read loop of
/// its own, as [`copy_capped`] is: `read_to_end` retries an `Interrupted` read, which is how a raised stop flag is told,
/// and would never stop.
fn read_capped(reader: &mut dyn Read, bytes: &mut Vec<u8>, most: u64) -> io::Result<()> {
    let mut buffer = vec![0_u8; 64 * 1024];
    let limit = usize::try_from(most).unwrap_or(usize::MAX).saturating_add(1);

    while bytes.len() < limit {
        let want = buffer.len().min(limit - bytes.len());
        let count = reader.read(&mut buffer[..want])?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }

    Ok(())
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
    #[cfg_attr(not(unix), expect(unused_mut, reason = "only Unix sets a mode on the builder"))]
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

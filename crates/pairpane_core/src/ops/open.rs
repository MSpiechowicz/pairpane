//! Opening files for a copy without following a link: the system is asked to refuse one, so a link swapped in
//! after the folder was read cannot lead the copy somewhere else.

use std::fs::{DirBuilder, File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

use crate::listing::platform;

use super::report::refusal;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

/// Windows `FILE_FLAG_OPEN_REPARSE_POINT`: open a link itself, not what it leads to.
#[cfg(windows)]
pub(super) const OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// Windows `FILE_FLAG_BACKUP_SEMANTICS`: needed to open a folder.
#[cfg(windows)]
pub(super) const BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// Windows `FILE_ATTRIBUTE_REPARSE_POINT`.
#[cfg(windows)]
const ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

/// Windows `FILE_WRITE_ATTRIBUTES`: enough to set the times of a folder.
#[cfg(windows)]
const WRITE_ATTRIBUTES: u32 = 0x0100;

fn changed() -> io::Error {
    refusal(io::ErrorKind::Other, "it changed while it was being copied")
}

/// Opens the regular file at `path` for reading. `seen` is what the folder walk saw there: the file must still
/// be a regular file, and on Unix the same one (same device and inode). A link, a pipe or a swapped file is an
/// error; a pipe is never waited on.
pub(super) fn source(path: &Path, seen: &Metadata) -> io::Result<File> {
    let (file, metadata) = open_source(path)?;

    if !metadata.is_file() || platform::identity(&metadata) != platform::identity(seen) {
        return Err(changed());
    }

    Ok(file)
}

#[cfg(unix)]
fn open_source(path: &Path) -> io::Result<(File, Metadata)> {
    use rustix::fs::OFlags;

    // `NOCTTY`: a terminal that slips in as the source (a swapped device file) must not become the job's terminal.
    let flags = (OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY).bits();
    let file = OpenOptions::new().read(true).custom_flags(flags as i32).open(path)?;
    let metadata = file.metadata()?;

    Ok((file, metadata))
}

/// Opens the link itself. A link is refused; a reparse point that is no link (a cloud placeholder, a
/// deduplicated file) is opened again the normal way, which is a race of the same kind as in `resolve_local`.
#[cfg(windows)]
fn open_source(path: &Path) -> io::Result<(File, Metadata)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;

    if metadata.file_type().is_symlink() {
        return Err(changed());
    }

    if metadata.file_attributes() & ATTRIBUTE_REPARSE_POINT == 0 {
        return Ok((file, metadata));
    }

    let file = OpenOptions::new().read(true).open(path)?;
    let metadata = file.metadata()?;

    Ok((file, metadata))
}

/// Creates the new file `path` for reading and writing; fails when anything is there (a link too, which is
/// never followed). On Unix only the owner may use it until its real permissions are set.
pub(super) fn create_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);

    #[cfg(unix)]
    options.mode(0o600);

    options.open(path)
}

/// Opens the file `path` that the system made (a clone) for reading, refusing a link. Its owner can set its
/// times and permissions through this handle, even when the clone is read-only like its source.
#[cfg(target_os = "macos")]
pub(super) fn existing(path: &Path) -> io::Result<File> {
    use rustix::fs::OFlags;

    let flags = (OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY).bits();

    OpenOptions::new().read(true).custom_flags(flags as i32).open(path)
}

/// Creates the folder `path`; fails when anything is there. With `private`, on Unix only the owner may enter
/// it until its real permissions are set, after its contents are in.
#[cfg(unix)]
pub(super) fn create_folder(path: &Path, private: bool) -> io::Result<()> {
    let mut builder = DirBuilder::new();

    if private {
        builder.mode(0o700);
    }

    builder.create(path)
}

#[cfg(not(unix))]
pub(super) fn create_folder(path: &Path, _private: bool) -> io::Result<()> {
    DirBuilder::new().create(path)
}

/// Opens the folder `path` to set its times and permissions, refusing a link.
#[cfg(unix)]
pub(super) fn folder(path: &Path) -> io::Result<File> {
    use rustix::fs::OFlags;

    let flags = (OFlags::NOFOLLOW | OFlags::DIRECTORY).bits();

    OpenOptions::new().read(true).custom_flags(flags as i32).open(path)
}

#[cfg(windows)]
pub(super) fn folder(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(WRITE_ATTRIBUTES)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
}

//! The two file operations the key file and the device list share: a read that cannot be made to take more memory than
//! a limit, and a write that is all or nothing.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};

/// The name of the lock file in the data folder, held by the running service and by anything else that changes the
/// folder's files while it is not running.
pub const LOCK_FILE_NAME: &str = "link.lock";

/// Reads the whole file, or `None` when it does not exist. A file of more than `cap` bytes is refused with
/// `InvalidData` and is never read past `cap + 1` bytes, whatever its size claims to be.
pub fn read_capped(path: &Path, cap: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;

    if bytes.len() as u64 > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the file is larger than allowed",
        ));
    }

    Ok(Some(bytes))
}

/// Counts the temporary files of this process, so two writes never share a name.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `dir/name` so a crash leaves either the old file or the new one: the bytes go to a temporary file
/// of the same folder (a name of its own, created fresh, never through a link that is already there; mode 0600 on
/// Unix), are flushed to disk, and the temporary file is renamed over the target. The folder is created when it is
/// missing. Temporary files that an earlier crash left for this name are removed first.
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    create_private_dir(dir)?;
    remove_stale_temps(dir, name);

    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.tmp-{}-{unique}", std::process::id()));

    let mut file = private_options().write(true).create_new(true).open(&temp)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);

    let renamed = written.and_then(|()| fs::rename(&temp, dir.join(name)));
    if renamed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    renamed?;

    sync_dir(dir);
    Ok(())
}

/// Removes the temporary files `.{name}.tmp*` of the folder, which only a crashed write leaves. The service holds a lock
/// on the folder, so no other writer is using one.
fn remove_stale_temps(dir: &Path, name: &str) {
    let prefix = format!(".{name}.tmp");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Takes an exclusive lock on the file `name` of `dir`, which is made if it is missing. The lock lasts as long as the
/// returned file is open, and the system lets it go when the process ends. A lock that another holder has is
/// [`Error::AlreadyRunning`].
pub fn lock_dir(dir: &Path, name: &str) -> Result<File> {
    create_private_dir(dir)?;

    let file = private_options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(name))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::AlreadyRunning),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

#[cfg(unix)]
fn private_options() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

#[cfg(not(unix))]
fn private_options() -> OpenOptions {
    OpenOptions::new()
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

/// Flushes the folder entry of a rename so it survives a power cut. A failure only costs that durability, so it is
/// ignored (Windows cannot open a folder as a file at all).
fn sync_dir(dir: &Path) {
    if let Ok(folder) = File::open(dir) {
        let _ = folder.sync_all();
    }
}

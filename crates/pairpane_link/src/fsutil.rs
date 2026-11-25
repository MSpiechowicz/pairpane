//! The two file operations the key file and the device list share: a read that cannot be made to take more memory than
//! a limit, and a write that is all or nothing.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

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

/// Writes `bytes` to `dir/name` so a crash leaves either the old file or the new one: the bytes go to a temporary file
/// of the same folder (created fresh, never through a link that is already there; mode 0600 on Unix), are flushed to
/// disk, and the temporary file is renamed over the target. The folder is created when it is missing.
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    create_private_dir(dir)?;

    let temp = dir.join(format!(".{name}.tmp"));
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

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

//! The private folder `unrar` unpacks into, and the way back out of it. `unrar` writes with its own rules; nothing it wrote
//! is trusted. The files the caller wants are looked up by the paths the index holds, one part at a time without following
//! a link, and only a regular file is opened. The folder is removed, with all that is in it, when the guard is dropped,
//! whether the unpacking was finished, failed, was cancelled or panicked.

use std::collections::hash_map::RandomState;
use std::fs::{self, DirBuilder, File};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};

/// Names tried for the folder before giving up.
const MAX_TRIES: u32 = 100;

/// The folder of one unpacking.
#[derive(Debug)]
pub struct Staging {
    root: PathBuf,
}

impl Staging {
    /// Creates a new, empty folder in `parent` that only its owner may enter (on Unix). Its name is random, so it is not
    /// guessed, and creating it fails when anything is there, so nothing is taken over.
    pub fn create(parent: &Path) -> io::Result<Self> {
        for _ in 0..MAX_TRIES {
            // A random state is seeded from the system, differently every time.
            let nonce = RandomState::new().build_hasher().finish();
            let root = parent.join(format!(".pairpane-rar-{:x}-{nonce:016x}", std::process::id()));

            match private_folder(&root) {
                Ok(()) => return Ok(Self { root }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::from(io::ErrorKind::AlreadyExists))
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Opens the regular file at `relative` (a normalized archive path, `a/b/c`). `None` when there is nothing there, or
    /// when a part of the way is a link or is not a folder, or the file itself is a link or anything but a regular file.
    pub fn open(&self, relative: &str) -> Option<File> {
        let mut path = self.root.clone();
        let mut parts = relative.split('/').peekable();

        while let Some(part) = parts.next() {
            path.push(part);
            let kind = fs::symlink_metadata(&path).ok()?.file_type();

            if parts.peek().is_some() {
                // The way to the file is folders, never a link.
                if !kind.is_dir() {
                    return None;
                }
            } else if !kind.is_file() {
                return None;
            }
        }

        File::open(path).ok()
    }

    /// Removes the file at `relative` once it is used, so a big archive does not hold its whole size on the disk.
    pub fn discard(&self, relative: &str) {
        let _ = fs::remove_file(self.root.join(relative));
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // `remove_dir_all` does not follow a link.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn private_folder(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        builder.mode(0o700);
    }

    builder.create(path)
}

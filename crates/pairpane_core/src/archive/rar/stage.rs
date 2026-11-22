//! The private folder `unrar` unpacks into, and the way back out of it. `unrar` writes with its own rules; nothing it wrote
//! is trusted. The files the caller wants are looked up by the paths the index holds, one part at a time without following
//! a link, and only a regular file is opened. The folder is removed, with all that is in it, when the guard is dropped,
//! whether the unpacking was finished, failed, was cancelled or panicked.

use std::collections::hash_map::RandomState;
use std::fs::{self, DirBuilder, File};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Names tried for the folder before giving up.
const MAX_TRIES: u32 = 100;

/// The start of the name of every staging folder (and the sweep removes the ones that are left).
const PREFIX: &str = ".pairpane-rar-";

/// How old a folder of another process must be for [`sweep`] to remove it where the system cannot tell whether that
/// process runs: longer than an unpacking may run (see `run::EXTRACT_DEADLINE`, six hours), so the folder of a process
/// that is still working is never taken. A running unpacking also touches its folder (see [`touch`]), so its age is the
/// time since its last sign of life.
const STALE_AFTER: Duration = Duration::from_secs(7 * 60 * 60);

/// A file made and removed in a staging folder to change the folder's modification time.
const PULSE: &str = ".pairpane-alive";

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

/// Removes the staging folders that an earlier run left in the temporary folder (a crash or a kill skips the guard's
/// drop), at start-up: folders named `.pairpane-rar-…` that are real folders (never a link), on Unix are ours, and belong
/// to a process that is gone. Where the system tells whether a process runs (Linux) the folder of a running one is never
/// taken, however old; elsewhere a folder is taken when it has not been touched for [`STALE_AFTER`] (longer than an
/// unpacking may run, and a running one touches its folder, so the folder of another running instance is not taken).
/// Run off the UI's way.
pub fn sweep() {
    sweep_in(&std::env::temp_dir(), SystemTime::now(), false);
}

/// [`sweep`] on exit: the folders of this process go too, whatever is still running in it is being stopped.
pub fn sweep_on_exit() {
    sweep_in(&std::env::temp_dir(), SystemTime::now(), true);
}

/// The sweep of the folder `temp` at the time `now`; `include_own` says whether the folders of this process are removed.
pub(super) fn sweep_in(temp: &Path, now: SystemTime, include_own: bool) {
    let Ok(listing) = fs::read_dir(temp) else {
        return;
    };
    let Some(owner) = owner(temp) else {
        return;
    };

    for entry in listing.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(pid) = name.strip_prefix(PREFIX).and_then(owner_pid) else {
            continue;
        };

        let Ok(seen) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let stale = seen
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_AFTER);
        let removable = if pid == std::process::id() {
            // This process runs; only its own exit takes its folders.
            include_own
        } else {
            match running(pid) {
                Some(running) => !running,
                None => stale,
            }
        };

        if seen.is_dir() && is_ours(&seen, owner) && removable {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The process id in the rest of a staging folder's name (`<pid hex>-<nonce>`).
fn owner_pid(rest: &str) -> Option<u32> {
    u32::from_str_radix(rest.split_once('-')?.0, 16).ok()
}

/// Whether the process `pid` runs; `None` where the system does not tell.
#[cfg(target_os = "linux")]
fn running(pid: u32) -> Option<bool> {
    Some(Path::new("/proc").join(pid.to_string()).exists())
}

#[cfg(not(target_os = "linux"))]
fn running(_pid: u32) -> Option<bool> {
    None
}

/// Changes the modification time of the staging folder `root` to now, by making and removing a file in it, so the sweep
/// of another instance sees a folder that is in use as young. Best effort: nothing is reported. A file that is there
/// already (an entry of the archive that has the name) is left alone.
pub(super) fn touch(root: &Path) {
    let pulse = root.join(PULSE);

    if File::options().write(true).create_new(true).open(&pulse).is_ok() {
        let _ = fs::remove_file(&pulse);
    }
}

/// Who we are, as the owner of a folder we make: the system tells no other way without `unsafe`. `None` where the
/// temporary folder cannot be written (nothing is then taken for ours).
#[cfg(unix)]
fn owner(temp: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    let probe = Staging::create(temp).ok()?;

    fs::symlink_metadata(probe.path()).ok().map(|seen| seen.uid())
}

#[cfg(unix)]
fn is_ours(seen: &fs::Metadata, owner: u32) -> bool {
    use std::os::unix::fs::MetadataExt;

    seen.uid() == owner
}

/// The temporary folder of a Windows user is that user's own.
#[cfg(not(unix))]
fn owner(_temp: &Path) -> Option<u32> {
    Some(0)
}

#[cfg(not(unix))]
fn is_ours(_seen: &fs::Metadata, _owner: u32) -> bool {
    true
}

fn private_folder(path: &Path) -> io::Result<()> {
    #[cfg_attr(not(unix), expect(unused_mut, reason = "only Unix sets a mode on the builder"))]
    let mut builder = DirBuilder::new();

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        builder.mode(0o700);
    }

    builder.create(path)
}

//! Temporary files that clean up after themselves: the name of one, and the guard that removes it unless it was
//! committed. A file is written under a temporary name next to its target and renamed into place when whole.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::report::refusal;
use super::transfer::{Run, Stop};

/// Names tried for a temporary file before giving up.
const MAX_TEMP_TRIES: u32 = 100;

/// What tells this run of the app from every other in the names of its temporary files, so a leftover of an earlier
/// run (a crash leaves its `.part` files behind, and job numbers start at 1 again) never has the same name.
pub(super) fn run_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();

    TOKEN.get_or_init(|| {
        // A random state is seeded from the system, differently for every run.
        let nonce = RandomState::new().build_hasher().finish();

        format!("{:x}-{:08x}", std::process::id(), nonce as u32)
    })
}

/// The temporary file of a copy in progress: removed again unless it was committed, whether the copy was
/// cancelled, failed or panicked.
pub(super) struct Partial {
    pub(super) path: PathBuf,
    pub(super) committed: bool,
}

impl Partial {
    /// Guards `path`, which this job has just created and no one else owns.
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path, committed: false }
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Run<'_> {
    /// A name for the temporary file in `dir`, unique to this run of the app and this job.
    pub(super) fn temp_path(&mut self, dir: &Path) -> PathBuf {
        self.sequence += 1;

        dir.join(format!(
            ".pairpane-{}-{}-{}.part",
            run_token(),
            self.job.id.0,
            self.sequence
        ))
    }

    /// Makes the temporary file for a copy in `dir` with `create`, which must create it exclusively. A name that is
    /// taken anyway (by anyone: nothing there is ever opened) is skipped, and the next one tried. Only `create`
    /// reporting the name as taken retries: whatever fails after the file exists is the copy's own failure.
    pub(super) fn make_temp<T>(
        &mut self,
        dir: &Path,
        mut create: impl FnMut(&Path) -> io::Result<T>,
    ) -> Result<(PathBuf, T), Stop> {
        for _ in 0..MAX_TEMP_TRIES {
            let temp = self.temp_path(dir);

            match create(&temp) {
                Ok(made) => return Ok((temp, made)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }

        Err(refusal(
            io::ErrorKind::AlreadyExists,
            "no free name was found for a temporary file",
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_partial_file_is_removed_when_it_is_dropped_uncommitted() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("a.part");
        fs::write(&path, "half").expect("file");

        drop(Partial::new(path.clone()));

        assert!(!path.exists());
    }

    #[test]
    fn a_committed_partial_file_stays() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("a.part");
        fs::write(&path, "whole").expect("file");

        let mut guard = Partial::new(path.clone());
        guard.committed = true;
        drop(guard);

        assert!(path.exists());
    }

    #[test]
    fn a_partial_file_is_removed_when_the_copy_panics() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("a.part");
        fs::write(&path, "half").expect("file");
        let guarded = path.clone();

        let result = std::panic::catch_unwind(move || {
            let _guard = Partial::new(guarded);
            panic!("the copy broke");
        });

        assert!(result.is_err());
        assert!(!path.exists());
    }
}

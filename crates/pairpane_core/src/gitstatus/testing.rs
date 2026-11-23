//! The status as the unit tests read it: see [`super::status_in_process`] (and, for the confinement and the process, the
//! helper through the real executable in `src-tauri/tests/git_helper.rs`, and `confine_tests`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::{GitError, Limits, in_process};
use crate::model::GitStatus;

/// The status of `folder`, with the search for a repository ending at the user's home folder as the app's does.
pub(super) fn status(folder: &Path, limits: Limits, cancel: &AtomicBool) -> Result<GitStatus, GitError> {
    super::status_in_process(folder, limits, cancel)
}

/// Like [`status`], with the folder the search for a repository must not go above given (`None`: the filesystem root).
pub(super) fn status_below(
    folder: &Path,
    ceiling: Option<PathBuf>,
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<GitStatus, GitError> {
    in_process(folder, ceiling, limits, cancel)
}

/// A temporary folder whose path has no link on it: the system's temporary folder is one on macOS (`/var` is a link to
/// `/private/var`), and the repository's pointers are refused unless they are physical (see `links::is_physical`), as the ones
/// git writes are (it writes real paths for a linked work tree and for `--separate-git-dir`).
pub(super) fn tempdir() -> tempfile::TempDir {
    let base = gix::path::realpath(std::env::temp_dir()).expect("the temporary folder has a real path");

    tempfile::Builder::new().tempdir_in(base).expect("a folder")
}

//! Where `unrar` is. Only places that this code names are looked in: a few fixed absolute folders, then the entries of
//! `PATH` that are absolute (a relative or empty entry would resolve against whatever folder the app runs in). Nothing is
//! asked of a shell, and a candidate must be a regular file (and, on Unix, executable).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::missing;

/// The folders of a fixed install, before `PATH` is looked at.
#[cfg(unix)]
const FIXED_FOLDERS: &[&str] = &["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"];
#[cfg(unix)]
const PROGRAM: &str = "unrar";

#[cfg(windows)]
const FIXED_FOLDERS: &[&str] = &[];
#[cfg(windows)]
const PROGRAM: &str = "UnRAR.exe";

/// The WinRAR install folders, relative to the variables that name where programs are installed.
#[cfg(windows)]
const WINDOWS_VARIABLES: &[&str] = &["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"];

/// The `unrar` of this system, or the error [`missing`] makes.
pub fn find() -> std::io::Result<PathBuf> {
    locate(&fixed_candidates(), std::env::var_os("PATH").as_deref()).ok_or_else(missing)
}

/// The places `unrar` is looked for before `PATH`.
fn fixed_candidates() -> Vec<PathBuf> {
    let folders = FIXED_FOLDERS.iter().map(|folder| Path::new(folder).join(PROGRAM));

    folders.chain(installed()).collect()
}

/// WinRAR's own folders on Windows, where programs are installed.
#[cfg(windows)]
fn installed() -> Vec<PathBuf> {
    WINDOWS_VARIABLES
        .iter()
        .filter_map(std::env::var_os)
        .map(|root| Path::new(&root).join("WinRAR").join(PROGRAM))
        .collect()
}

#[cfg(not(windows))]
fn installed() -> Vec<PathBuf> {
    Vec::new()
}

/// The first of `fixed` that is usable, else the first usable `unrar` in an absolute folder of `path` (a `PATH` value).
pub(super) fn locate(fixed: &[PathBuf], path: Option<&OsStr>) -> Option<PathBuf> {
    let from_path: Vec<PathBuf> = path
        .map(|value| std::env::split_paths(&OsString::from(value)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(PROGRAM))
        .collect();

    fixed
        .iter()
        .chain(&from_path)
        .find(|candidate| candidate.is_absolute() && usable(candidate))
        .cloned()
}

/// Whether `path` is a regular file that can be run.
fn usable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };

    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    true
}

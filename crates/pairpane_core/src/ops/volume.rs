//! Which volume a destination is on, so jobs for one volume queue behind each other and jobs for different
//! volumes run side by side.
//!
//! The key only schedules; it never decides what is safe, because a move always tries a plain rename first and a
//! copy checks every target itself. A wrong key costs speed (two jobs to one disk at once), nothing else.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::listing::platform;

/// Identifies a volume for scheduling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum VolumeKey {
    /// A Unix device number.
    #[cfg_attr(not(unix), allow(dead_code, reason = "made on Unix only"))]
    Device(u64),
    /// A Windows drive or mounted volume, such as `C:` or `VOLUME{GUID}`.
    #[cfg_attr(not(windows), allow(dead_code, reason = "made on Windows only"))]
    Named(String),
    /// The target could not be looked at; the job runs and fails with its own error.
    Unknown,
}

/// Finds the key of the volume that holds, or would hold, the folder `target`.
pub(super) fn key_of(target: &Path) -> VolumeKey {
    let Ok(Some((existing, _))) = existing_part(target) else {
        return VolumeKey::Unknown;
    };

    key_of_existing(&existing)
}

#[cfg(unix)]
fn key_of_existing(existing: &Path) -> VolumeKey {
    fs::metadata(existing)
        .ok()
        .and_then(|metadata| platform::device(&metadata))
        .map_or(VolumeKey::Unknown, VolumeKey::Device)
}

#[cfg(not(unix))]
fn key_of_existing(existing: &Path) -> VolumeKey {
    existing
        .to_str()
        .and_then(prefix_key)
        .map_or(VolumeKey::Unknown, VolumeKey::Named)
}

/// Splits `path` into the deepest part that exists, resolved the way the platform rules say (on Windows every
/// link on it is checked, see [`platform::local_path`]), and the names below it that do not exist yet. `None`
/// when not even the root exists.
///
/// # Errors
///
/// A path that must not be touched (on Windows: a link on it leads to a network or device path), or a part
/// that cannot be read.
pub(super) fn existing_part(path: &Path) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let mut missing: Vec<&std::ffi::OsStr> = Vec::new();
    let mut current = path;

    loop {
        if let Some(resolved) = exists(current)? {
            return Ok(Some((resolved, missing.iter().rev().collect())));
        }

        let (Some(parent), Some(name)) = (current.parent(), current.file_name()) else {
            return Ok(None);
        };
        missing.push(name);
        current = parent;
    }
}

/// `path` resolved when it exists; `None` when it, or a folder on the way, is missing.
fn exists(path: &Path) -> io::Result<Option<PathBuf>> {
    let found = if cfg!(windows) {
        platform::local_path(path)
    } else {
        fs::metadata(path).map(|_| path.to_owned())
    };

    match found {
        Ok(resolved) => Ok(Some(resolved)),
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => Ok(None),
        Err(error) => Err(error),
    }
}

/// The volume part of a Windows path: `C:` for `C:\x` and `\\?\C:\x`, `VOLUME{GUID}` for `\\?\Volume{GUID}\x`
/// (upper case, so one volume has one spelling). `None` for any other path. A pure check on the text, so it is
/// tested on every system, and used on Windows only.
#[cfg_attr(not(windows), allow(dead_code, reason = "used on Windows; tested everywhere"))]
pub(super) fn prefix_key(path: &str) -> Option<String> {
    let rest = path.strip_prefix(r"\\?\").unwrap_or(path);
    let first = rest.split('\\').next().unwrap_or_default();

    if first.get(..7).is_some_and(|head| head.eq_ignore_ascii_case("Volume{")) {
        return first.ends_with('}').then(|| first.to_ascii_uppercase());
    }

    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => Some(format!("{}:", letter.to_ascii_uppercase())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn windows_paths_name_their_volume_in_one_spelling() {
        let guid = "26a21bda-a627-11d7-9931-806e6f6e6963";

        assert_eq!(prefix_key(r"C:\x\y").as_deref(), Some("C:"));
        assert_eq!(prefix_key(r"c:\x").as_deref(), Some("C:"));
        assert_eq!(prefix_key(r"\\?\C:\x").as_deref(), Some("C:"));
        assert_eq!(
            prefix_key(&format!(r"\\?\Volume{{{guid}}}\x")).as_deref(),
            Some(format!("VOLUME{{{}}}", guid.to_ascii_uppercase()).as_str())
        );
        assert_eq!(
            prefix_key(&format!(r"\\?\volume{{{guid}}}")).as_deref(),
            Some(format!("VOLUME{{{}}}", guid.to_ascii_uppercase()).as_str())
        );
    }

    #[test]
    fn other_windows_paths_have_no_volume() {
        for other in [
            r"\\host\share\x",
            r"\\?\UNC\h\s",
            r"\\.\pipe\x",
            r"\x",
            "",
            r"1:\x",
            r"\\?\Volume{open",
        ] {
            assert_eq!(prefix_key(other), None, "{other}");
        }
    }

    #[test]
    fn the_existing_part_stops_at_the_first_missing_folder() {
        let temp = TempDir::new().expect("temp dir");
        let target = temp.path().join("a").join("b").join("c");

        let (existing, missing) = existing_part(&target).expect("resolves").expect("the temp dir exists");

        assert_eq!(existing, platform::local_path(temp.path()).expect("local"));
        assert_eq!(missing, Path::new("a").join("b").join("c"));
    }

    #[test]
    fn a_folder_has_the_key_of_the_volume_it_would_be_created_on() {
        let temp = TempDir::new().expect("temp dir");

        let existing = key_of(temp.path());
        let missing = key_of(&temp.path().join("not").join("there"));

        assert_ne!(existing, VolumeKey::Unknown);
        assert_eq!(existing, missing);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_below_a_file_is_on_the_files_volume() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("file");
        fs::write(&file, b"x").expect("file");

        // `file` exists, so it is the existing part; the key is still its device.
        assert_eq!(key_of(&file.join("below")), key_of(temp.path()));
    }
}

//! Platform rules for listings, all in one place: which paths a pane may open, what counts as hidden, and how
//! names sort.

use std::cmp::Ordering;
use std::fs::Metadata;
use std::path::{Component, Path, Prefix};

/// Longest path a pane opens, in bytes.
pub const MAX_PATH_BYTES: usize = 4_096;

/// Checks a folder path from the UI: absolute, no `.`/`..`, not too long, and on a local drive. Network
/// (`\\host\share`) and device (`\\.\pipe`) paths are refused for now: touching one makes Windows connect to that host,
/// which a page must not be able to trigger. Network places get their own, explicit way in later.
pub fn check_folder(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("the path must be absolute");
    }
    if path.as_os_str().len() > MAX_PATH_BYTES {
        return Err("the path is too long");
    }

    for component in path.components() {
        match component {
            Component::CurDir | Component::ParentDir => return Err("the path must not contain `.` or `..`"),
            Component::Prefix(prefix) if !is_local(prefix.kind()) => {
                return Err("network and device paths cannot be opened here");
            }
            _ => {}
        }
    }

    Ok(())
}

/// A file name to open inside a folder: one plain name, nothing that climbs, jumps or (on Windows) names a
/// stream or another drive.
pub fn is_plain_name(name: &str) -> bool {
    let unix_ok = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0']);
    let windows_ok = !cfg!(windows) || !(name.contains(['\\', ':']) || name.ends_with(['.', ' ']));

    unix_ok && windows_ok
}

/// Whether listing may follow a symbolic link to `target`: not to a network or device path (Windows would
/// connect to that host just because the folder holding the link was shown).
pub fn may_follow(target: &Path) -> bool {
    !matches!(target.components().next(), Some(Component::Prefix(prefix)) if !is_local(prefix.kind()))
}

/// A drive letter path (`C:\`, `\\?\C:\`); only Windows paths have prefixes.
fn is_local(prefix: Prefix<'_>) -> bool {
    matches!(prefix, Prefix::Disk(_) | Prefix::VerbatimDisk(_))
}

/// Whether names sort ignoring case: on macOS and Windows (whose filesystems usually ignore case), not on Linux.
pub const CASE_INSENSITIVE: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Hidden on Unix: the name starts with a dot.
#[cfg(not(windows))]
pub fn is_hidden(name: &str, _metadata: Option<&Metadata>) -> bool {
    name.starts_with('.')
}

/// Hidden on Windows: the hidden attribute is set.
#[cfg(windows)]
pub fn is_hidden(_name: &str, metadata: Option<&Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    metadata.is_some_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

/// Whether hiding needs the metadata (Windows attributes) or only the name (Unix).
pub const HIDDEN_NEEDS_METADATA: bool = cfg!(windows);

/// The key names sort by: lowercased where sorting ignores case, else empty (the name itself is the key).
pub fn sort_key(name: &str) -> String {
    if CASE_INSENSITIVE {
        name.to_lowercase()
    } else {
        String::new()
    }
}

/// Orders two names by their [`sort_key`]s, falling back to the exact name so the order is total and stable.
pub fn compare(a_key: &str, a_name: &str, b_key: &str, b_name: &str) -> Ordering {
    a_key.cmp(b_key).then_with(|| a_name.cmp(b_name))
}

/// Unix permission bits, or `None` on Windows.
#[cfg(unix)]
pub fn mode(metadata: &Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    Some(metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
pub fn mode(_metadata: &Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut keyed: Vec<(String, &str)> = names.iter().map(|name| (sort_key(name), *name)).collect();
        keyed.sort_by(|(a_key, a), (b_key, b)| compare(a_key, a, b_key, b));
        keyed.into_iter().map(|(_, name)| name.to_owned()).collect()
    }

    #[test]
    fn folder_paths_must_be_plain_absolute_and_local() {
        let root = if cfg!(windows) { "C:\\data" } else { "/data" };

        assert_eq!(check_folder(Path::new(root)), Ok(()));
        assert!(check_folder(Path::new("relative")).is_err());
        assert!(check_folder(&Path::new(root).join("..").join("x")).is_err());
        assert!(check_folder(&Path::new(root).join("a".repeat(MAX_PATH_BYTES))).is_err());

        #[cfg(windows)]
        for remote in [
            r"\\attacker\x",
            "//attacker/x",
            r"\\?\UNC\a\b",
            r"\\.\pipe\x",
            r"\\?\GLOBALROOT\x",
        ] {
            assert!(check_folder(Path::new(remote)).is_err(), "{remote}");
            assert!(!may_follow(Path::new(remote)), "{remote}");
        }
        #[cfg(windows)]
        assert_eq!(check_folder(Path::new(r"\\?\C:\data")), Ok(()));
    }

    #[test]
    fn child_names_are_single_plain_names() {
        for good in ["photos", "a.b", ".config", "..."] {
            assert!(is_plain_name(good) || cfg!(windows), "{good}");
        }
        for bad in ["", ".", "..", "a/b", "a/", "/etc", "nul\0"] {
            assert!(!is_plain_name(bad), "{bad:?}");
        }
        #[cfg(windows)]
        for bad in ["a\\b", "C:", "x::$INDEX_ALLOCATION", "trailing.", "trailing ", "..."] {
            assert!(!is_plain_name(bad), "{bad}");
        }
    }

    #[test]
    fn sorts_by_the_platform_case_rule() {
        let names = sorted(&["b", "B", "a", "A"]);

        if CASE_INSENSITIVE {
            assert_eq!(names, ["A", "a", "B", "b"]);
        } else {
            assert_eq!(names, ["A", "B", "a", "b"]);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn dot_files_are_hidden_on_unix() {
        assert!(is_hidden(".git", None));
        assert!(!is_hidden("git", None));
        assert!(!is_hidden("a.b", None));
    }
}

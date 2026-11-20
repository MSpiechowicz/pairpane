//! Platform rules for listings, all in one place: which paths a pane may open, what counts as hidden, and how
//! names sort.

use std::cmp::Ordering;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

mod resolve;

pub use resolve::{local_target, resolve_local};

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
            Component::Normal(name) if is_unsafe_name(&name.to_string_lossy()) => {
                return Err("this path has a name that cannot be opened here");
            }
            _ => {}
        }
    }

    Ok(())
}

/// A file name to open inside a folder: one plain name, nothing that climbs, jumps or (on Windows) names a
/// stream, another drive or a device.
pub fn is_plain_name(name: &str) -> bool {
    let unix_ok = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0']);
    let windows_ok = !cfg!(windows) || !(name.contains('\\') || is_special_windows_name(name));

    unix_ok && windows_ok
}

/// Whether a name must not be opened or followed on this platform: on Windows one that [`is_special_windows_name`].
/// Used for every component of a path and every listed entry.
pub fn is_unsafe_name(name: &str) -> bool {
    cfg!(windows) && is_special_windows_name(name)
}

/// Whether Windows reads `name` as something other than the file it spells: a device, a stream (`:`), a name it
/// rewrites by dropping trailing spaces and dots (`x ` is opened as `x`), or one holding a separator or another
/// character Windows forbids in names (inside a `\\?\` path `//host/share` is one component, and joining it to a path
/// would turn the path into a network one). Such a name would slip past a check of the component as spelled, so every
/// one is refused. A pure check, testable on every system; only Windows applies it (see [`is_unsafe_name`]).
pub fn is_special_windows_name(name: &str) -> bool {
    let forbidden = |c: char| c.is_ascii_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*');

    name.contains(forbidden) || name.ends_with(['.', ' ']) || is_reserved_windows_name(name)
}

/// Whether Windows treats `name` as a device, not a file: `CON`, `PRN`, `AUX`, `NUL`, `COM0`-`COM9`, `LPT0`-`LPT9`
/// (the digit also as a superscript `¹²³`), `CONIN$` and `CONOUT$`, in any case, with anything after the first `.` or
/// `:` (`NUL.txt`, `COM1:`, `NUL:x`) and with spaces before it (`NUL .txt`). Opening one reaches the device (a
/// printer, a serial port), so it is refused. A pure check, testable on every system; only Windows applies it.
pub fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name.split(['.', ':']).next().unwrap_or_default().trim_end_matches(' ');
    let stem = stem.to_ascii_uppercase();

    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }

    let Some(digit) = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT")) else {
        return false;
    };
    let mut chars = digit.chars();

    match (chars.next(), chars.next()) {
        (Some(digit), None) => digit.is_ascii_digit() || matches!(digit, '¹' | '²' | '³'),
        _ => false,
    }
}

/// Whether listing may follow a symbolic link to `target`: not to a network or device path (Windows would
/// connect to that host just because the folder holding the link was shown).
pub fn may_follow(target: &Path) -> bool {
    !matches!(target.components().next(), Some(Component::Prefix(prefix)) if !is_local(prefix.kind()))
}

/// A local place: a drive letter path (`C:\`, `\\?\C:\`) or a mounted volume (`\\?\Volume{GUID}\`, how Windows
/// shows a folder that is another volume's mount point). Network (`\\host\share`, `\\?\UNC\`) and device
/// (`\\.\pipe`, `\\?\GLOBALROOT`) prefixes are not. Only Windows paths have prefixes.
fn is_local(prefix: Prefix<'_>) -> bool {
    match prefix {
        Prefix::Disk(_) | Prefix::VerbatimDisk(_) => true,
        Prefix::Verbatim(name) => name.to_str().is_some_and(is_volume_guid),
        _ => false,
    }
}

/// `Volume{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`: the volume name Windows gives a mounted volume.
fn is_volume_guid(name: &str) -> bool {
    name.split_at_checked(7).is_some_and(|(head, rest)| {
        head.eq_ignore_ascii_case("Volume{")
            && rest
                .strip_suffix('}')
                .is_some_and(|guid| !guid.is_empty() && guid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    })
}

/// The path to open for `path`. On Windows every link on it is resolved and checked (see [`resolve_local`]); on other
/// systems it is `path` itself, as the system does not connect to a host just by resolving a path.
pub fn local_path(path: &Path) -> io::Result<PathBuf> {
    if cfg!(windows) {
        resolve_local(path)
    } else {
        Ok(path.to_owned())
    }
}

/// The metadata of what the link at `path` leads to, or `None` when it must not be followed (see [`local_path`]) or
/// cannot be read.
pub fn follow(path: &Path) -> Option<Metadata> {
    fs::metadata(local_path(path).ok()?).ok()
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

/// Bytes available to the user on the volume holding `path`; `None` when the platform cannot tell.
pub fn free_space(path: &Path) -> Option<u64> {
    fs4::available_space(path).ok()
}

/// The device (volume) an entry is on, to keep a folder walk on one filesystem. `None` on Windows, where the
/// standard library exposes no volume without `unsafe`; there the walk relies on never following junctions
/// and mount points, which is how other volumes are attached to a folder.
#[cfg(unix)]
pub fn device(metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;

    Some(metadata.dev())
}

#[cfg(not(unix))]
pub fn device(_metadata: &Metadata) -> Option<u64> {
    None
}

/// What identifies a directory on its filesystem (device and inode), to walk each one once even when a bind mount
/// shows it twice. `None` on Windows, which has no mount tricks to the same folder that the walk would follow.
#[cfg(unix)]
pub fn identity(metadata: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
pub fn identity(_metadata: &Metadata) -> Option<(u64, u64)> {
    None
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
pub(crate) mod link_tests;

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
    fn windows_device_names_are_recognised_in_any_spelling() {
        for device in [
            "CON",
            "con",
            "Prn",
            "AUX",
            "nUl",
            "NUL.txt",
            "nul.tar.gz",
            "NUL .txt",
            "NUL ",
            "NUL..",
            "COM0",
            "com1",
            "COM9",
            "LPT0",
            "lpt9.log",
            "COM¹",
            "com².x",
            "LPT³",
            "CONIN$",
            "conout$",
            "CONOUT$.txt",
            "COM1:",
            "NUL:x",
            "con:",
            "CON :x",
            "LPT¹:stream",
        ] {
            assert!(is_reserved_windows_name(device), "{device}");
        }
        for file in [
            "", ".", "NULL", "CONSOLE", "COM", "COM10", "COMA", "LPT", "LPT¹²", "xNUL", "NUL-1", ".NUL", "a.NUL",
            "CONIN", "CON$", "photos", "x:CON", "x.CON:",
        ] {
            assert!(!is_reserved_windows_name(file), "{file}");
        }
    }

    #[test]
    fn windows_names_it_rewrites_or_reads_as_something_else_are_special() {
        for special in [
            "x ", "x.", "x..", "...", "..", "a:b", "x::$DATA", "NUL", "com1.txt", "x  ", "//h/s", r"\\h\s", "a/b",
            r"a\b", "a?b", "a*b", "a<b", "a>b", "a|b", "a\"b", "a\u{1}b",
        ] {
            assert!(is_special_windows_name(special), "{special:?}");
        }
        for plain in ["x", ".x", "a b", "a.b", "photos", ".config", "NULL"] {
            assert!(!is_special_windows_name(plain), "{plain:?}");
        }

        // Only Windows applies the rule.
        assert_eq!(is_unsafe_name("x "), cfg!(windows));
    }

    #[cfg(windows)]
    #[test]
    fn network_and_device_prefixes_are_not_local_but_volumes_and_drives_are() {
        let volume = r"\\?\Volume{26a21bda-a627-11d7-9931-806e6f6e6963}\mount\x";
        for local in [
            r"C:\x",
            r"\\?\C:\x",
            volume,
            r"\\?\volume{26A21BDA-a627-11d7-9931-806e6f6e6963}\",
        ] {
            assert!(may_follow(Path::new(local)), "{local}");
            assert_eq!(check_folder(Path::new(local)), Ok(()), "{local}");
        }
        for remote in [
            r"\\attacker\x",
            r"\\?\UNC\a\b",
            r"\\.\pipe\x",
            r"\\?\GLOBALROOT\x",
            r"\\?\Volume\x",
            r"\\?\Volume{}\x",
            r"\\?\Volume{zz}\x",
            r"\\?\Volume{26a21bda}x\",
        ] {
            assert!(!may_follow(Path::new(remote)), "{remote}");
            assert!(check_folder(Path::new(remote)).is_err(), "{remote}");
        }
        // In a `\\?\` path only `\` separates: `//attacker/share` is one component, refused before anything is opened.
        for smuggled in [r"\\?\C:\//attacker/share", r"\\?\C:\a/b", r"\\?\C:\x\//h/s\y"] {
            assert!(check_folder(Path::new(smuggled)).is_err(), "{smuggled}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_refuses_device_names_as_children_and_in_paths() {
        for device in [
            "NUL", "con.txt", "COM1", "LPT¹", "CONOUT$", "aux ", "COM1:", "x ", "x.", "x. ",
        ] {
            assert!(!is_plain_name(device), "{device}");
            assert!(check_folder(&Path::new(r"C:\data").join(device)).is_err(), "{device}");
            assert!(
                check_folder(&Path::new(r"C:\data").join(device).join("x")).is_err(),
                "{device}"
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn device_names_are_ordinary_names_off_windows() {
        assert!(is_plain_name("NUL"));
        assert_eq!(check_folder(Path::new("/data/NUL")), Ok(()));
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

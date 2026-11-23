//! Where the files of a repository may point to. A repository's own files name other places (the `gitdir:` line of a `.git`
//! file, a `commondir` file, the lines of `objects/info/alternates`), and they are read before anything of the repository
//! is trusted. On Windows touching `\\host\share` makes the system connect to that host and hand it the user's
//! credentials, and on macOS a path under `/net` mounts a remote share on first use; so a pointer that does not lead to a
//! local place makes the folder "not in a repository", exactly as the listing refuses to follow a link there
//! ([`crate::listing::platform::may_follow`]).

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::listing::platform;

/// The place the pointer `text` leads to, read from the folder `base` (an absolute path), or `None` when `text` is not a
/// pointer to a local place: it is empty, holds a NUL, is a network or device path (`\\host\share`, `//host/share`,
/// `\\?\UNC\host\share`, `\\.\pipe\x`), is under an automounted network folder, or (on Windows) leads through a link to
/// one of those. A relative pointer is joined to `base` and is not otherwise changed: the system resolves a `..` as it
/// resolves the path the repository is read through.
pub(super) fn pointer_target(base: &Path, text: &str) -> Option<PathBuf> {
    let text = text.trim_end_matches(['\r', '\n']);
    if text.trim().is_empty() || text.contains('\0') || looks_like_network_path(text) {
        return None;
    }

    let path = Path::new(text);
    let has_prefix = matches!(path.components().next(), Some(Component::Prefix(_)));
    let target = if path.is_absolute() || has_prefix {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    is_local(&target).then_some(target)
}

/// Whether `place` (a path the program is about to touch) is a local one; see [`pointer_target`].
pub(super) fn is_local(place: &Path) -> bool {
    if !platform::may_follow(place) || is_automount(place) {
        return false;
    }

    // On Windows every link on the way is read and checked; a place that is not there yet is not a network one.
    match platform::local_path(place) {
        Ok(_) => true,
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

/// A text that starts like a network or device path in either separator (`\\host\share`, `//host/share`, `\\.\pipe\x`, a
/// `\\?\` path that is not for a drive or a volume: `\\?\UNC\host\share`, `\\?\GLOBALROOT\...`): no local pointer is
/// written that way, and on Unix such a text is an ordinary path, which is refused too, so that the answer does not
/// depend on the system a repository was made on. A `\\?\C:\...` path is a local one, and the system's own rules decide.
fn looks_like_network_path(text: &str) -> bool {
    let alike: Vec<char> = text.chars().take(4).map(|c| if c == '/' { '\\' } else { c }).collect();
    let separator = |c: Option<&char>| c == Some(&'\\');

    if alike == ['\\', '\\', '?', '\\'] {
        let rest = text.get(4..).unwrap_or_default();

        return !(is_drive(rest) || is_volume(rest));
    }

    separator(alike.first()) && separator(alike.get(1))
}

/// `C:` at the start of `text`, followed by nothing or a separator.
fn is_drive(text: &str) -> bool {
    let mut chars = text.chars();

    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(letter), Some(':'), None | Some('\\' | '/')) if letter.is_ascii_alphabetic()
    )
}

/// `Volume{...}` at the start of `text`: how Windows names a mounted volume.
fn is_volume(text: &str) -> bool {
    text.get(..7).is_some_and(|head| head.eq_ignore_ascii_case("Volume{"))
}

/// A path under a folder the system mounts a network share into on first use (macOS `/net` and `/Network`, also as seen
/// through the data volume). Never a place on Windows, where the same names are ordinary folders.
fn is_automount(place: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }

    let mut names = place.components().filter_map(|component| match component {
        Component::Normal(name) => name.to_str(),
        _ => None,
    });
    let first = names.next();

    match first {
        Some("net" | "Network") => true,
        Some("System") => {
            let rest: Vec<_> = names.take(3).collect();

            matches!(rest.as_slice(), ["Volumes", "Data", _]) && matches!(rest.last(), Some(&("net" | "Network")))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\work\project")
        } else {
            PathBuf::from("/work/project")
        }
    }

    #[test]
    fn network_and_device_paths_in_any_spelling_are_refused() {
        for text in [
            r"\\host\share",
            r"\\host\share\repo.git",
            "//host/share",
            "//host/share/repo.git",
            r"\\?\UNC\host\share\repo.git",
            r"//?/UNC/host/share/repo.git",
            r"\\.\pipe\git",
            r"\/host/share",
            "gitdir-with-nul\0",
            "",
            "   ",
            "\n",
        ] {
            assert_eq!(pointer_target(&base(), text), None, "{text:?}");
        }
    }

    #[test]
    fn only_a_verbatim_path_for_a_drive_or_a_volume_is_not_a_network_path() {
        for text in [
            r"\\?\C:\repos\a.git",
            r"\\?\d:",
            "//?/C:/repos",
            r"\\?\Volume{0123-4567}\repos",
        ] {
            assert!(!looks_like_network_path(text), "{text}");
        }
        for text in [
            r"\\?\UNC\h\s",
            r"\\?\GLOBALROOT\Device\x",
            r"\\?\",
            r"\\?\CC:\x",
            r"\\.\C:\x",
            "//h/s",
            r"\\h\s",
        ] {
            assert!(looks_like_network_path(text), "{text}");
        }
        for text in ["/srv/repo.git", "../x", "C:\\repos", "relative", "a//b", ""] {
            assert!(!looks_like_network_path(text), "{text}");
        }
    }

    #[test]
    fn a_relative_pointer_is_read_from_the_folder_that_holds_it() {
        let base = base();

        assert_eq!(pointer_target(&base, "../x"), Some(base.join("../x")));
        assert_eq!(pointer_target(&base, "worktrees/a\r\n"), Some(base.join("worktrees/a")));
        assert_eq!(
            pointer_target(&base, "../../.git/modules/m\n"),
            Some(base.join("../../.git/modules/m"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn an_absolute_local_pointer_is_taken_as_it_is_and_a_backslash_name_is_one_folder_name() {
        let base = base();

        assert_eq!(
            pointer_target(&base, "/srv/repo.git"),
            Some(PathBuf::from("/srv/repo.git"))
        );
        // Not a network path here: one name that holds backslashes, inside the folder.
        assert_eq!(pointer_target(&base, r"host\share"), Some(base.join(r"host\share")));
    }

    #[cfg(not(windows))]
    #[test]
    fn a_path_under_an_automounted_network_folder_is_refused() {
        for text in [
            "/net/host/export",
            "/Network/Servers/host",
            "/System/Volumes/Data/net/host",
            "/System/Volumes/Data/Network/Servers",
        ] {
            assert_eq!(pointer_target(&base(), text), None, "{text}");
        }
        for text in [
            "/network/x",
            "/netx/y",
            "/System/Volumes/Data/home",
            "/System/Library",
            "/srv/net/x",
        ] {
            assert!(pointer_target(&base(), text).is_some(), "{text}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_relative_or_device_prefix_is_refused_on_windows() {
        for text in [r"\\?\GLOBALROOT\Device\x", r"\\.\C:\x", r"//./pipe/x", "C:relative"] {
            assert_eq!(pointer_target(&base(), text), None, "{text}");
        }

        assert!(pointer_target(&base(), r"D:\repos\a.git").is_some());
        assert!(pointer_target(&base(), r"\\?\D:\repos\a.git").is_some());
    }
}

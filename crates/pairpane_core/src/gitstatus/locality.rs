//! Where the files of a repository may point to. A repository's own files name other places (the `gitdir:` line of a `.git`
//! file, a `commondir` file, the lines of `objects/info/alternates`), and they are read before anything of the repository
//! is trusted. On Windows touching `\\host\share` makes the system connect to that host and hand it the user's
//! credentials, and on macOS a path under `/net` mounts a remote share on first use; so a pointer that does not lead to a
//! local place makes the folder "not in a repository", exactly as the listing refuses to follow a link there
//! ([`crate::listing::platform::may_follow`]).

use std::path::{Path, PathBuf};

use super::links::{is_local, is_physical};

/// The place the pointer `pointer` leads to, read from the folder `base` (an absolute path), or `None` when it is not a
/// pointer to a local place that has no link on it: it is empty, holds a NUL, is a network or device path (`\\host\share`,
/// `//host/share`, `\\?\UNC\host\share`, `\\.\pipe\x`), is under an automounted network folder, leads through a link to
/// one of those (see [`is_local`]), or has a link anywhere on it (see [`is_physical`]: gix takes the `..` of a pointer out by
/// reading it, and a link before a `..` would lead the system elsewhere than gix goes). `pointer` is what the file said, as
/// the parser of gix gave it (a `gitdir:` file and the list of alternates are parsed by gix's own functions, so that what is
/// checked here is what gix opens). A relative pointer is joined to `base` as gix joins it and is not otherwise changed.
pub(super) fn pointer_target(base: &Path, pointer: &Path) -> Option<PathBuf> {
    let text = pointer.to_string_lossy();
    if text.is_empty() || text.contains('\0') || looks_like_network_path(&text) {
        return None;
    }

    let target = base.join(pointer);

    (is_local(&target) && is_physical(&target)).then_some(target)
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
        ] {
            assert_eq!(pointer_target(&base(), Path::new(text)), None, "{text:?}");
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
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        let base = root.join("work/project");
        std::fs::create_dir_all(&base).expect("folders");
        std::fs::create_dir_all(root.join(".git/modules")).expect("folders");

        assert_eq!(pointer_target(&base, Path::new("../x")), Some(base.join("../x")));
        // A submodule's own place: `..` after real folders is fine.
        assert_eq!(
            pointer_target(&base, Path::new("../../.git/modules/m")),
            Some(base.join("../../.git/modules/m"))
        );
        // Nothing is trimmed here: the text is what gix's parser gave, which cut the end of a line and nothing else.
        assert_eq!(pointer_target(&base, Path::new(" a")), Some(base.join(" a")));
    }

    #[cfg(unix)]
    #[test]
    fn a_pointer_with_a_link_on_its_way_is_refused_whatever_it_leads_to() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        std::fs::create_dir_all(root.join("x/y")).expect("folders");
        std::os::unix::fs::symlink("x/y", root.join("l")).expect("a link");
        std::os::unix::fs::symlink("gone", root.join("dangling")).expect("a link");

        for pointer in ["l", "l/../real", "dangling/repo.git", "x/../l"] {
            assert_eq!(pointer_target(&root, Path::new(pointer)), None, "{pointer}");
        }
        assert!(pointer_target(&root, Path::new("x/y/../y")).is_some());
    }

    #[cfg(not(windows))]
    #[test]
    fn an_absolute_local_pointer_is_taken_as_it_is_and_a_backslash_name_is_one_folder_name() {
        let base = base();

        assert_eq!(
            pointer_target(&base, Path::new("/srv/repo.git")),
            Some(PathBuf::from("/srv/repo.git"))
        );
        // Not a network path here: one name that holds backslashes, inside the folder.
        assert_eq!(
            pointer_target(&base, Path::new(r"host\share")),
            Some(base.join(r"host\share"))
        );
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
            assert_eq!(pointer_target(&base(), Path::new(text)), None, "{text}");
        }
        for text in [
            "/network/x",
            "/netx/y",
            "/System/Volumes/Data/home",
            "/System/Library",
            "/srv/net/x",
        ] {
            assert!(pointer_target(&base(), Path::new(text)).is_some(), "{text}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn a_pointer_that_reaches_an_automounted_folder_through_dots_is_refused() {
        // Read away, the first is `/net/host`; the system goes through `/net` for the second one, and that is what is refused.
        for text in ["/srv/../net/host/export", "/net/../srv", "../../../net/host"] {
            let base = PathBuf::from("/work/project");

            assert_eq!(pointer_target(&base, Path::new(text)), None, "{text}");
        }
        assert!(pointer_target(Path::new("/work/project"), Path::new("/srv/../home")).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_relative_or_device_prefix_is_refused_on_windows() {
        for text in [r"\\?\GLOBALROOT\Device\x", r"\\.\C:\x", r"//./pipe/x", "C:relative"] {
            assert_eq!(pointer_target(&base(), Path::new(text)), None, "{text}");
        }

        assert!(pointer_target(&base(), Path::new(r"D:\repos\a.git")).is_some());
        assert!(pointer_target(&base(), Path::new(r"\\?\D:\repos\a.git")).is_some());
    }
}

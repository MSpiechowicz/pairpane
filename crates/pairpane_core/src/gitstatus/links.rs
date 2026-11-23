//! Looking at a path the way the system will resolve it, before it is opened. A repository names places (a `gitdir:` pointer,
//! a `commondir`, an alternate object folder, a `.git` that is a link), and a path is not what it looks like: a link in the
//! middle leads somewhere else, and a `..` after a link goes up from where the link leads, not from where it stands. So a
//! path is resolved one component at a time with `lstat` and `readlink` only, and every hop is checked against the folders
//! macOS mounts a network share into on first use (see [`is_automount`]) *before* it is followed, and not only the path as it
//! was written. On Windows the same is done by [`platform::resolve_local`], which also knows network and device paths.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::listing::platform;

#[cfg(not(windows))]
use std::ffi::OsString;
#[cfg(not(windows))]
use std::fs;

/// Most links followed while resolving one path, like the Linux kernel's own limit.
#[cfg(not(windows))]
const MAX_LINKS: usize = 40;

/// Most components handled while resolving one path (the links add the components of their targets).
#[cfg(not(windows))]
const MAX_STEPS: usize = 4_096;

/// The path with its `.` and `..` taken out by reading it, with no look at the disk: how a path is compared with another one,
/// and what a name is under after a `..` that does not follow a link. (A `..` at the root stays at the root, and a relative path
/// keeps the `..` it cannot take out.)
pub(super) fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }

    out
}

/// Whether `place` is under a folder the system mounts a network share into on first use (macOS `/net` and `/Network`, also as
/// seen through the data volume), after its `..` are taken out. Never on Windows, where the same names are ordinary folders.
/// A link on the way is not looked at here; see [`resolve`].
pub(super) fn is_automount(place: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }

    let place = normalise(place);
    let mut names = place.components().filter_map(|component| match component {
        Component::Normal(name) => name.to_str(),
        _ => None,
    });

    match names.next() {
        Some("net" | "Network") => true,
        Some("System") => {
            let rest: Vec<_> = names.take(3).collect();

            matches!(rest.as_slice(), ["Volumes", "Data", _]) && matches!(rest.last(), Some(&("net" | "Network")))
        }
        _ => false,
    }
}

/// The path that the absolute `path` finally leads to, with every link on it followed one at a time and every hop checked
/// before it is followed (an automounted network folder, too many links). `InvalidInput` says a hop must not be followed;
/// `NotFound` that a name on the way is not there (the path is not a network place then, but it leads nowhere either).
#[cfg(not(windows))]
pub(super) fn resolve(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(refused());
    }

    let mut todo: Vec<Step> = steps(path).rev().collect();
    let mut resolved = PathBuf::new();
    let mut hops = 0;
    let mut budget = MAX_STEPS;

    while let Some(next) = todo.pop() {
        budget = budget.checked_sub(1).ok_or_else(refused)?;

        match next {
            Step::Root => resolved = PathBuf::from("/"),
            Step::Parent => {
                resolved.pop();
            }
            Step::Name(name) => {
                resolved.push(&name);
                if is_automount(&resolved) {
                    return Err(refused());
                }
                let metadata = match fs::symlink_metadata(&resolved) {
                    Ok(metadata) => metadata,
                    Err(error) => return Err(unreachable_or_automount(error, resolved, todo)),
                };
                if !metadata.file_type().is_symlink() {
                    continue;
                }

                hops += 1;
                if hops > MAX_LINKS {
                    return Err(refused());
                }

                // The target replaces the link: an absolute one starts again at the root, a relative one goes on from the
                // folder that held the link.
                let target = fs::read_link(&resolved)?;
                resolved.pop();
                todo.extend(steps(&target).rev());
            }
        }
    }

    Ok(resolved)
}

/// The error for a name that cannot be looked at: when it is not there, the rest of the path is not touched by anything, but it
/// may still lead into an automounted folder on a machine where that exists (`/System/Volumes/Data/net` is not there on Linux),
/// and so the rest is read as written, and refused when it does.
#[cfg(not(windows))]
fn unreachable_or_automount(error: io::Error, mut at: PathBuf, rest: Vec<Step>) -> io::Error {
    for step in rest.into_iter().rev() {
        match step {
            Step::Root => return error,
            Step::Parent => at.push(".."),
            Step::Name(name) => at.push(name),
        }
    }

    if is_automount(&at) { refused() } else { error }
}

/// See the other `resolve`: on Windows every link on the way is read and checked by [`platform::resolve_local`].
#[cfg(windows)]
pub(super) fn resolve(path: &Path) -> io::Result<PathBuf> {
    platform::resolve_local(path)
}

/// Whether the path leads nowhere (a name on it is not there, or is no folder), which is not an error of a pointer: gix finds
/// nothing there either.
pub(super) fn leads_nowhere(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory)
}

/// Whether `place` is a local one: no hop of it (as written, and as it reads without its `..`) is a network or device path, an
/// automounted folder, or a loop of links. A place that is not there is not a network one.
pub(super) fn is_local(place: &Path) -> bool {
    if !platform::may_follow(place) || is_automount(place) {
        return false;
    }

    let flat = normalise(place);
    let both = [place, flat.as_path()];

    both.iter().all(|path| match resolve(path) {
        Ok(_) => true,
        Err(error) => leads_nowhere(&error),
    })
}

#[cfg(not(windows))]
fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "this path must not be followed")
}

/// One component of a path still to be resolved.
#[cfg(not(windows))]
enum Step {
    Root,
    Parent,
    Name(OsString),
}

#[cfg(not(windows))]
fn steps(path: &Path) -> impl DoubleEndedIterator<Item = Step> {
    path.components().filter_map(|component| match component {
        Component::RootDir => Some(Step::Root),
        Component::ParentDir => Some(Step::Parent),
        Component::Normal(name) => Some(Step::Name(name.to_owned())),
        Component::CurDir | Component::Prefix(_) => None,
    })
}

#[cfg(all(test, not(windows)))]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn dots_are_taken_out_by_reading_the_path() {
        for (text, flat) in [
            ("/a/b/../c", "/a/c"),
            ("/a/./b", "/a/b"),
            ("/../a", "/a"),
            ("/a/../..", "/"),
            ("a/../../b", "../b"),
            ("/net/../srv", "/srv"),
        ] {
            assert_eq!(normalise(Path::new(text)), PathBuf::from(flat), "{text}");
        }
    }

    #[test]
    fn an_automounted_folder_is_found_whatever_dots_lead_to_it() {
        for text in [
            "/net/host/x",
            "/srv/../net/host",
            "/srv/x/../../Network/Servers",
            "/./net",
            "/System/Volumes/Data/net/host",
            "/home/../System/Volumes/Data/Network/x",
        ] {
            assert!(is_automount(Path::new(text)), "{text}");
            assert!(!is_local(Path::new(text)), "{text}");
        }
        for text in [
            "/srv/net/x",
            "/net2",
            "/network",
            "/srv/../home",
            "/System/Volumes/Data/home",
        ] {
            assert!(!is_automount(Path::new(text)), "{text}");
        }
    }

    #[test]
    fn a_hop_to_an_automounted_folder_is_found_even_when_the_dots_after_it_lead_back() {
        // Read lexically `/net/../srv` is `/srv`, but the system goes through `/net` to get to the root again.
        assert!(!is_local(Path::new("/net/../srv")));
    }

    #[test]
    fn a_link_that_leads_to_an_automounted_folder_is_refused_at_any_hop() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        symlink("/net/host/export", root.join("direct")).expect("a link");
        symlink(root.join("direct"), root.join("second")).expect("a link");
        symlink("second", root.join("third")).expect("a link");
        fs::create_dir(root.join("real")).expect("a folder");
        symlink("/System/Volumes/Data/Network", root.join("real/inside")).expect("a link");

        for name in ["direct", "second", "third", "third/deeper", "real/inside/x"] {
            assert!(!is_local(&root.join(name)), "{name}");
            assert_eq!(
                resolve(&root.join(name)).expect_err(name).kind(),
                io::ErrorKind::InvalidInput,
                "{name}"
            );
        }
    }

    #[test]
    fn a_link_to_a_local_place_is_followed_to_where_it_leads() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        fs::create_dir_all(root.join("a/b")).expect("folders");
        fs::write(root.join("a/b/file"), b"x").expect("a file");
        symlink("a/b", root.join("one")).expect("a link");
        symlink(root.join("one"), root.join("two")).expect("a link");
        symlink("..", root.join("a/b/up")).expect("a link");

        assert_eq!(
            resolve(&root.join("two/file")).expect("resolved"),
            root.join("a/b/file")
        );
        // `..` goes up from where the link leads: `one/..` is `a`, and not the folder that holds `one`.
        assert_eq!(resolve(&root.join("one/..")).expect("resolved"), root.join("a"));
        assert_eq!(resolve(&root.join("a/b/up/b")).expect("resolved"), root.join("a/b"));
        assert!(is_local(&root.join("two")));
    }

    #[test]
    fn a_path_that_leads_nowhere_is_local_and_a_loop_of_links_is_not() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(folder.path()).expect("its real path");
        fs::write(root.join("file"), b"x").expect("a file");
        symlink("loop-b", root.join("loop-a")).expect("a link");
        symlink("loop-a", root.join("loop-b")).expect("a link");
        symlink("gone", root.join("dangling")).expect("a link");

        assert_eq!(
            resolve(&root.join("none/deeper")).expect_err("missing").kind(),
            io::ErrorKind::NotFound
        );
        assert!(is_local(&root.join("none/deeper")));
        assert!(
            is_local(&root.join("file/deeper")),
            "a file is no folder: nothing is there"
        );
        assert!(is_local(&root.join("dangling")));
        assert!(!is_local(&root.join("loop-a")));
        assert!(
            !is_local(Path::new("relative/path")),
            "a path that is not absolute cannot be checked"
        );
    }
}

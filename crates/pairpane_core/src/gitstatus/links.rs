//! Looking at a path the way the system will resolve it, before it is opened. A repository names places (a `gitdir:` pointer,
//! a `commondir`, an alternate object folder, a `.git` that is a link), and a path is not what it looks like: a link in the
//! middle leads somewhere else, and a `..` after a link goes up from where the link leads, not from where it stands. So a
//! path is resolved one component at a time with `lstat` and `readlink` only, and every hop is checked against the folders a
//! network share is mounted into on first use (by name, and by asking the folder that is about to be entered which file system
//! it is: see [`automount`]) *before* it is followed, and not only the path as it was written. On Windows the same is done by
//! [`platform::resolve_local`], which also knows network and device paths.

use std::io;
use std::path::{Component, Path, PathBuf};

use super::automount::is_automount;
use crate::listing::platform;

#[cfg(not(windows))]
use super::automount::{is_autofs, is_automount_with};
#[cfg(not(windows))]
use std::ffi::OsString;
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

/// The path that the absolute `path` finally leads to, with every link on it followed one at a time and every hop checked
/// before it is followed (an automounted network folder, too many links). `InvalidInput` says a hop must not be followed;
/// `NotFound` that a name on the way is not there (the path is not a network place then, but it leads nowhere either).
///
/// A path that this walk cannot follow to its end but the system can is refused as well (`InvalidInput`), and is never "led
/// nowhere": that is a magic link, which has no path to walk, such as `/proc/self/fd/3` for a pipe (`pipe:[1234]`), a socket
/// (`socket:[1234]`), an `anon_inode:` or a deleted file (`/tmp/x (deleted)`). The system opens the very object it stands
/// for, which no look at its text can tell, so any such path is odd.
#[cfg(not(windows))]
pub(super) fn resolve(path: &Path) -> io::Result<PathBuf> {
    resolved(path, &mut is_autofs)
}

/// [`resolve`] for the helper, which runs confined: the folders it is allowed to open are the ones of the repository, and the
/// ones above them it may only look at (macOS's sandbox says so), so it cannot ask the file system of each folder it passes
/// whether that is autofs. The names are checked as ever, and the parent has asked of every folder of every path it looked at
/// before the helper was started.
#[cfg(not(windows))]
pub(super) fn resolve_confined(path: &Path) -> io::Result<PathBuf> {
    resolved(path, &mut |_| false)
}

/// [`resolve`] with `is_autofs` as the answer to whether a folder that is about to be entered is an autofs mount point.
#[cfg(not(windows))]
fn resolved(path: &Path, is_autofs: &mut dyn FnMut(&Path) -> bool) -> io::Result<PathBuf> {
    match walk_with(path, cfg!(target_os = "macos"), is_autofs) {
        Err(error) if leads_nowhere(&error) && fs::metadata(path).is_ok() => Err(refused()),
        other => other,
    }
}

/// The walk of [`resolve`] for a system that is macOS or is not (`macos`: see [`is_automount_with`]), with `is_autofs` as the
/// answer to whether a folder that is about to be entered is an autofs mount point. It is asked of every real folder the path goes
/// through, after that folder was looked at and before anything below it is.
#[cfg(not(windows))]
fn walk_with(path: &Path, macos: bool, is_autofs: &mut dyn FnMut(&Path) -> bool) -> io::Result<PathBuf> {
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
                if is_automount_with(&resolved, macos) {
                    return Err(refused());
                }
                let metadata = match fs::symlink_metadata(&resolved) {
                    Ok(metadata) => metadata,
                    Err(error) => return Err(unreachable_or_automount(error, resolved, todo, macos)),
                };

                // A folder that is about to be entered: whatever its name is, it must not be a mount point that mounts a share when
                // something below it is looked up.
                if metadata.is_dir() && is_autofs(&resolved) {
                    return Err(refused());
                }
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
fn unreachable_or_automount(error: io::Error, mut at: PathBuf, rest: Vec<Step>, macos: bool) -> io::Error {
    for step in rest.into_iter().rev() {
        match step {
            Step::Root => return error,
            Step::Parent => at.push(".."),
            Step::Name(name) => at.push(name),
        }
    }

    if is_automount_with(&at, macos) {
        refused()
    } else {
        error
    }
}

/// See the other `resolve`: on Windows every link on the way is read and checked by [`platform::resolve_local`].
#[cfg(windows)]
pub(super) fn resolve(path: &Path) -> io::Result<PathBuf> {
    platform::resolve_local(path)
}

/// See the other `resolve_confined`: on Windows [`resolve`] is all there is.
#[cfg(windows)]
pub(super) fn resolve_confined(path: &Path) -> io::Result<PathBuf> {
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

/// Whether the system reads `path` as it is written once its `.` and `..` are taken out: no link (or, on Windows, junction)
/// is anywhere on it, so what a program that takes the dots out first (gix does, for a `gitdir:` pointer) means is what the
/// system opens, and `resolve(path) == normalise(path)`. A name that is not there has no link in it; a `..` after a name
/// that is not there, or after a file, is one the system cannot follow, and so is not physical. `..` is fine where it
/// follows real folders only (a submodule's `gitdir: ../.git/modules/x`).
pub(super) fn is_physical(path: &Path) -> bool {
    follows_real_folders(path, false)
}

/// Whether a link on `path` may be followed: the same as [`is_physical`] but for the links, which are allowed, so that the
/// folder that `resolve` leads to is the place (a repository made with `git clone --shared` through a linked home folder names
/// its source by a path with a link in it). A `..` straight after a link is still refused: gix takes it out by reading the path,
/// and the system goes up from where the link leads.
pub(super) fn is_followable(path: &Path) -> bool {
    follows_real_folders(path, true)
}

/// The walk of [`is_physical`] and [`is_followable`] (`links_fine`).
fn follows_real_folders(path: &Path, links_fine: bool) -> bool {
    if !path.is_absolute() {
        return false;
    }

    let mut at = PathBuf::new();
    let mut in_folder = true;
    let mut components = path.components();

    while let Some(component) = components.next() {
        match component {
            Component::Prefix(_) | Component::RootDir => at.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !in_folder {
                    return false;
                }
                at.pop();
            }
            Component::Normal(name) => {
                at.push(name);

                match fs::symlink_metadata(&at) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        if !links_fine {
                            return false;
                        }
                        in_folder = false;
                    }
                    Ok(metadata) => in_folder = metadata.is_dir(),
                    Err(error) if leads_nowhere(&error) => {
                        return components.all(|rest| !matches!(rest, Component::ParentDir));
                    }
                    Err(_) => return false,
                }
            }
        }
    }

    true
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
#[path = "links_tests.rs"]
mod tests;

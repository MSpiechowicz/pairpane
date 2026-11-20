//! Resolving a path link by link, checking every hop, so no link on the way can make Windows connect to a network
//! host or open a device.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use super::{is_unsafe_name, may_follow};

/// Most links followed while resolving one path, like the Linux kernel's own limit.
pub(super) const MAX_LINKS: usize = 40;

/// Most path components handled while resolving one path (links add the components of their targets).
const MAX_STEPS: usize = 4_096;

/// One component of a path still to be resolved.
enum Step {
    Prefix(OsString),
    Root,
    Parent,
    Name(OsString),
}

fn steps(path: &Path) -> impl DoubleEndedIterator<Item = Step> {
    path.components().filter_map(|component| match component {
        Component::Prefix(prefix) => Some(Step::Prefix(prefix.as_os_str().to_owned())),
        Component::RootDir => Some(Step::Root),
        Component::ParentDir => Some(Step::Parent),
        Component::Normal(name) => Some(Step::Name(name.to_owned())),
        Component::CurDir => None,
    })
}

/// Resolves `link` (an absolute path, normally a symbolic link) to the local path it finally leads to, or `None` when
/// it must not be followed: see [`resolve_local`] for what refuses it.
pub fn local_target(link: &Path) -> Option<PathBuf> {
    resolve_local(link).ok()
}

/// Resolves the absolute `path` to the local path it finally leads to, or fails when it must not be touched
/// (`InvalidInput`): a link on the way (in the path itself or in the target of another link) leads to a network or
/// device path, or to a drive-relative or rooted-without-drive path (`C:x`, `\x`), or has a name Windows rewrites or reads as a device (see
/// [`is_unsafe_name`]); or there are too many links or components (a loop). A component that cannot be read fails with
/// its own error, such as `NotFound`.
///
/// `fs::metadata` and `fs::read_dir` resolve a whole chain by themselves, so checking only a link's own target is not
/// enough: on Windows `a -> C:\x\b` with `b -> \\host\share` (or any path through a folder link or junction to a
/// network share) would make the system connect to that host. This resolves the path one component at a time with
/// `symlink_metadata` only (nothing is listed), reading each link with `read_link` and checking every hop's target
/// before anything below it is touched. The path can still change between the check and the use; that matters only to
/// someone who can already write to the folders on it.
pub fn resolve_local(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(refused("the path must be absolute"));
    }

    let mut pending: Vec<Step> = steps(path).rev().collect();
    let mut resolved = PathBuf::new();
    let (mut links, mut handled) = (0, 0);

    while let Some(step) = pending.pop() {
        handled += 1;
        if handled > MAX_STEPS {
            return Err(refused("this folder leads through too many links"));
        }

        match step {
            Step::Prefix(prefix) => {
                if !may_follow(Path::new(&prefix)) {
                    return Err(refused("this folder leads to a network or device path"));
                }
                // `C:x` is relative to a working folder per drive: not a place this can check.
                if !matches!(pending.last(), Some(Step::Root)) {
                    return Err(refused("this folder leads to a path relative to a drive"));
                }
                resolved = PathBuf::from(prefix);
            }
            Step::Root => {
                // A root without a drive (`\x` on Windows) stays on the drive reached so far.
                let mut root = PathBuf::new();
                if let Some(Component::Prefix(prefix)) = resolved.components().next() {
                    root.push(prefix.as_os_str());
                }
                root.push(Component::RootDir);
                resolved = root;
            }
            Step::Parent => {
                resolved.pop();
            }
            Step::Name(name) => {
                // Windows opens `x ` as `x`, so a name like that would be checked as one thing and used as another.
                if is_unsafe_name(&name.to_string_lossy()) {
                    return Err(refused("this path has a name that cannot be opened here"));
                }

                // `push` re-reads its argument: a name holding a separator could add several components, or replace
                // the whole path (`//host/share`). Only a step of exactly one name is taken.
                let next = resolved.join(&name);
                if next.parent() != Some(resolved.as_path()) || next.file_name() != Some(name.as_os_str()) {
                    return Err(refused("this path has a name that cannot be opened here"));
                }
                resolved = next;
                if !fs::symlink_metadata(&resolved)?.file_type().is_symlink() {
                    continue;
                }

                links += 1;
                if links > MAX_LINKS {
                    return Err(refused("this folder leads through too many links"));
                }

                // The target replaces the link: an absolute one restarts the path, a relative one continues from
                // the folder that held the link.
                let target = fs::read_link(&resolved)?;
                if cfg!(windows) && is_rooted_without_drive(&target) {
                    return Err(refused("this folder leads to a path that cannot be checked"));
                }
                resolved.pop();
                pending.extend(steps(&target).rev());
            }
        }
    }

    Ok(resolved)
}

/// A path that starts at a root but names no drive (`\x`, `\Device\Mup\h\s`, `\GLOBAL??\UNC\h\s`). Windows reads a
/// link target like that as relative to the link's own drive or as a path in the system's own namespace, and
/// `read_link` cannot say which, so it is never followed there. (Elsewhere a root is just the root.)
fn is_rooted_without_drive(target: &Path) -> bool {
    matches!(target.components().next(), Some(Component::RootDir))
}

fn refused(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_root_without_a_drive_is_rooted_without_one() {
        assert!(is_rooted_without_drive(Path::new("/x")));
        assert!(!is_rooted_without_drive(Path::new("x")));
        assert!(!is_rooted_without_drive(Path::new("../x")));

        #[cfg(windows)]
        {
            for rooted in [r"\x", r"\Device\Mup\h\s", r"\GLOBAL??\UNC\h\s", r"\DosDevices\UNC\h\s"] {
                assert!(is_rooted_without_drive(Path::new(rooted)), "{rooted}");
            }
            for ok in [r"C:\x", r"\\?\C:\x", r"x\y", r"..\x"] {
                assert!(!is_rooted_without_drive(Path::new(ok)), "{ok}");
            }
        }
    }

    /// In a `\\?\` path only `\` separates, so `//h/s` is one component; joining it would make the path `\\h\s`.
    /// It is refused before anything is touched (no file system needed).
    #[cfg(windows)]
    #[test]
    fn a_verbatim_component_with_separators_is_refused_untouched() {
        for path in [r"\\?\C:\//h/s", r"\\?\C:\a/b"] {
            let error = resolve_local(Path::new(path)).expect_err(path);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{path}");
        }
    }
}

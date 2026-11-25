//! Renaming an item of a folder held open, without replacing anything: on Unix by the handle of the folder (`renameat`),
//! so the names are looked up in that folder whatever happens to the path it was opened by; on Windows by path, as the
//! other Windows jobs go.
//!
//! Races that remain, as in the other jobs of this crate: where the filesystem cannot refuse a replacing rename itself, a
//! file is linked to its new name (which fails when the name is taken) and its old name removed while it is still that
//! file, so another file put at the old name in between is kept; a folder, a link and a file on a filesystem without hard
//! links are checked for and then renamed, two steps that someone who can write to the folder in between can win; and on
//! Windows the folder is a path, so a folder swapped for a link between two renames is followed.

use std::io;

use super::held::Held;

#[cfg(unix)]
impl Held {
    /// Renames the item `from` of this folder to `to`, unless something (a link too) is at `to`: then `AlreadyExists`,
    /// and nothing changes. Both are plain names.
    ///
    /// Linux and macOS ask the system to refuse (`renameat2` `NOREPLACE`, `renameatx_np` `EXCL`), so nothing can appear
    /// at `to` between the check and the rename. Where the filesystem lacks that flag, and on the other systems, see
    /// [`link_or_check`].
    pub(super) fn rename_noreplace(&self, from: &str, to: &str) -> io::Result<()> {
        system_noreplace(&self.dir, from, to)
    }
}

#[cfg(windows)]
impl Held {
    /// Renames the item `from` of this folder to `to`, unless something is at `to` (see `rename::noreplace`).
    pub(super) fn rename_noreplace(&self, from: &str, to: &str) -> io::Result<()> {
        super::rename::noreplace(&self.path.join(from), &self.path.join(to))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn system_noreplace(dir: &std::fs::File, from: &str, to: &str) -> io::Result<()> {
    use rustix::fs::{RenameFlags, renameat_with};
    use rustix::io::Errno;

    match renameat_with(dir, from, dir, to, RenameFlags::NOREPLACE) {
        // The system or the filesystem does not know the flag (macOS answers `ENOTSUP` for a filesystem without `EXCL`;
        // on Linux the two codes are one).
        Err(errno) if [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP, Errno::NOTSUP].contains(&errno) => {
            link_or_check(dir, from, to)
        }
        result => result.map_err(io::Error::from),
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn system_noreplace(dir: &std::fs::File, from: &str, to: &str) -> io::Result<()> {
    link_or_check(dir, from, to)
}

/// A rename of `from` to `to` (relative to `dir`, or paths with `CWD`) that replaces nothing, where the system has no
/// such rename: a regular file is linked to `to`, which fails when anything is there, and then its name `from` is
/// removed; anything else, and a file on a filesystem without hard links, is checked for and then renamed (#77).
#[cfg(unix)]
pub(super) fn link_or_check<D: rustix::fd::AsFd + Copy, P: rustix::path::Arg + Copy>(
    dir: D,
    from: P,
    to: P,
) -> io::Result<()> {
    use rustix::fs::{AtFlags, FileType, linkat, renameat, statat, unlinkat};
    use rustix::io::Errno;

    let id = |name: P| statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map(|stat| (stat.st_dev, stat.st_ino));
    let kind = statat(dir, from, AtFlags::SYMLINK_NOFOLLOW).map(|stat| FileType::from_raw_mode(stat.st_mode))?;
    if kind == FileType::RegularFile {
        match linkat(dir, from, dir, to, AtFlags::empty()) {
            Ok(()) => {
                #[cfg(test)]
                BETWEEN.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().take() {
                        hook();
                    }
                });
                // The old name goes only while it is still the file now at the new one: a file another program put there
                // since (a save that renames a new file over it) is kept, and so are both names.
                let linked = id(to);
                if linked.is_err() || id(from).ok() != linked.as_ref().ok().copied() {
                    return Err(io::Error::other(
                        "the item changed while it was being renamed; it is under both names",
                    ));
                }

                return match unlinkat(dir, from, AtFlags::empty()) {
                    Ok(()) => Ok(()),
                    Err(errno) => {
                        // The new name is taken back while it is still that file, so the item is as it was.
                        if id(to).ok() == linked.ok() {
                            let _ = unlinkat(dir, to, AtFlags::empty());
                        }
                        Err(io::Error::from(errno))
                    }
                };
            }
            Err(Errno::EXIST) => return Err(io::Error::from(io::ErrorKind::AlreadyExists)),
            // No hard links here (FAT, exFAT, some network filesystems): the two steps below.
            Err(_) => {}
        }
    }

    match statat(dir, to, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(Errno::NOENT) => renameat(dir, from, dir, to).map_err(io::Error::from),
        Err(errno) => Err(io::Error::from(errno)),
    }
}

#[cfg(all(test, unix))]
thread_local! {
    /// Run once between the link and the removal of the old name, on this thread: a test changes the folder there.
    static BETWEEN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;

    use super::link_or_check;

    #[test]
    fn a_file_is_moved_by_a_link_that_refuses_a_taken_name() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let dir = fs::File::open(temp.path()).expect("folder");
        fs::write(temp.path().join("a"), b"a").expect("a");
        fs::write(temp.path().join("taken"), b"someone else's").expect("taken");

        let refused = link_or_check(&dir, "a", "taken").expect_err("taken");
        assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(temp.path().join("taken")).expect("taken"), b"someone else's");
        assert_eq!(fs::read(temp.path().join("a")).expect("a"), b"a");

        link_or_check(&dir, "a", "b").expect("moves");
        assert!(!temp.path().join("a").exists());
        assert_eq!(fs::read(temp.path().join("b")).expect("b"), b"a");
    }

    #[test]
    fn a_file_saved_at_the_old_name_in_between_is_kept() {
        // Another program renames a new file over the old name between the link and its removal (#77).
        let temp = tempfile::TempDir::new().expect("temp dir");
        let dir = fs::File::open(temp.path()).expect("folder");
        fs::write(temp.path().join("a"), b"old").expect("a");
        let folder = temp.path().to_owned();
        super::BETWEEN.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(folder.join("saved"), b"saved meanwhile").expect("saved");
                fs::rename(folder.join("saved"), folder.join("a")).expect("over the old name");
            }));
        });

        let error = link_or_check(&dir, "a", "b").expect_err("changed");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(fs::read(temp.path().join("a")).expect("a"), b"saved meanwhile");
        assert_eq!(fs::read(temp.path().join("b")).expect("b"), b"old");
    }

    #[test]
    fn a_folder_and_a_link_are_renamed_as_themselves() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let dir = fs::File::open(temp.path()).expect("folder");
        fs::create_dir(temp.path().join("f")).expect("folder");
        std::os::unix::fs::symlink("f", temp.path().join("l")).expect("link");

        link_or_check(&dir, "f", "g").expect("folder moves");
        link_or_check(&dir, "l", "m").expect("link moves");

        assert!(temp.path().join("g").is_dir());
        assert_eq!(
            fs::read_link(temp.path().join("m")).expect("a link"),
            std::path::Path::new("f")
        );
        let refused = link_or_check(&dir, "g", "m").expect_err("taken");
        assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
    }
}

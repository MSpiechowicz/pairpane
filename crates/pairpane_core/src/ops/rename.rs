//! Renaming that never replaces by accident: a no-replace rename, the rename shortcut built on it, and the
//! check that an error means "another disk".

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::listing::platform;

use super::report::refusal;

/// Windows `ERROR_NOT_SAME_DEVICE`, in case the standard library does not map it to `CrossesDevices`.
const WINDOWS_NOT_SAME_DEVICE: i32 = 17;

/// Moves `from` to `to` unless something is at `to`: then `AlreadyExists`, and nothing changes.
///
/// Linux and macOS ask the system to refuse (`renameat2` `NOREPLACE`, `renamex_np` `EXCL`), so nothing can
/// appear at `to` between the check and the rename. Where the filesystem lacks that flag, and on Windows, it is
/// check-then-rename: someone who can write to the folder at that very moment can still win the race.
pub(super) fn noreplace(from: &Path, to: &Path) -> io::Result<()> {
    match system_noreplace(from, to) {
        Err(error) if refused_flag(&error) => check_then_rename(from, to),
        result => result,
    }
}

/// Moves `from` to `to`, replacing what is at `to` (a file or a link; a folder is refused by the system).
pub(super) fn replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Whether `error` says the two paths are on different disks, so a rename cannot do the move.
pub(super) fn crosses_devices(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices
        || (cfg!(windows) && error.raw_os_error() == Some(WINDOWS_NOT_SAME_DEVICE))
}

fn check_then_rename(from: &Path, to: &Path) -> io::Result<()> {
    match fs::symlink_metadata(to) {
        Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(from, to),
        Err(error) => Err(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn system_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn system_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    check_then_rename(from, to)
}

/// Whether the system rejected the no-replace flag itself (an old kernel, a filesystem without it).
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn refused_flag(error: &io::Error) -> bool {
    use rustix::io::Errno;

    [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP]
        .into_iter()
        .any(|errno| error.raw_os_error() == Some(errno.raw_os_error()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn refused_flag(_error: &io::Error) -> bool {
    false
}

/// Renames `from` to `to` inside `folder` without replacing anything (see [`super::rename_in`]).
pub(super) fn rename_in(folder: &Path, from: &str, to: &str) -> io::Result<()> {
    if !platform::is_plain_name(from) || !platform::is_plain_name(to) {
        return Err(refuse_name());
    }

    // Onto its own name: some systems (macOS) take a no-replace rename of a file onto itself as done.
    if from == to {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "an item with this name exists already",
        ));
    }

    let (from_path, to_path) = (folder.join(from), folder.join(to));

    if platform::CASE_INSENSITIVE && from != to && from.to_lowercase() == to.to_lowercase() {
        return case_only(&from_path, &to_path);
    }

    noreplace(&from_path, &to_path)
}

fn refuse_name() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the name is not a plain file name")
}

/// Names tried for the temporary name of a change of case.
const MAX_SIDE_NAMES: u32 = 100;

/// A change of case only, on a system whose filesystems usually ignore case: `to` "exists" because it is `from`
/// itself, so a rename straight to it would need to replace. A folder can be case-sensitive after all (NTFS can make
/// one), and there `to` may be a different file that a replace would destroy, and nothing can tell the two cases
/// apart beforehand. So the item goes through a temporary name in two steps that each refuse to replace: `from` to
/// the temporary name, and that to `to`. When `to` is taken by another file the second step fails, and the item is
/// put back under its own name.
fn case_only(from: &Path, to: &Path) -> io::Result<()> {
    let side = aside(from)?;

    match noreplace(&side, to) {
        Ok(()) => Ok(()),
        Err(error) => {
            if noreplace(&side, from).is_err() {
                // Nothing else can be done here, and the item must not be lost from sight.
                let name = side.file_name().unwrap_or_default().to_string_lossy().into_owned();

                return Err(io::Error::other(format!("the item was left under the name {name}")));
            }

            Err(error)
        }
    }
}

/// The temporary name an item `original` is moved aside to while it is renamed: `{prefix}-{original}` when that fits in a
/// name, so an item left there (the app stopped half way, the machine went off) still says what it was; else
/// `{prefix}.tmp`. `prefix` starts with `.pairpane-rename-` and makes the name unique (#74).
pub(super) fn aside_name(prefix: &str, original: &str) -> String {
    let named = format!("{prefix}-{original}");
    if super::name_fits(&named) {
        named
    } else {
        format!("{prefix}.tmp")
    }
}

/// Moves `from` to a free temporary name beside it, and returns that path. `NotFound` when `from` is gone.
fn aside(from: &Path) -> io::Result<PathBuf> {
    let (Some(folder), Some(original)) = (from.parent(), from.file_name()) else {
        return Err(refuse_name());
    };
    let original = original.to_string_lossy();

    for number in 0..MAX_SIDE_NAMES {
        let side = folder.join(aside_name(&format!(".pairpane-rename-{number}"), &original));

        match noreplace(from, &side) {
            Ok(()) => return Ok(side),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(refusal(
        io::ErrorKind::AlreadyExists,
        "no free temporary name was found",
    ))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    fn write(path: &Path, text: &str) {
        fs::write(path, text).expect("file");
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("file")
    }

    #[test]
    fn noreplace_moves_to_a_free_name() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), "a");

        noreplace(&temp.path().join("a"), &temp.path().join("b")).expect("renames");

        assert_eq!(read(&temp.path().join("b")), "a");
        assert!(!temp.path().join("a").exists());
    }

    #[test]
    fn noreplace_refuses_an_existing_name_and_changes_nothing() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), "a");
        write(&temp.path().join("b"), "b");

        let error = noreplace(&temp.path().join("a"), &temp.path().join("b")).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&temp.path().join("a")), "a");
        assert_eq!(read(&temp.path().join("b")), "b");
    }

    #[test]
    fn noreplace_refuses_an_existing_link_without_following_it() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), "a");
        let dangling = temp.path().join("dangling");
        if !make_link(&temp.path().join("missing"), &dangling) {
            return;
        }

        let error = noreplace(&temp.path().join("a"), &dangling).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&temp.path().join("a")), "a");
    }

    #[test]
    fn replace_overwrites_a_file() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), "a");
        write(&temp.path().join("b"), "b");

        replace(&temp.path().join("a"), &temp.path().join("b")).expect("replaces");

        assert_eq!(read(&temp.path().join("b")), "a");
    }

    #[test]
    fn a_missing_source_is_not_found() {
        let temp = TempDir::new().expect("temp dir");

        let error = rename_in(temp.path(), "nothing", "else").expect_err("missing");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn rename_in_refuses_an_existing_name() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a.txt"), "a");
        write(&temp.path().join("b.txt"), "b");

        let error = rename_in(temp.path(), "a.txt", "b.txt").expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&temp.path().join("b.txt")), "b");
        assert_eq!(read(&temp.path().join("a.txt")), "a");
    }

    #[test]
    fn rename_in_refuses_names_that_are_not_plain() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), "a");

        for bad in ["", ".", "..", "x/y"] {
            let error = rename_in(temp.path(), "a", bad).expect_err(bad);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad}");
        }
    }

    #[test]
    fn rename_in_to_the_same_name_is_refused_as_existing() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), "a");

        let error = rename_in(temp.path(), "a", "a").expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn a_change_of_case_only_works_where_names_ignore_case_and_is_a_plain_rename_elsewhere() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a.txt"), "a");

        rename_in(temp.path(), "a.txt", "A.TXT").expect("renames");

        let names: Vec<_> = fs::read_dir(temp.path())
            .expect("folder")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["A.TXT"]);
    }

    #[test]
    fn a_change_of_case_never_replaces_another_file_that_differs_only_in_case() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a.txt"), "a");
        write(&temp.path().join("A.TXT"), "upper");
        // Where names ignore case these are one file; there is nothing to check.
        if !temp.path().join("a.txt").exists() || read(&temp.path().join("a.txt")) != "a" {
            return;
        }

        let result = rename_in(temp.path(), "a.txt", "A.TXT");

        assert_eq!(result.expect_err("refused").kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&temp.path().join("A.TXT")), "upper");
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .expect("folder")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();

        names
    }

    #[test]
    fn a_change_of_case_goes_through_a_temporary_name_and_leaves_nothing_behind() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a.txt"), "a");

        case_only(&temp.path().join("a.txt"), &temp.path().join("A.TXT")).expect("renames");

        assert_eq!(names(temp.path()), ["A.TXT"]);
        assert_eq!(read(&temp.path().join("A.TXT")), "a");
    }

    /// Asks `case_only` for `a.txt` to `A.TXT`, which is taken, and checks that it is refused and both names are
    /// as they were. `make_taken` makes the `A.TXT`; `false` when this system cannot hold both names (it ignores
    /// case, or cannot hard link).
    fn refused_onto_a_taken_name(make_taken: impl FnOnce(&Path) -> bool) {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a.txt"), "a");
        if !make_taken(temp.path()) || names(temp.path()).len() != 2 {
            return;
        }

        let error = case_only(&temp.path().join("a.txt"), &temp.path().join("A.TXT")).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(names(temp.path()), ["A.TXT", "a.txt"]);
        assert_eq!(read(&temp.path().join("a.txt")), "a");
    }

    #[test]
    fn a_change_of_case_onto_a_different_file_is_refused_and_puts_the_item_back() {
        refused_onto_a_taken_name(|folder| {
            write(&folder.join("A.TXT"), "upper");

            true
        });
    }

    #[test]
    fn a_change_of_case_onto_another_name_of_the_same_file_is_refused_too() {
        // Two names of one file, in a folder that tells them apart. A rename between them would change nothing and
        // still say it worked; the name `A.TXT` is taken.
        refused_onto_a_taken_name(|folder| fs::hard_link(folder.join("a.txt"), folder.join("A.TXT")).is_ok());
    }

    #[test]
    fn a_temporary_name_that_is_taken_is_skipped_and_left_alone() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a.txt"), "a");
        write(&temp.path().join(".pairpane-rename-0-a.txt"), "someone else's");

        case_only(&temp.path().join("a.txt"), &temp.path().join("A.TXT")).expect("renames");

        assert_eq!(names(temp.path()), [".pairpane-rename-0-a.txt", "A.TXT"]);
        assert_eq!(read(&temp.path().join(".pairpane-rename-0-a.txt")), "someone else's");
    }

    #[test]
    fn a_temporary_name_says_what_the_item_was_unless_that_is_too_long() {
        use super::aside_name;

        assert_eq!(
            aside_name(".pairpane-rename-3", "report.pdf"),
            ".pairpane-rename-3-report.pdf"
        );
        let long = "x".repeat(250);
        assert_eq!(aside_name(".pairpane-rename-3", &long), ".pairpane-rename-3.tmp");
    }

    #[test]
    fn a_change_of_case_of_a_missing_item_is_not_found() {
        let temp = TempDir::new().expect("temp dir");

        let error = case_only(&temp.path().join("a.txt"), &temp.path().join("A.TXT")).expect_err("missing");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(names(temp.path()), Vec::<String>::new());
    }

    #[test]
    fn the_cross_device_error_is_recognised() {
        assert!(crosses_devices(&io::Error::from(io::ErrorKind::CrossesDevices)));
        assert!(!crosses_devices(&io::Error::from(io::ErrorKind::NotFound)));
    }

    /// Makes a link at `link`; `false` when this system does not allow it.
    fn make_link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(target, link);

        made.is_ok()
    }
}

//! What a copy keeps besides the bytes: modification and access time (and creation time where the system lets
//! it be set), and permissions. Extended attributes (with the POSIX ACLs kept in them) and alternate data streams are
//! carried by `extras`; ownership is not copied.

use std::fs::{File, FileTimes, Metadata};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::open;

/// The times of `source` as a value to set.
fn times(source: &Metadata) -> FileTimes {
    let mut times = FileTimes::new();

    if let Ok(modified) = source.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = source.accessed() {
        times = times.set_accessed(accessed);
    }

    #[cfg(target_os = "macos")]
    if let Ok(created) = source.created() {
        use std::os::macos::fs::FileTimesExt;

        times = times.set_created(created);
    }
    #[cfg(windows)]
    if let Ok(created) = source.created() {
        use std::os::windows::fs::FileTimesExt;

        times = times.set_created(created);
    }

    times
}

/// Gives the open file `file` the times and permissions of `source`, as read before the source was copied
/// (reading it changes its access time). Call it after the last write: writing changes the times.
///
/// On Unix the permission bits are kept without setuid, setgid and sticky (a sticky bit on a file means something
/// on macOS and some BSDs); on Windows the read-only attribute is, last.
pub(super) fn apply_file(file: &File, source: &Metadata) -> io::Result<()> {
    file.set_times(times(source))?;

    apply_permissions(file, source, FILE_MODE)
}

/// The Unix permission bits a copied file keeps: no setuid, setgid or sticky bit (unused on Windows).
const FILE_MODE: u32 = 0o0777;

/// The Unix permission bits an archive entry keeps: no write for the group and others (an archive must not make what it
/// extracts writable by everyone) and no setuid, setgid or sticky bit.
#[cfg(unix)]
const ENTRY_MODE: u32 = 0o0755;

/// Gives the open file or folder `file` what an archive entry states: its modification time (milliseconds since the
/// Unix epoch; one that cannot be set is left out) and, when `mode` is given, its Unix permission bits as [`ENTRY_MODE`]
/// keeps them, always with read and write for the owner (and, on a folder, entering it), so what is extracted can be read
/// and removed whatever the archive says. On Windows only the read-only attribute follows from `mode` (no owner write
/// bit), and a folder gets none. Call it after the last write.
pub(super) fn apply_entry(file: &File, mode: Option<u32>, modified: Option<i64>) -> io::Result<()> {
    let time = modified.and_then(|millis| {
        let span = Duration::from_millis(millis.unsigned_abs());

        if millis >= 0 {
            SystemTime::UNIX_EPOCH.checked_add(span)
        } else {
            SystemTime::UNIX_EPOCH.checked_sub(span)
        }
    });

    if let Some(time) = time {
        file.set_times(FileTimes::new().set_modified(time))?;
    }

    match mode {
        Some(mode) => entry_permissions(file, mode),
        None => Ok(()),
    }
}

#[cfg(unix)]
fn entry_permissions(file: &File, mode: u32) -> io::Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let owner = if file.metadata()?.is_dir() { 0o700 } else { 0o600 };

    file.set_permissions(Permissions::from_mode(mode & ENTRY_MODE | owner))
}

#[cfg(windows)]
fn entry_permissions(file: &File, mode: u32) -> io::Result<()> {
    if mode & 0o200 != 0 || file.metadata()?.is_dir() {
        return Ok(());
    }

    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);

    file.set_permissions(permissions)
}

/// Gives the folder at `path` what an archive entry states (see [`apply_entry`]), once its contents are in.
pub(super) fn apply_entry_folder(path: &Path, mode: Option<u32>, modified: Option<i64>) -> io::Result<()> {
    apply_entry(&open::folder(path)?, mode, modified)
}

/// The Unix permission bits a copied folder keeps: the sticky bit stays (a shared folder like `/tmp` needs it), setuid
/// and setgid do not.
#[cfg(unix)]
const FOLDER_MODE: u32 = 0o1777;

#[cfg(unix)]
fn apply_permissions(file: &File, source: &Metadata, keep: u32) -> io::Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(Permissions::from_mode(source.permissions().mode() & keep))
}

#[cfg(windows)]
fn apply_permissions(file: &File, source: &Metadata, _keep: u32) -> io::Result<()> {
    if !source.permissions().readonly() {
        return Ok(());
    }

    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);

    file.set_permissions(permissions)
}

/// Gives the folder at `path` the times and permissions of `source`. Called after its contents are in, as
/// adding them changes its modification time (and a read-only folder could not be filled).
pub(super) fn apply_folder(path: &Path, source: &Metadata) -> io::Result<()> {
    let folder = open::folder(path)?;
    folder.set_times(times(source))?;

    #[cfg(unix)]
    apply_permissions(&folder, source, FOLDER_MODE)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    const PAST: Duration = Duration::from_secs(1_000_000_000);

    /// A folder with the files `source` and `copy`.
    fn pair() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new().expect("temp dir");
        let (source, copy) = (temp.path().join("source"), temp.path().join("copy"));
        fs::write(&source, "x").expect("file");
        fs::write(&copy, "x").expect("file");

        (temp, source, copy)
    }

    /// Gives `copy` the times and permissions of `source`, as the copy of a file does.
    fn apply(source: &Path, copy: &Path) {
        let metadata = fs::metadata(source).expect("metadata");

        apply_file(&File::options().write(true).open(copy).expect("open"), &metadata).expect("applies");
    }

    #[test]
    fn a_file_gets_the_times_of_its_source() {
        let (_temp, source, copy) = pair();
        let when = SystemTime::UNIX_EPOCH + PAST;
        let times = FileTimes::new().set_modified(when).set_accessed(when);
        File::options()
            .write(true)
            .open(&source)
            .expect("open")
            .set_times(times)
            .expect("times");

        apply(&source, &copy);

        let copied = fs::metadata(&copy).expect("metadata");
        assert_eq!(copied.modified().expect("modified"), when);
        assert_eq!(copied.accessed().expect("accessed"), when);
    }

    #[test]
    fn a_folder_gets_the_times_of_its_source() {
        let temp = TempDir::new().expect("temp dir");
        let (source, copy) = (temp.path().join("source"), temp.path().join("copy"));
        fs::create_dir(&source).expect("dir");
        fs::create_dir(&copy).expect("dir");
        let when = SystemTime::UNIX_EPOCH + PAST;
        open::folder(&source)
            .and_then(|folder| folder.set_times(FileTimes::new().set_modified(when)))
            .expect("times");

        apply_folder(&copy, &fs::metadata(&source).expect("metadata")).expect("applies");

        assert_eq!(
            fs::metadata(&copy).expect("metadata").modified().expect("modified"),
            when
        );
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_kept_without_setuid_and_setgid() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, source, copy) = pair();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o6751)).expect("mode");

        apply(&source, &copy);

        let mode = fs::metadata(&copy).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o7777, 0o751);
    }

    #[cfg(unix)]
    #[test]
    fn the_sticky_bit_is_dropped_from_a_file_and_kept_on_a_folder() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, source, copy) = pair();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o1751)).expect("mode");
        apply(&source, &copy);

        let folder_source = source.with_file_name("shared");
        let folder_copy = source.with_file_name("shared-copy");
        for folder in [&folder_source, &folder_copy] {
            fs::create_dir(folder).expect("folder");
        }
        fs::set_permissions(&folder_source, fs::Permissions::from_mode(0o3777)).expect("mode");
        apply_folder(&folder_copy, &fs::metadata(&folder_source).expect("metadata")).expect("applies");

        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o7777;
        assert_eq!(mode(&copy), 0o751);
        assert_eq!(mode(&folder_copy), 0o1777);
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_keeps_no_write_for_others_and_always_the_owner_read_and_write() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("temp dir");
        let folder = temp.path().join("folder");
        fs::create_dir(&folder).expect("folder");
        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o7777;

        for (path, stated, kept) in [
            ("file", 0o6777, 0o755),
            ("file", 0o777, 0o755),
            ("file", 0o666, 0o644),
            ("file", 0o000, 0o600),
            ("file", 0o4400, 0o600),
        ] {
            let target = temp.path().join(path);
            let file = File::create(&target).expect("file");

            apply_entry(&file, Some(stated), None).expect("applies");

            assert_eq!(mode(&target), kept, "{stated:o}");
        }

        for (stated, kept) in [(0o1777, 0o755), (0o000, 0o700), (0o555, 0o755)] {
            apply_entry_folder(&folder, Some(stated), None).expect("applies");

            assert_eq!(mode(&folder), kept, "{stated:o}");
        }
    }

    #[cfg(windows)]
    #[test]
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the Windows read-only attribute; there is no world-writable bit to set"
    )]
    fn the_read_only_attribute_is_kept() {
        let (_temp, source, copy) = pair();
        let mut permissions = fs::metadata(&source).expect("metadata").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions).expect("read-only");

        apply(&source, &copy);

        assert!(fs::metadata(&copy).expect("metadata").permissions().readonly());
        for path in [&source, &copy] {
            let mut permissions = fs::metadata(path).expect("metadata").permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).expect("writable again");
        }
    }
}

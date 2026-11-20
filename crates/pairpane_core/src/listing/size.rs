//! Folder sizes: the bytes of all files below a folder, for the panes' "calculate size" command.
//!
//! The walk is a plain iterative one that never follows a link below the root, stays on the root's device
//! (like `du -x`), walks each directory once, and skips what it cannot read.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::platform;

/// `progress` is called at most this often.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Sums the sizes of the regular files below `root` (absolute), calling `progress` with the running total now
/// and then. Returns `None` once `cancel` is set. Fails only when `root` itself cannot be read.
///
/// Symbolic links and junctions below `root` are never followed or counted, nor is a subfolder on another
/// device. On Unix `root` itself may be a symbolic link. On Windows it may not: a link there can lead through
/// several hops to a network path, and only the first hop can be checked without touching the target.
pub fn folder_size(root: &Path, cancel: &AtomicBool, progress: impl FnMut(u64)) -> io::Result<Option<u64>> {
    walk(root, cancel, PROGRESS_INTERVAL, progress)
}

/// [`folder_size`] with its own progress interval.
fn walk(
    root: &Path,
    cancel: &AtomicBool,
    interval: Duration,
    mut progress: impl FnMut(u64),
) -> io::Result<Option<u64>> {
    if !root.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the path must be absolute"));
    }

    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }

    if root_link_refused(root)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "this link cannot be followed",
        ));
    }

    let root_metadata = fs::metadata(root)?;
    let root_device = platform::device(&root_metadata);
    let mut visited = HashSet::new();
    first_visit(&mut visited, &root_metadata);
    let mut total = 0_u64;
    let mut reported = Instant::now();
    let mut pending: Vec<PathBuf> = Vec::new();

    // The root must be readable: that error is the caller's. Everything below it is best effort.
    let mut folder = fs::read_dir(root)?;

    loop {
        for entry in folder {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }

            let Ok(entry) = entry else { continue };
            let Ok(file_type) = entry.file_type() else { continue };

            if file_type.is_dir() {
                // The folder is opened by path later, so it may be swapped for another in between; accepted, as the
                // walk only reads sizes.
                if let Ok(metadata) = entry.metadata()
                    && platform::device(&metadata) == root_device
                    && first_visit(&mut visited, &metadata)
                {
                    pending.push(entry.path());
                }
            } else if file_type.is_file() {
                let len = entry.metadata().map_or(0, |metadata| metadata.len());
                total = total.saturating_add(len);
            }

            if reported.elapsed() >= interval {
                progress(total);
                reported = Instant::now();
            }
        }

        // The next readable subfolder; one that cannot be read is skipped.
        folder = loop {
            let Some(path) = pending.pop() else {
                return Ok(Some(total));
            };
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }

            if let Ok(next) = fs::read_dir(path) {
                break next;
            }
        };
    }
}

/// Whether `root` is a link the walk must not start from: any link on Windows (it may chain to a network path),
/// elsewhere one that leads to a network or device path.
fn root_link_refused(root: &Path) -> io::Result<bool> {
    if !fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Ok(false);
    }

    Ok(cfg!(windows) || !fs::read_link(root).is_ok_and(|target| platform::may_follow(&target)))
}

/// Records a directory as walked; `false` when it already was. Always `true` where directories have no identity.
fn first_visit(visited: &mut HashSet<(u64, u64)>, metadata: &Metadata) -> bool {
    platform::identity(metadata).is_none_or(|identity| visited.insert(identity))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tempfile::TempDir;

    use super::*;

    fn size(root: &Path) -> io::Result<Option<u64>> {
        folder_size(root, &AtomicBool::new(false), |_| {})
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, vec![b'x'; bytes]).expect("file");
    }

    #[test]
    fn sums_files_at_every_depth() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), 10);
        write(&temp.path().join("sub/b"), 20);
        write(&temp.path().join("sub/deep/er/c"), 30);
        fs::create_dir(temp.path().join("empty")).expect("dir");

        assert_eq!(size(temp.path()).expect("walks"), Some(60));
    }

    #[test]
    fn a_file_or_missing_root_is_an_error() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("file"), 1);

        assert!(size(&temp.path().join("file")).is_err());
        assert!(size(&temp.path().join("missing")).is_err());
        assert!(size(Path::new("relative")).is_err());
    }

    #[test]
    fn a_set_cancel_flag_stops_the_walk() {
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("sub/a"), 10);

        let cancelled = folder_size(temp.path(), &AtomicBool::new(true), |_| {});

        assert_eq!(cancelled.expect("walks"), None);
    }

    #[test]
    fn cancelling_from_progress_stops_a_running_walk() {
        let temp = TempDir::new().expect("temp dir");
        for index in 0..50 {
            write(&temp.path().join(format!("d{index}/f")), 1);
        }
        let cancel = AtomicBool::new(false);
        let calls = Cell::new(0);

        let result = walk(temp.path(), &cancel, Duration::ZERO, |_| {
            calls.set(calls.get() + 1);
            cancel.store(true, Ordering::Relaxed);
        });

        assert_eq!(result.expect("walks"), None);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn cancelling_after_the_last_entry_does_not_walk_the_subfolders_left() {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join("only")).expect("dir");
        let cancel = AtomicBool::new(false);

        // Progress fires after the root's only entry, so the cancel lands between entries and the next subfolder.
        let result = walk(temp.path(), &cancel, Duration::ZERO, |_| {
            cancel.store(true, Ordering::Relaxed)
        });

        assert_eq!(result.expect("walks"), None);
    }

    #[test]
    fn reports_the_running_total() {
        let temp = TempDir::new().expect("temp dir");
        for index in 0..5 {
            write(&temp.path().join(format!("f{index}")), 10);
        }
        let mut seen = Vec::new();

        let total = walk(temp.path(), &AtomicBool::new(false), Duration::ZERO, |bytes| {
            seen.push(bytes)
        });

        assert_eq!(total.expect("walks"), Some(50));
        assert!(!seen.is_empty());
        assert!(seen.iter().all(|bytes| *bytes <= 50), "{seen:?}");
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
    }

    #[cfg(unix)]
    #[test]
    fn links_below_the_root_are_not_followed() {
        use std::os::unix::fs::symlink;

        let outside = TempDir::new().expect("temp dir");
        write(&outside.path().join("big"), 1000);
        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), 10);
        symlink(outside.path(), temp.path().join("folder-link")).expect("link");
        symlink(outside.path().join("big"), temp.path().join("file-link")).expect("link");
        symlink(temp.path(), temp.path().join("loop")).expect("link");

        assert_eq!(size(temp.path()).expect("walks"), Some(10));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_seen_twice_is_walked_once() {
        let temp = TempDir::new().expect("temp dir");
        let metadata = fs::metadata(temp.path()).expect("metadata");
        let mut visited = HashSet::new();

        assert!(first_visit(&mut visited, &metadata));
        assert!(!first_visit(&mut visited, &metadata));
    }

    #[cfg(unix)]
    #[test]
    fn a_root_link_to_a_folder_is_followed() {
        use std::os::unix::fs::symlink;

        let target = TempDir::new().expect("temp dir");
        write(&target.path().join("a"), 10);
        let temp = TempDir::new().expect("temp dir");
        symlink(target.path(), temp.path().join("link")).expect("link");

        assert_eq!(size(&temp.path().join("link")).expect("walks"), Some(10));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_subfolder_is_skipped() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("temp dir");
        write(&temp.path().join("a"), 10);
        write(&temp.path().join("locked/b"), 100);
        let locked = temp.path().join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

        // Root ignores permissions, so there is nothing to skip for it.
        let readable = fs::read_dir(&locked).is_ok();
        let result = size(temp.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");

        assert_eq!(result.expect("walks"), Some(if readable { 110 } else { 10 }));
    }

    #[cfg(unix)]
    #[test]
    fn another_device_is_not_entered() {
        // Only meaningful where `/dev/shm` is a separate filesystem from `/dev`; elsewhere there is nothing to check.
        let dev = Path::new("/dev");
        let shm = Path::new("/dev/shm");
        if !(dev.is_dir() && shm.is_dir()) {
            return;
        }
        let same =
            platform::device(&fs::metadata(dev).expect("dev")) == platform::device(&fs::metadata(shm).expect("shm"));
        if same {
            return;
        }

        // `/dev` is on devtmpfs and `/dev/shm` on tmpfs: the walk of `/dev` must not add what is in `/dev/shm`.
        let marker = TempDir::new_in(shm).expect("temp dir in shm");
        write(&marker.path().join("big"), 4_000_000);

        let total = size(dev).expect("walks").expect("not cancelled");

        assert!(total < 4_000_000, "{total}");
    }
}

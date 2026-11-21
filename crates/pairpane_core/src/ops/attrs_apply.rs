//! What changing permissions and times does to one item, by platform. Nothing here follows a link: the callers skip
//! links, and what is swapped in afterwards is refused.
//!
//! Unix: every step is relative to the handle of the folder holding the item (see `held`). Times go through that
//! handle and the name (`utimensat`, no-follow) after the entry is compared with what the walk saw, so they need no
//! access to the item; permissions through a handle opened from the folder without following links and compared (`fchmod`),
//! and, for an item that cannot be opened at all (no read permission), through a handle that needs no read permission
//! and is never a link (see `set_without_opening`). Windows: a handle opened for attributes only, without following
//! links, sets the times and the attributes (read-only on files; hidden, system and archive on files and folders; see
//! `win_attrs`).

use std::io;

use crate::model::AttributeChange;
#[cfg(any(unix, test))]
use crate::model::ModeChange;

use super::held::{Held, Node};
use super::report::refusal;

/// The permission bits that `change` gives an item that has `current` now: the bits of `remove` cleared, then the
/// bits of `add` set; or exactly `mode`.
#[cfg(any(unix, test))]
pub(super) fn new_mode(change: ModeChange, current: u32) -> u32 {
    match change {
        ModeChange::Set { mode } => mode,
        ModeChange::Adjust { add, remove } => (current & !remove) | add,
    }
}

/// The time `millis` after the Unix epoch (before it when negative), when the system can spell it.
#[cfg(any(windows, test))]
pub(super) fn system_time(millis: i64) -> Option<std::time::SystemTime> {
    use std::time::{Duration, SystemTime};

    let span = Duration::from_millis(millis.unsigned_abs());

    if millis >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(span)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(span)
    }
}

/// Whether a folder with the permission bits `mode` can still be listed once `change` is made, so the contents are
/// changed first. A folder that cannot is changed after its contents.
pub(super) fn stays_open(change: &AttributeChange, mode: u32) -> bool {
    #[cfg(unix)]
    {
        change.mode.is_none_or(|change| new_mode(change, mode) & 0o500 == 0o500)
    }
    #[cfg(not(unix))]
    {
        let _ = (change, mode);

        true
    }
}

#[cfg(unix)]
fn changed() -> io::Error {
    refusal(
        io::ErrorKind::Other,
        "it changed while its attributes were being changed",
    )
}

#[cfg(unix)]
impl Held {
    /// Gives `node`, an entry of this folder, the permissions of `change`, if it has any.
    pub(super) fn set_permissions(&self, node: &Node, change: &AttributeChange) -> io::Result<()> {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let Some(mode) = change.mode else {
            return Ok(());
        };
        if new_mode(mode, node.mode) == node.mode {
            return Ok(());
        }

        match self.open_entry(node) {
            Ok(file) => {
                // The mode adjusted is the one the item has now.
                let current = file.metadata()?.permissions().mode() & 0o7777;

                file.set_permissions(Permissions::from_mode(new_mode(mode, current)))
            }
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => self.set_without_opening(node, mode),
            Err(error) => Err(error),
        }
    }

    /// The change for an item that cannot be opened (it has no read permission). Never done by a path or by a call that
    /// follows a link: the item is looked at through the folder's own handle, once, and what is changed is that very
    /// object, only when it is the item that was seen and no link.
    ///
    /// Linux and Android open the entry itself without needing any permission on it (`O_PATH`, no-follow), check what
    /// the handle is (the mode to adjust is the one it has now), and change it through `/proc/self/fd`; where `/proc`
    /// is not there the item is refused. Other systems check the entry with `fstatat` (no-follow) and change it with
    /// `fchmodat` and `AT_SYMLINK_NOFOLLOW`, which they support; the name is in the folder this handle holds, so no
    /// swap of a folder can send it elsewhere.
    pub(super) fn set_without_opening(&self, node: &Node, mode: ModeChange) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use rustix::fs::{Mode, OFlags, fstat, openat};
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::PermissionsExt;

            let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let handle = openat(&self.dir, &node.name, flags, Mode::empty()).map_err(io::Error::from)?;
            let wanted = checked_mode(&fstat(&handle).map_err(io::Error::from)?, node, mode)?;

            // A path that names this very object through the handle, not through any folder.
            let by_handle = format!("/proc/self/fd/{}", handle.as_raw_fd());

            std::fs::set_permissions(by_handle, std::fs::Permissions::from_mode(wanted)).map_err(unavailable)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            use rustix::fs::{AtFlags, Mode, RawMode, chmodat, statat};

            let now = statat(&self.dir, &node.name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
            let wanted = checked_mode(&now, node, mode)?;

            chmodat(
                &self.dir,
                &node.name,
                Mode::from_bits_truncate(wanted as RawMode),
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(io::Error::from)
        }
    }

    /// Gives `node` the times of `change`, if it has any, once the entry is compared with what the walk saw. The item
    /// itself is not opened.
    pub(super) fn set_times(&self, node: &Node, change: &AttributeChange) -> io::Result<()> {
        use rustix::fs::{AtFlags, Timespec, Timestamps, UTIME_OMIT, statat, utimensat};

        if change.modified.is_none() && change.accessed.is_none() {
            return Ok(());
        }

        let now = statat(&self.dir, &node.name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
        if !node.is(&now) {
            return Err(changed());
        }

        let stamp = |millis: Option<i64>| match millis {
            Some(millis) => Timespec {
                tv_sec: millis.div_euclid(1000),
                tv_nsec: millis.rem_euclid(1000) * 1_000_000,
            },
            None => Timespec {
                tv_sec: 0,
                tv_nsec: UTIME_OMIT,
            },
        };
        let times = Timestamps {
            last_access: stamp(change.accessed),
            last_modification: stamp(change.modified),
        };

        utimensat(&self.dir, &node.name, &times, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)
    }
}

/// The permission bits `mode` gives the object `now` (read through a handle), once it is checked to be the entry that
/// was seen: a regular file or a folder, the same device and inode, and the same kind. Never a link.
#[cfg(unix)]
#[allow(
    clippy::unnecessary_cast,
    reason = "the mode is as wide as u32 on some systems and narrower on others"
)]
fn checked_mode(now: &rustix::fs::Stat, node: &Node, mode: ModeChange) -> io::Result<u32> {
    use super::plan::Kind;

    if !node.is(now) || !matches!(node.kind, Kind::Dir | Kind::File) {
        return Err(changed());
    }

    Ok(new_mode(mode, now.st_mode as u32 & 0o7777))
}

/// The error for a handle that cannot be turned into a path (no `/proc`): the item is refused, never changed another way.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn unavailable(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        return refusal(
            io::ErrorKind::PermissionDenied,
            "this item cannot be changed without read access",
        );
    }

    error
}

#[cfg(windows)]
impl Held {
    /// Sets the attributes of `change` on `node` through one handle: read-only on a file (a folder takes none), hidden,
    /// system and archive on a file or a folder.
    pub(super) fn set_permissions(&self, node: &Node, change: &AttributeChange) -> io::Result<()> {
        use std::os::windows::fs::MetadataExt;

        use super::plan::Kind;
        use super::win_attrs::{Wanted, set};

        let wanted = Wanted::of(change, matches!(node.kind, Kind::File));
        if wanted.is_empty() {
            return Ok(());
        }

        let file = super::open::attributes(&node.path)?;
        let metadata = file.metadata()?;
        // A folder swapped in for a file (or the other way) is refused: read-only is for files only.
        if metadata.is_dir() != matches!(node.kind, Kind::Dir) {
            return Err(refusal(
                io::ErrorKind::Other,
                "it changed while its attributes were being changed",
            ));
        }
        // The attributes adjusted are the ones the item has now.
        let current = metadata.file_attributes();

        set(&file, current, wanted)
    }

    /// Sets the times of `change` on `node`.
    pub(super) fn set_times(&self, node: &Node, change: &AttributeChange) -> io::Result<()> {
        use std::fs::FileTimes;

        if change.modified.is_none() && change.accessed.is_none() {
            return Ok(());
        }

        let mut times = FileTimes::new();
        if let Some(millis) = change.modified {
            times = times.set_modified(system_time(millis).ok_or_else(out_of_range)?);
        }
        if let Some(millis) = change.accessed {
            times = times.set_accessed(system_time(millis).ok_or_else(out_of_range)?);
        }

        super::open::attributes(&node.path)?.set_times(times)
    }
}

#[cfg(windows)]
fn out_of_range() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "that time cannot be set")
}

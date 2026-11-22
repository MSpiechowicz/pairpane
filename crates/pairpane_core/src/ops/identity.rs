//! What makes two paths one file or folder: the identity the filesystem gives it, not the spelling of its path.
//!
//! On Unix that is the device and inode. On Windows it is the volume serial number and file index of a handle
//! (the `same-file` crate reads them, safely), so `C:\data\d`, its 8.3 name, another case and `\\?\Volume{GUID}\data\d`
//! are all the same folder. The handle is opened for its attributes only and never follows a link: a link has the
//! identity of the link itself, and nothing here can make Windows connect to a network host.
//!
//! `FolderId` is the same identity as plain numbers, for a log that is kept and must not keep a handle open.
//!
//! An identity that cannot be read (`None`) is never evidence that two paths differ: callers fail closed and treat
//! the two as the same file (nothing is overwritten, deleted or entered). Names are not compared instead, as on
//! Windows an ACL can refuse the open while two spellings of one folder still exist. The file index of a filesystem
//! that does not keep one stable would likewise at worst make two files look the same.

use std::fs::Metadata;
use std::path::Path;

use super::transfer::Run;

#[cfg(windows)]
use super::open::{BACKUP_SEMANTICS, OPEN_REPARSE_POINT};

/// Windows `FILE_READ_ATTRIBUTES`: all that reading a file's identity needs, so a file that is locked or cannot be
/// read still has one.
#[cfg(windows)]
const READ_ATTRIBUTES: u32 = 0x0080;

/// `path` opened for its attributes only, without following a link.
#[cfg(windows)]
fn open_for_identity(path: &Path) -> Option<std::fs::File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .access_mode(READ_ATTRIBUTES)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
        .ok()
}

/// The identity of one existing file, folder or link.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct FileId((u64, u64));

/// The identity of one existing file, folder or link. Holds the handle it was read from open.
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct FileId(same_file::Handle);

impl FileId {
    /// The identity of an entry whose device and inode were read through a handle.
    #[cfg(unix)]
    pub(super) fn of_ids(ids: (u64, u64)) -> Self {
        Self(ids)
    }

    /// The identity of what the walk saw at `path` as `meta` (read without following links).
    #[cfg(unix)]
    pub(super) fn of_meta(_path: &Path, meta: &Metadata) -> Option<Self> {
        crate::listing::platform::identity(meta).map(Self)
    }

    /// The identity of the file at `path`; on Windows the path is opened for it.
    #[cfg(windows)]
    pub(super) fn of_meta(path: &Path, _meta: &Metadata) -> Option<Self> {
        Self::of_path(path)
    }

    /// The identity of what is at `path` right now, a link being its own thing. `None` when it cannot be read.
    #[cfg(unix)]
    pub(super) fn of_path(path: &Path) -> Option<Self> {
        let meta = std::fs::symlink_metadata(path).ok()?;

        Self::of_meta(path, &meta)
    }

    #[cfg(windows)]
    pub(super) fn of_path(path: &Path) -> Option<Self> {
        let file = open_for_identity(path)?;

        same_file::Handle::from_file(file).ok().map(Self)
    }
}

/// The identity of a folder as plain numbers, for what must outlive a job (the log of a rename, kept for an undo).
/// Unlike [`FileId`] it never holds a handle: on Windows the handle it is read from is closed before it is returned,
/// so remembering it blocks no safe-eject and keeps no deleted folder delete-pending.
///
/// It is the device and inode (Unix) or the volume serial number and file index (Windows). Both can be reused when the
/// folder is deleted and another is created, and a filesystem that does not keep them stable (some FUSE and CIFS
/// mounts) gives another one at each open; `RenameLog` says what the undo makes of either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FolderId((u64, u64));

impl FolderId {
    /// The identity of the folder `path` is right now (a link being its own thing); `None` when it cannot be read. On
    /// Unix a job reads it from the folder it holds open (`Held::identity`), so this is for the tests.
    #[cfg(all(unix, test))]
    pub(super) fn of_path(path: &Path) -> Option<Self> {
        let meta = std::fs::symlink_metadata(path).ok()?;

        crate::listing::platform::identity(&meta).map(Self)
    }

    #[cfg(windows)]
    pub(super) fn of_path(path: &Path) -> Option<Self> {
        let file = open_for_identity(path)?;
        let info = winapi_util::file::information(&file).ok()?;

        // `file` is closed here, as the function ends.
        Some(Self((info.volume_serial_number(), info.file_index())))
    }
}

/// Whether two existing paths are one file, given their identities: they are different only when both were read and
/// differ. A missing identity counts as "same", the answer that loses nothing.
pub(super) fn same_place(a: Option<FileId>, b: Option<FileId>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

impl Run<'_> {
    /// The identity of what the walk saw at `path` as `meta`; `None` when it cannot be read.
    pub(super) fn id_of_meta(&self, path: &Path, meta: &Metadata) -> Option<FileId> {
        if self.inner.hooks.unreadable_identity {
            return None;
        }

        FileId::of_meta(path, meta)
    }

    /// The identity of what is at `path` right now; `None` when it cannot be read.
    pub(super) fn id_of_path(&self, path: &Path) -> Option<FileId> {
        if self.inner.hooks.unreadable_identity {
            return None;
        }

        FileId::of_path(path)
    }
}

impl super::held::Held {
    /// The identity of the folder this holds open (on Windows, of the path it is held by); `None` when it cannot be read.
    #[cfg(unix)]
    pub(super) fn identity(&self) -> Option<FolderId> {
        let meta = self.dir.metadata().ok()?;

        crate::listing::platform::identity(&meta).map(FolderId)
    }

    #[cfg(windows)]
    pub(super) fn identity(&self) -> Option<FolderId> {
        FolderId::of_path(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_file_is_itself_and_not_another() {
        let temp = TempDir::new().expect("temp dir");
        let (a, b) = (temp.path().join("a"), temp.path().join("b"));
        fs::write(&a, "a").expect("file");
        fs::write(&b, "a").expect("file");

        assert_eq!(FileId::of_path(&a), FileId::of_path(&a));
        assert_ne!(FileId::of_path(&a), FileId::of_path(&b));
        assert!(FileId::of_path(&temp.path().join("missing")).is_none());
    }

    #[test]
    fn a_hard_link_is_the_same_file_and_a_folder_has_its_own_identity() {
        let temp = TempDir::new().expect("temp dir");
        let (a, link, folder) = (
            temp.path().join("a"),
            temp.path().join("link"),
            temp.path().join("folder"),
        );
        fs::write(&a, "a").expect("file");
        fs::hard_link(&a, &link).expect("hard link");
        fs::create_dir(&folder).expect("folder");

        assert_eq!(FileId::of_path(&a), FileId::of_path(&link));
        assert_ne!(FileId::of_path(&a), FileId::of_path(&folder));
        assert!(FileId::of_path(&folder).is_some());
    }

    #[test]
    fn the_same_folder_by_another_spelling_has_one_identity() {
        let temp = TempDir::new().expect("temp dir");
        let folder = temp.path().join("folder");
        fs::create_dir(&folder).expect("folder");
        let respelled = folder.join("..").join("folder");

        assert_eq!(FileId::of_path(&folder), FileId::of_path(&respelled));
    }
}

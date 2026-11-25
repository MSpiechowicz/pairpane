//! What a file holds besides its bytes, times and permissions, carried to its copy: extended attributes on Linux and
//! macOS (POSIX ACLs are kept in two of them on Linux), alternate data streams on Windows.
//!
//! A copy keeps its source, so what the target cannot hold is left out quietly. A move deletes its source, so there
//! anything that should go with the file and could not be written keeps the source, and the item says what was not
//! kept (data loss audit, #68; #73). What stays behind on purpose:
//! - Linux `security.*` and `trusted.*` attributes (a security label belongs to the disk it is on and is given there;
//!   the trusted ones need privileges) and the other `system.*` ones;
//! - the Windows `Zone.Identifier` stream (where a file came from), dropped where the target holds no streams, as
//!   Explorer does;
//! - ACLs that are not extended attributes (macOS) and explicit Windows ACLs: the copy gets those of its new folder.

use std::fs::File;
use std::path::Path;

#[cfg(windows)]
use super::win_streams;

/// What a file had that its copy does not: the name of the first attribute or stream that could not be written.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct NotKept(pub(super) String);

/// Writes the extended attributes (or alternate streams) of `source` onto `copy`, the new file at `copy_path` made from
/// the file at `source_path`. `refuse` makes every write fail, for tests. Gives the first one that should have been kept
/// and was not.
pub(super) fn carry(
    source: &File,
    copy: &File,
    (source_path, copy_path): (&Path, &Path),
    refuse: bool,
) -> Result<(), NotKept> {
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
    {
        let _ = (source_path, copy_path);

        xattrs::carry(source, copy, refuse)
    }
    #[cfg(windows)]
    {
        let _ = (source, copy);

        win_streams::carry(source_path, copy_path, refuse)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        windows
    )))]
    {
        let _ = (source, copy, source_path, copy_path, refuse);

        Ok(())
    }
}

/// Whether the extended attribute `name` goes with a file to its copy (see the module's text). A pure rule, tested on
/// every system.
#[cfg_attr(
    windows,
    allow(dead_code, reason = "used where files have extended attributes; tested everywhere")
)]
pub(super) fn is_carried(name: &[u8], linux: bool) -> bool {
    if !linux {
        return true;
    }

    name.starts_with(b"user.") || name == b"system.posix_acl_access" || name == b"system.posix_acl_default"
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
mod xattrs {
    use std::ffi::CString;
    use std::fs::File;

    use rustix::fs::{XattrFlags, fgetxattr, flistxattr, fsetxattr};
    use rustix::io::Errno;

    use super::{NotKept, is_carried};

    /// The most bytes of names or of one value that are read; larger is refused (a file system's own limits are far
    /// below it: 64 KiB on Linux, about 128 KiB for a resource fork read this way is not usual either).
    const MOST: usize = 16 * 1024 * 1024;

    pub(super) fn carry(source: &File, copy: &File, refuse: bool) -> Result<(), NotKept> {
        let names = match read(|buffer| flistxattr(source, buffer)) {
            Ok(names) => names,
            // A file system without extended attributes has none to carry.
            Err(error) if error == Errno::NOTSUP || error == Errno::OPNOTSUPP => return Ok(()),
            Err(error) => return Err(NotKept(format!("its extended attributes ({error})"))),
        };

        let linux = cfg!(any(target_os = "linux", target_os = "android"));
        for name in names.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
            if !is_carried(name, linux) {
                continue;
            }

            let shown = String::from_utf8_lossy(name).into_owned();
            let Ok(c_name) = CString::new(name) else {
                return Err(NotKept(shown));
            };
            let value = match read(|buffer| fgetxattr(source, c_name.as_c_str(), buffer)) {
                Ok(value) => value,
                // Gone since the names were read.
                Err(Errno::NODATA) => continue,
                Err(_) => return Err(NotKept(shown)),
            };

            let written = if refuse {
                Err(Errno::NOTSUP)
            } else {
                fsetxattr(copy, c_name.as_c_str(), &value, XattrFlags::empty())
            };
            if written.is_err() {
                return Err(NotKept(shown));
            }
        }

        Ok(())
    }

    /// The bytes a call that fills a buffer gives, the buffer grown until they fit.
    fn read(mut call: impl FnMut(&mut [u8]) -> Result<usize, Errno>) -> Result<Vec<u8>, Errno> {
        let mut buffer = vec![0; 4096];
        loop {
            match call(&mut buffer) {
                Ok(length) => {
                    buffer.truncate(length);
                    return Ok(buffer);
                }
                Err(Errno::RANGE) if buffer.len() < MOST => buffer.resize((buffer.len() * 4).min(MOST), 0),
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_carried;

    #[test]
    fn linux_carries_user_attributes_and_acls_only() {
        assert!(is_carried(b"user.xdg.origin.url", true));
        assert!(is_carried(b"system.posix_acl_access", true));
        assert!(is_carried(b"system.posix_acl_default", true));
        assert!(!is_carried(b"security.selinux", true));
        assert!(!is_carried(b"security.capability", true));
        assert!(!is_carried(b"trusted.overlay.opaque", true));
        assert!(!is_carried(b"system.nfs4_acl", true));
    }

    #[test]
    fn macos_carries_every_attribute() {
        assert!(is_carried(b"com.apple.quarantine", false));
        assert!(is_carried(b"com.apple.metadata:_kMDItemUserTags", false));
    }
}

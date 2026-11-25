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
/// the file at `source_path`. `refuse` makes every write fail, for tests; `stopped` says the job was cancelled, which
/// ends the work early. Goes on past one that cannot be written (a copy takes all it can) and gives the first that
/// should have been kept and was not.
pub(super) fn carry(
    source: &File,
    copy: &File,
    (source_path, copy_path): (&Path, &Path),
    (refuse, stopped): (bool, &dyn Fn() -> bool),
) -> Result<(), NotKept> {
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
    {
        let _ = (source_path, copy_path, stopped);

        xattrs::carry(source, copy, refuse)
    }
    #[cfg(windows)]
    {
        win_streams::carry((source, source_path), (copy, copy_path), (refuse, stopped))
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        windows
    )))]
    {
        let _ = (source, copy, source_path, copy_path, refuse, stopped);

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
        // Kept by the system for the file where it is (which apps may open it, the protection of system files).
        return name != b"com.apple.macl" && name != b"com.apple.rootless";
    }

    name.starts_with(b"user.") || name == b"system.posix_acl_access" || name == b"system.posix_acl_default"
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
mod xattrs {
    use std::ffi::CString;
    use std::fs::File;

    use rustix::fs::{XattrFlags, fgetxattr, fgetxattr_query, flistxattr, flistxattr_query, fsetxattr};
    use rustix::io::Errno;

    use super::{NotKept, is_carried};

    /// The most bytes of the names, or of one value, that are read. Linux allows 64 KiB per value; a larger value (a big
    /// macOS resource fork) is not read, so a move of its file keeps the source.
    const MOST: usize = 16 * 1024 * 1024;

    pub(super) fn carry(source: &File, copy: &File, refuse: bool) -> Result<(), NotKept> {
        let names = match read(|| flistxattr_query(source), |buffer| flistxattr(source, buffer)) {
            Ok(names) => names,
            // A file system without extended attributes has none to carry.
            Err(error) if error == Errno::NOTSUP || error == Errno::OPNOTSUPP || error == Errno::NOSYS => return Ok(()),
            Err(error) => return Err(NotKept(format!("extended attributes ({error})"))),
        };

        let linux = cfg!(any(target_os = "linux", target_os = "android"));
        let mut first_missing = None;
        for name in names.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
            if !is_carried(name, linux) {
                continue;
            }

            let shown = format!("extended attribute {:?}", String::from_utf8_lossy(name));
            let Ok(c_name) = CString::new(name) else {
                first_missing.get_or_insert(NotKept(shown));
                continue;
            };
            let value = match read(
                || fgetxattr_query(source, c_name.as_c_str()),
                |buffer| fgetxattr(source, c_name.as_c_str(), buffer),
            ) {
                Ok(value) => value,
                // Gone since the names were read.
                Err(Errno::NODATA) => continue,
                Err(_) => {
                    first_missing.get_or_insert(NotKept(shown));
                    continue;
                }
            };

            // A clone (APFS) already has the attribute, and may already be read-only: nothing to write.
            let present = read(
                || fgetxattr_query(copy, c_name.as_c_str()),
                |buffer| fgetxattr(copy, c_name.as_c_str(), buffer),
            );
            let written = if refuse {
                Err(Errno::NOTSUP)
            } else if present.is_ok_and(|there| there == value) {
                Ok(())
            } else {
                fsetxattr(copy, c_name.as_c_str(), &value, XattrFlags::empty())
            };
            match written {
                Ok(()) => {}
                // On macOS the system keeps the provenance of a file to itself and refuses to let a process set it; it
                // gives the copy its own. Only that one is let go of: a refused quarantine flag keeps the source.
                Err(Errno::PERM) if is_system_set(name) => {}
                Err(_) => {
                    first_missing.get_or_insert(NotKept(shown));
                }
            }
        }

        first_missing.map_or(Ok(()), Err)
    }

    /// Whether the system sets the attribute `name` itself and refuses a process that tries (macOS).
    fn is_system_set(name: &[u8]) -> bool {
        cfg!(any(target_os = "macos", target_os = "ios")) && name == b"com.apple.provenance"
    }

    /// The whole of what `fill` reads, its size asked for first with `size`. Only a read of exactly that size is taken
    /// (macOS may read a resource fork in part without saying so); anything else asks again, a few times. Larger than
    /// `MOST` is refused.
    fn read(
        mut size: impl FnMut() -> Result<usize, Errno>,
        mut fill: impl FnMut(&mut [u8]) -> Result<usize, Errno>,
    ) -> Result<Vec<u8>, Errno> {
        for _ in 0..4 {
            let wanted = size()?;
            if wanted > MOST {
                return Err(Errno::TOOBIG);
            }

            let mut buffer = vec![0; wanted + 1];
            match fill(&mut buffer) {
                // Exactly the size asked for, or the size asked for again (it changed between the two calls): whole.
                Ok(length) if length == wanted || (length < wanted && size()? == length) => {
                    buffer.truncate(length);
                    return Ok(buffer);
                }
                // Grown since its size was read, or cut short.
                Ok(_) | Err(Errno::RANGE) => {}
                Err(error) => return Err(error),
            }
        }

        Err(Errno::AGAIN)
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
    fn macos_carries_every_attribute_but_those_the_system_keeps() {
        assert!(is_carried(b"com.apple.quarantine", false));
        assert!(is_carried(b"com.apple.metadata:_kMDItemUserTags", false));
        assert!(is_carried(b"user.pairpane.test", false));
        assert!(!is_carried(b"com.apple.macl", false));
        assert!(!is_carried(b"com.apple.rootless", false));
    }
}

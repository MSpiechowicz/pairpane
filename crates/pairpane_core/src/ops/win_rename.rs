//! A rename on Windows that the system refuses when something is at the new name (#77).
//!
//! This is one of the few modules of the workspace that may use `unsafe` (the lint is denied everywhere else): the
//! standard library's rename replaces what is at the new name, and only `MoveFileExW` called without
//! `MOVEFILE_REPLACE_EXISTING` (nor `MOVEFILE_COPY_ALLOWED`) refuses it in the same step, so nothing can appear at the new
//! name between a check and the rename. It moves a link itself, never what it leads to, and fails between two disks
//! (`ERROR_NOT_SAME_DEVICE`), as a rename does. Where a path is too long for the plain form, it is given again in the
//! `\\?\` form, which the system takes as it is (see [`verbatim`], safe and tested on every system).
#![allow(unsafe_code, reason = "the one call of MoveFileExW; see `move_noreplace`")]
#![deny(clippy::undocumented_unsafe_blocks)]

/// `path` (absolute) in the `\\?\` form: `C:\a` as `\\?\C:\a`, `\\host\share\a` as `\\?\UNC\host\share\a`, with every
/// `/` as `\`; one already in that form as it is. `None` for a path that is not absolute in one of those forms.
#[cfg_attr(not(windows), allow(dead_code, reason = "used on Windows; tested everywhere"))]
pub(super) fn verbatim(path: &str) -> Option<String> {
    if path.starts_with(r"\\?\") {
        return Some(path.to_owned());
    }

    let path = path.replace('/', "\\");
    // The `\\?\` form takes `.` and `..` as names, not as steps: such a path is not given in it.
    if path.split('\\').any(|part| part == "." || part == "..") {
        return None;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return Some(format!(r"\\?\{path}"));
    }
    if let Some(rest) = path.strip_prefix(r"\\")
        && !rest.starts_with(['\\', '?', '.'])
    {
        return Some(format!(r"\\?\UNC\{rest}"));
    }

    None
}

#[cfg(windows)]
pub(super) use imp::move_noreplace;

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    use super::verbatim;

    /// Windows `ERROR_PATH_NOT_FOUND` and `ERROR_FILENAME_EXCED_RANGE`: what a path too long for its plain form gives.
    const TOO_LONG: [i32; 2] = [3, 206];

    /// Moves `from` to `to` unless something is at `to`: then `AlreadyExists` (`ERROR_ALREADY_EXISTS` or
    /// `ERROR_FILE_EXISTS`, which the standard library maps so), and nothing changes.
    pub(in super::super) fn move_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        // MoveFileExW takes another name of the same file (a hard link) at `to` for no conflict and moves onto it; that
        // is something at `to` all the same. A file that appears after this look is refused by the system itself.
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }

        match call(from.as_os_str(), to.as_os_str()) {
            Err(error) if error.raw_os_error().is_some_and(|code| TOO_LONG.contains(&code)) => {
                let (Some(from), Some(to)) = (from.to_str().and_then(verbatim), to.to_str().and_then(verbatim)) else {
                    return Err(error);
                };

                call(OsStr::new(&from), OsStr::new(&to))
            }
            result => result,
        }
    }

    fn call(from: &OsStr, to: &OsStr) -> io::Result<()> {
        let from: Vec<u16> = from.encode_wide().collect();
        let to: Vec<u16> = to.encode_wide().collect();
        // A NUL inside a path would end it early, naming another item.
        if from.contains(&0) || to.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let from: Vec<u16> = from.into_iter().chain(Some(0)).collect();
        let to: Vec<u16> = to.into_iter().chain(Some(0)).collect();

        // SAFETY: both are NUL-terminated UTF-16 paths that outlive the call; flags 0 asks for a plain move that
        // replaces nothing and copies nothing.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::verbatim;

    #[test]
    fn a_path_is_given_in_the_form_the_system_takes_as_it_is() {
        assert_eq!(verbatim(r"C:\data\a.txt").as_deref(), Some(r"\\?\C:\data\a.txt"));
        assert_eq!(verbatim("C:/data/a.txt").as_deref(), Some(r"\\?\C:\data\a.txt"));
        assert_eq!(
            verbatim(r"\\host\share\a.txt").as_deref(),
            Some(r"\\?\UNC\host\share\a.txt")
        );
        assert_eq!(verbatim(r"\\?\C:\data").as_deref(), Some(r"\\?\C:\data"));
        assert_eq!(verbatim(r"data\a.txt"), None);
        assert_eq!(verbatim(r"\\.\pipe\x"), None);
        assert_eq!(verbatim(r"C:data"), None);
        assert_eq!(verbatim(r"C:\data\..\other"), None);
        assert_eq!(verbatim(r"C:\data\.\a"), None);
    }
}

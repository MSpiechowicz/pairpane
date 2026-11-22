//! The origin marker of an archive that came from elsewhere, and its copy onto what is extracted from it, so a file taken
//! out of a downloaded archive is still known as downloaded (and the system warns before it is run).
//!
//! - Windows: the alternate data stream `Zone.Identifier` of the archive (the "mark of the web"), copied byte for byte to
//!   a stream of the same name on each file.
//! - macOS: the extended attribute `com.apple.quarantine`, copied to each file.
//! - Elsewhere there is no such marker and nothing is done.
//!
//! The marker is read once per job ([`Origin::of`]) and put on each file after it is in place ([`Origin::apply`]). It is
//! best effort: an archive without a marker, a marker that cannot be read, and a file that does not take one (a volume
//! that has no streams or attributes) all leave the job as it is, and nothing is reported.

use std::path::Path;

/// The most bytes of a marker that are copied; a real one is a few hundred.
const MAX_MARKER: usize = 4096;

/// The marker of an archive: what is put on each file extracted from it, or nothing.
#[derive(Debug, Clone, Default)]
pub(super) struct Origin(Option<Vec<u8>>);

impl Origin {
    /// The marker of the archive file at `archive`; nothing when it has none or the platform has none.
    pub(super) fn of(archive: &Path) -> Self {
        Self(platform::read(archive).filter(|marker| !marker.is_empty() && marker.len() <= MAX_MARKER))
    }

    /// Puts the marker on the file at `file`, which was just written. Never fails.
    pub(super) fn apply(&self, file: &Path) {
        if let Some(marker) = &self.0 {
            platform::write(file, marker);
        }
    }

    /// The marker, when there is one.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::Read;
    use std::path::Path;

    use super::MAX_MARKER;

    /// The name of the stream of the file at `path`.
    fn stream(path: &Path) -> OsString {
        let mut name = path.as_os_str().to_owned();
        name.push(":Zone.Identifier");

        name
    }

    pub(super) fn read(archive: &Path) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(stream(archive))
            .ok()?
            .take(MAX_MARKER as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;

        Some(bytes)
    }

    pub(super) fn write(file: &Path, marker: &[u8]) {
        let _ = fs::write(stream(file), marker);
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::Path;

    use super::MAX_MARKER;
    use super::attribute::{read_attribute, write_attribute};

    const QUARANTINE: &str = "com.apple.quarantine";

    pub(super) fn read(archive: &Path) -> Option<Vec<u8>> {
        read_attribute(archive, QUARANTINE, MAX_MARKER)
    }

    pub(super) fn write(file: &Path, marker: &[u8]) {
        write_attribute(file, QUARANTINE, marker);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use std::path::Path;

    pub(super) fn read(_archive: &Path) -> Option<Vec<u8>> {
        None
    }

    pub(super) fn write(_file: &Path, _marker: &[u8]) {}
}

/// Extended attributes by path, without following a link in the last part (macOS; Linux in the tests, where the same calls
/// can be tried on a `user.` attribute).
#[cfg(any(target_os = "macos", all(target_os = "linux", test)))]
pub(super) mod attribute {
    use std::path::Path;

    use rustix::fs::{XattrFlags, lgetxattr, lsetxattr};

    /// The attribute `name` of `path`, at most `limit` bytes of it; `None` when it is not there or is longer.
    pub(in crate::ops) fn read_attribute(path: &Path, name: &str, limit: usize) -> Option<Vec<u8>> {
        let mut buffer = vec![0_u8; limit];
        let length = lgetxattr(path, name, &mut buffer[..]).ok()?;
        buffer.truncate(length);

        Some(buffer)
    }

    /// Sets the attribute `name` of `path` to `value`; a failure is ignored.
    pub(in crate::ops) fn write_attribute(path: &Path, name: &str, value: &[u8]) {
        let _ = lsetxattr(path, name, value, XattrFlags::empty());
    }
}

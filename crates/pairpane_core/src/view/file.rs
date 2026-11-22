//! The open file of a viewer session: reads at offsets through the handle, never through a shared position.

use std::fs::File;
use std::io;

use crate::archive::ReadAt;

use super::{MAX_READ, refusal};

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

/// A file opened for reading. Many threads may read it at once (`read_at` takes `&self` and uses no shared
/// position), so a session shares one through an `Arc`.
#[derive(Debug)]
pub struct ViewFile {
    file: File,
}

impl ViewFile {
    pub(super) fn new(file: File) -> Self {
        Self { file }
    }

    /// The size now, asked of the system each time (the file may grow or shrink while it is open).
    pub fn size(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    /// Up to `len` bytes at `offset` (at most [`MAX_READ`]; more is refused). Fewer come back at the end of the file,
    /// none at or past it.
    pub fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        if len > MAX_READ {
            return Err(refusal(io::ErrorKind::InvalidInput, "the read is too long"));
        }

        self.read_chunk(offset, len)
    }

    #[cfg(unix)]
    fn read_some(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        FileExt::read_at(&self.file, buffer, offset)
    }

    #[cfg(windows)]
    fn read_some(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        self.file.seek_read(buffer, offset)
    }
}

impl ReadAt for ViewFile {
    fn size(&self) -> io::Result<u64> {
        ViewFile::size(self)
    }

    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        self.read_some(buffer, offset)
    }
}

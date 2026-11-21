//! The open file of a viewer session: reads at offsets through the handle, never through a shared position.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

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

        let left = self.size()?.saturating_sub(offset);
        let wanted = usize::try_from(left).map_or(len, |left| left.min(len));
        let mut bytes = vec![0; wanted];
        let mut filled = 0;

        while filled < wanted {
            let at = offset.saturating_add(filled as u64);

            match self.read_some(&mut bytes[filled..], at) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }

        bytes.truncate(filled);

        Ok(bytes)
    }

    /// A reader (and seeker) over the file, for the libraries that take one. It keeps its own position.
    pub(super) fn reader(&self) -> FileReader<'_> {
        FileReader {
            file: self,
            position: 0,
        }
    }

    #[cfg(unix)]
    fn read_some(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        self.file.read_at(buffer, offset)
    }

    #[cfg(windows)]
    fn read_some(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        self.file.seek_read(buffer, offset)
    }
}

/// `Read` and `Seek` over a [`ViewFile`]; each reader has a position of its own.
pub(super) struct FileReader<'a> {
    file: &'a ViewFile,
    position: u64,
}

impl FileReader<'_> {
    /// The size of the file now.
    pub(super) fn size(&self) -> io::Result<u64> {
        self.file.size()
    }
}

impl Read for FileReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read_some(buffer, self.position)?;
        self.position = self.position.saturating_add(count as u64);

        Ok(count)
    }
}

impl Seek for FileReader<'_> {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let before_start = || refusal(io::ErrorKind::InvalidInput, "the position is before the start");

        let position = match target {
            SeekFrom::Start(at) => at,
            SeekFrom::Current(by) => self.position.checked_add_signed(by).ok_or_else(before_start)?,
            SeekFrom::End(by) => self.file.size()?.checked_add_signed(by).ok_or_else(before_start)?,
        };
        self.position = position;

        Ok(position)
    }
}

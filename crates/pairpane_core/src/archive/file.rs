//! Reading an archive file at offsets, through the handle and never through a shared position, so one open file serves
//! many readers at once.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use crate::view::refusal;

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

/// A file that can be read at an offset by many threads at once.
pub trait ReadAt {
    /// The size now, asked of the system each time (the file may grow or shrink while it is open).
    fn size(&self) -> io::Result<u64>;

    /// Reads up to `buffer.len()` bytes at `offset`; 0 at or past the end.
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize>;

    /// Up to `len` bytes at `offset`; fewer at the end of the file, none at or past it. `len` is the caller's to bound.
    fn read_chunk(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let left = self.size()?.saturating_sub(offset);
        let wanted = usize::try_from(left).map_or(len, |left| left.min(len));
        let mut bytes = vec![0; wanted];
        let mut filled = 0;

        while filled < wanted {
            match self.read_at(&mut bytes[filled..], offset.saturating_add(filled as u64)) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }

        bytes.truncate(filled);

        Ok(bytes)
    }
}

impl ReadAt for File {
    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    #[cfg(unix)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        FileExt::read_at(self, buffer, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        self.seek_read(buffer, offset)
    }
}

/// `Read` and `Seek` over a [`ReadAt`]; each reader has a position of its own.
pub struct FileReader<'a, F: ReadAt + ?Sized> {
    file: &'a F,
    position: u64,
}

impl<'a, F: ReadAt + ?Sized> FileReader<'a, F> {
    /// A reader at the start of `file`.
    pub fn new(file: &'a F) -> Self {
        Self { file, position: 0 }
    }

    /// The size of the file now.
    pub fn size(&self) -> io::Result<u64> {
        self.file.size()
    }
}

impl<F: ReadAt + ?Sized> Read for FileReader<'_, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read_at(buffer, self.position)?;
        self.position = self.position.saturating_add(count as u64);

        Ok(count)
    }
}

impl<F: ReadAt + ?Sized> Seek for FileReader<'_, F> {
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

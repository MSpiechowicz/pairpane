//! Moving the bytes of one file: the quickest way the system offers, in chunks small enough to pause or cancel
//! between them.
//!
//! - Linux: a reflink (`FICLONE`, instant on btrfs, XFS and bcachefs), else `copy_file_range` in 8 MiB calls,
//!   which stays in the kernel (and lets an NFS server copy by itself).
//! - macOS: `fclonefileat` (instant on APFS).
//! - Everywhere, and whenever the above is refused: a loop over a 1 MiB buffer.
//!
//! Windows always uses the loop: the system's own copy cannot report progress or stop in the middle of a file.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use super::open;
use super::transfer::{Run, Stop};

/// Bytes asked of the kernel per call, so pausing and cancelling take effect within this many.
#[cfg(target_os = "linux")]
const KERNEL_CHUNK: usize = 8 * 1024 * 1024;

/// Size of the buffer of the loop.
const BUFFER_CHUNK: usize = 1024 * 1024;

/// What [`create`] made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    /// The new file holds all the data already: the system cloned it.
    Cloned,
    /// The new file is empty and waits for the data.
    Empty,
}

/// Creates the new file `temp` for the copy of `source`: as a clone where the system can, else empty. Fails
/// when anything is at `temp`.
///
/// `clone` is whether to try a clone at all.
pub(super) fn create(source: &File, temp: &Path, clone: bool) -> io::Result<(File, Start)> {
    if clone && let Some(clone) = clone_new(source, temp)? {
        return Ok((clone, Start::Cloned));
    }

    let copy = open::create_new(temp)?;

    if clone && clone_into(&copy, source) {
        return Ok((copy, Start::Cloned));
    }

    Ok((copy, Start::Empty))
}

/// macOS: makes `temp` as a clone of `source` (it fails when `temp` exists, and does not follow a link there).
/// `None` when the filesystem cannot clone.
#[cfg(target_os = "macos")]
fn clone_new(source: &File, temp: &Path) -> io::Result<Option<File>> {
    use rustix::fs::{CWD, CloneFlags, fclonefileat};
    use rustix::io::Errno;

    match fclonefileat(source, CWD, temp, CloneFlags::NOFOLLOW) {
        Ok(()) => open::existing(temp).map(Some).inspect_err(|_| {
            let _ = std::fs::remove_file(temp);
        }),
        Err(Errno::EXIST) => Err(io::Error::from(Errno::EXIST)),
        Err(_) => Ok(None),
    }
}

#[cfg(not(target_os = "macos"))]
fn clone_new(_source: &File, _temp: &Path) -> io::Result<Option<File>> {
    Ok(None)
}

/// Linux: makes the empty `copy` share the data of `source` (a reflink). `false` when the filesystem cannot.
#[cfg(target_os = "linux")]
pub(super) fn clone_into(copy: &File, source: &File) -> bool {
    rustix::fs::ioctl_ficlone(copy, source).is_ok()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn clone_into(_copy: &File, _source: &File) -> bool {
    false
}

/// Linux: asks the kernel to copy up to [`KERNEL_CHUNK`] bytes from `source` to `copy`, both at their current
/// offsets. `None` when it cannot do that for these two files (another filesystem, or one that does not
/// support it).
#[cfg(target_os = "linux")]
fn kernel_chunk(source: &File, copy: &File) -> io::Result<Option<usize>> {
    use rustix::io::Errno;

    loop {
        match rustix::fs::copy_file_range(source, None, copy, None, KERNEL_CHUNK) {
            Ok(copied) => return Ok(Some(copied)),
            Err(Errno::INTR) => {}
            Err(Errno::XDEV | Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP | Errno::PERM) => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn kernel_chunk(_source: &File, _copy: &File) -> io::Result<Option<usize>> {
    Ok(None)
}

impl Run<'_> {
    /// Copies the rest of `source` (about `size` bytes) into `copy`, counting the bytes as they go and stopping
    /// between chunks for a pause or a cancel.
    pub(super) fn fill(&mut self, source: &mut File, copy: &mut File, size: u64) -> Result<(), Stop> {
        let mut kernel = !self.inner.hooks.buffered_only;
        let mut copied = 0_u64;

        loop {
            self.checkpoint()?;

            // A kernel copy that reports the end of a file that is not at its end has given up silently (some
            // special filesystems do): the loop takes over, and finds the real end.
            let mut moved = None;
            if kernel {
                moved = kernel_chunk(source, copy)?.filter(|bytes| *bytes > 0 || copied >= size);
                kernel = moved.is_some();
            }
            let moved = match moved {
                Some(bytes) => bytes,
                None => self.buffered_chunk(source, copy)?,
            };

            if moved == 0 {
                return Ok(());
            }

            copied += moved as u64;
            self.add_bytes(moved as u64);
        }
    }

    /// Moves one buffer of bytes; 0 at the end of `source`.
    fn buffered_chunk(&mut self, source: &mut File, copy: &mut File) -> io::Result<usize> {
        self.buffer.resize(BUFFER_CHUNK, 0);

        let read = loop {
            match source.read(&mut self.buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => break result?,
            }
        };
        copy.write_all(&self.buffer[..read])?;

        Ok(read)
    }
}

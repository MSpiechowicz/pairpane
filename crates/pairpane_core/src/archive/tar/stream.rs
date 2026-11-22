//! The byte streams a tar is read from: the plain file, and the decoders of the compressions a tar comes in. A stream is
//! read forward only and can step over bytes (`skip`): a plain file by a seek, a decoder by decoding into a sink.

use std::cell::Cell;
use std::io::{self, BufReader, Read, Seek};
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::read::GzDecoder;

use crate::archive::entries::read_within;
use crate::archive::error::{check, unreadable, unsupported};
use crate::archive::file::{FileReader, ReadAt};
use crate::model::ArchiveFormat;

/// A tar to read headers from, and to step over data of.
pub trait Stream: Read {
    /// Moves on by `count` bytes without keeping any of them.
    fn skip(&mut self, count: u64) -> io::Result<()>;
}

impl<S: Stream + ?Sized> Stream for Box<S> {
    fn skip(&mut self, count: u64) -> io::Result<()> {
        (**self).skip(count)
    }
}

/// Reads at most `left` bytes of `inner`, then reports the end of the data and remembers it was cut. Once `cancel`
/// is raised a read fails with `Interrupted`.
pub struct Capped<'a, R> {
    pub inner: R,
    pub left: u64,
    pub reached: &'a Cell<bool>,
    pub cancel: &'a AtomicBool,
}

impl<R: Read> Read for Capped<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check(self.cancel)?;

        if self.left == 0 {
            self.reached.set(true);
            return Ok(0);
        }

        let count = read_within(&mut self.inner, self.left, buffer)?;
        self.left -= count as u64;

        Ok(count)
    }
}

impl<R: Read> Stream for Capped<'_, R> {
    fn skip(&mut self, mut count: u64) -> io::Result<()> {
        let mut sink = [0; 32 * 1024];

        while count > 0 {
            let room = usize::try_from(count).map_or(sink.len(), |count| count.min(sink.len()));

            // An `Interrupted` error is the answer to a raised flag, unless the flag is down (a signal): then read again.
            match self.read(&mut sink[..room]) {
                Ok(0) => return Err(unreadable()),
                Ok(read) => count -= read as u64,
                Err(error) if error.kind() == io::ErrorKind::Interrupted && !self.cancel.load(Ordering::Relaxed) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }
}

impl<F: ReadAt + ?Sized> Stream for BufReader<FileReader<'_, F>> {
    fn skip(&mut self, count: u64) -> io::Result<()> {
        self.seek_relative(i64::try_from(count).map_err(|_| unreadable())?)?;

        // A seek past the end succeeds: data that the file does not hold is the file being cut short.
        if self.stream_position()? > self.get_ref().size()? {
            return Err(unreadable());
        }

        Ok(())
    }
}

/// The stream of the tar in `file`, in the compression `format` names. A decoded stream ends after `scan` bytes and
/// then sets `reached`, so a walk can tell a limit from the end of the data.
///
/// Every format that is a tar inside a compression is opened here; those that are not yet decoded say `Unsupported`.
pub fn open<'a>(
    format: ArchiveFormat,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    reached: &'a Cell<bool>,
    scan: u64,
) -> io::Result<Box<dyn Stream + 'a>> {
    match format {
        ArchiveFormat::Tar => Ok(Box::new(BufReader::new(FileReader::new(file)))),
        ArchiveFormat::TarGz => Ok(capped(
            GzDecoder::new(BufReader::new(FileReader::new(file))),
            scan,
            reached,
            cancel,
        )),
        _ => Err(unsupported("this kind of archive cannot be read yet")),
    }
}

/// `inner` as a stream that ends after `scan` bytes, for the decoders.
pub fn capped<'a, R: Read + 'a>(
    inner: R,
    scan: u64,
    reached: &'a Cell<bool>,
    cancel: &'a AtomicBool,
) -> Box<dyn Stream + 'a> {
    Box::new(Capped {
        inner,
        left: scan,
        reached,
        cancel,
    })
}

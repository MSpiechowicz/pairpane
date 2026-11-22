//! The byte streams a tar is read from: the plain file, and the decoders of the compressions a tar comes in. A stream is
//! read forward only and can step over bytes (`skip`): a plain file by a seek, a decoder by decoding into a sink.

use std::cell::Cell;
use std::io::{self, BufRead, BufReader, Read, Seek};
use std::sync::atomic::{AtomicBool, Ordering};

use bzip2::read::MultiBzDecoder;
use flate2::bufread::GzDecoder;
use lzma_rust2::XzReader;
use ruzstd::decoding::{FrameDecoder, StreamingDecoder};

use crate::archive::entries::read_within;
use crate::archive::error::{check, data_after_end, unreadable, unsupported};
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

/// The most memory, in KiB, the LZMA2 decoder of one XZ block may take (its dictionary and model): 256 MiB, past the
/// 64 MiB of `xz -9` and its 192 MiB with `-e`. A tar that needs more cannot be read.
const XZ_MEMORY_KB: u32 = 256 * 1024;

/// A Zstandard stream of one or more frames back to back (what `cat a.zst b.zst` makes), read as one: the decoder of
/// `ruzstd` stops at the end of a frame, so the next frame is started when the file has more. A frame's window is held to
/// the decoder's own default of 128 MiB.
struct ZstdFrames<R: BufRead> {
    /// `None` once the last frame is read.
    frame: Option<StreamingDecoder<R, FrameDecoder>>,
}

impl<R: BufRead> ZstdFrames<R> {
    fn new(source: R) -> io::Result<Self> {
        let frame = StreamingDecoder::new(source).map_err(|_| unreadable())?;

        Ok(Self { frame: Some(frame) })
    }
}

impl<R: BufRead> Read for ZstdFrames<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            let Some(frame) = self.frame.as_mut() else {
                return Ok(0);
            };

            let count = frame.read(buffer)?;

            if count > 0 || buffer.is_empty() {
                return Ok(count);
            }

            // The frame is done: another one follows when the file has bytes left.
            let (mut source, decoder) = self
                .frame
                .take()
                .map(StreamingDecoder::into_parts)
                .ok_or_else(unreadable)?;

            if source.fill_buf()?.is_empty() {
                return Ok(0);
            }

            self.frame = Some(StreamingDecoder::new_with_decoder(source, decoder).map_err(|_| unreadable())?);
        }
    }
}

/// The first member of a gzip file that fails at its end when the file holds more bytes: what a reader that lists the
/// tar can leave unread, a rewrite would drop.
struct GzWhole<R: BufRead> {
    decoder: GzDecoder<R>,
}

impl<R: BufRead> Read for GzWhole<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.decoder.read(buffer)?;

        if count == 0 && !buffer.is_empty() && !self.decoder.get_mut().fill_buf()?.is_empty() {
            return Err(data_after_end());
        }

        Ok(count)
    }
}

/// The stream of the tar in `file`, in the compression `format` names. A decoded stream ends after `scan` bytes and
/// then sets `reached`, so a walk can tell a limit from the end of the data. An XZ, Zstandard or bzip2 file of several
/// streams in a row is read through all of them (a gzip file is read to the end of its first member).
///
/// Every format that is a tar inside a compression is opened here; any other format is an error.
pub fn open<'a>(
    format: ArchiveFormat,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    reached: &'a Cell<bool>,
    scan: u64,
) -> io::Result<Box<dyn Stream + 'a>> {
    open_as(format, file, cancel, reached, scan, false)
}

/// [`open`] for a copy of the whole stream: a gzip file with bytes after its first member fails at the end of that member
/// with [`data_after_end`], where [`open`] ends there (the other compressions read through every stream already).
pub fn open_whole<'a>(
    format: ArchiveFormat,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    reached: &'a Cell<bool>,
    scan: u64,
) -> io::Result<Box<dyn Stream + 'a>> {
    open_as(format, file, cancel, reached, scan, true)
}

fn open_as<'a>(
    format: ArchiveFormat,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    reached: &'a Cell<bool>,
    scan: u64,
    whole: bool,
) -> io::Result<Box<dyn Stream + 'a>> {
    match format {
        ArchiveFormat::Tar => Ok(Box::new(BufReader::new(FileReader::new(file)))),
        ArchiveFormat::TarGz => {
            let decoder = GzDecoder::new(BufReader::new(FileReader::new(file)));

            if whole {
                Ok(capped(GzWhole { decoder }, scan, reached, cancel))
            } else {
                Ok(capped(decoder, scan, reached, cancel))
            }
        }
        ArchiveFormat::TarXz => Ok(capped(
            XzReader::new_mem_limit(BufReader::new(FileReader::new(file)), true, XZ_MEMORY_KB),
            scan,
            reached,
            cancel,
        )),
        ArchiveFormat::TarZst => Ok(capped(
            ZstdFrames::new(BufReader::new(FileReader::new(file)))?,
            scan,
            reached,
            cancel,
        )),
        ArchiveFormat::TarBz2 => Ok(capped(
            MultiBzDecoder::new(BufReader::new(FileReader::new(file))),
            scan,
            reached,
            cancel,
        )),
        ArchiveFormat::Zip | ArchiveFormat::Gz | ArchiveFormat::SevenZ | ArchiveFormat::Rar => {
            Err(unsupported("this is not a tar"))
        }
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

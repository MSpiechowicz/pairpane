//! The data of a zip entry on its way into the archive: counted as it is written, and deflated with its CRC taken.

use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;

use crc32fast::Hasher;
use flate2::Compression;
use flate2::write::DeflateEncoder;

use super::pump;

/// Counts what is written through it.
pub(super) struct Counting<W> {
    pub(super) inner: W,
    pub(super) written: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buffer)?;
        self.written += count as u64;

        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Copies `reader` into `sink`, deflating it at `deflate` (or as it is), and counts its CRC and size.
pub(super) fn deflated(
    reader: &mut dyn Read,
    sink: &mut dyn Write,
    deflate: Option<u32>,
    cancel: &AtomicBool,
    crc: &mut Hasher,
) -> io::Result<u64> {
    let Some(level) = deflate else {
        return pump(reader, sink, cancel, |chunk| crc.update(chunk));
    };

    let mut encoder = DeflateEncoder::new(sink, Compression::new(level));
    let size = pump(reader, &mut encoder, cancel, |chunk| crc.update(chunk))?;
    encoder.finish()?;

    Ok(size)
}

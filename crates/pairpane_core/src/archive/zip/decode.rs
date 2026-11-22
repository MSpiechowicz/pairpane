//! The compression methods of a zip entry, and the checksum that the whole of an entry's content must match.

use std::io::{self, BufRead, BufReader, Read};

use bzip2::bufread::BzDecoder;
use flate2::bufread::DeflateDecoder;
use lzma_rust2::LzmaReader;
use ruzstd::decoding::StreamingDecoder;

use crate::archive::error::{unreadable, unsupported};
use crate::view::refusal;

/// The most memory a dictionary of an LZMA entry may need; an entry that asks for more is not decoded.
const MAX_LZMA_DICTIONARY: u32 = 256 * 1024 * 1024;

/// A compression method this code decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Stored,
    Deflate,
    Bzip2,
    Lzma,
    Zstd,
}

impl Method {
    /// The method for a code of the zip format; any other (deflate64, PPMd, ...) is `Unsupported`.
    pub fn of(code: u16) -> io::Result<Self> {
        match code {
            0 => Ok(Self::Stored),
            8 => Ok(Self::Deflate),
            12 => Ok(Self::Bzip2),
            14 => Ok(Self::Lzma),
            93 => Ok(Self::Zstd),
            _ => Err(unsupported("this compression method is not supported")),
        }
    }
}

/// A decoder of `method` reading `source`, for an entry whose content is `size` bytes.
pub fn decoder<'a, R: BufRead + 'a>(method: Method, mut source: R, size: u64) -> io::Result<Box<dyn Read + 'a>> {
    Ok(match method {
        Method::Stored => Box::new(source),
        Method::Deflate => Box::new(DeflateDecoder::new(source)),
        Method::Bzip2 => Box::new(BzDecoder::new(source)),
        Method::Zstd => Box::new(StreamingDecoder::new(source).map_err(|_| unreadable())?),
        Method::Lzma => {
            // Two bytes of version, two of the length of the properties (5), then the properties: lc/lp/pb and the dictionary.
            let mut head = [0; 9];
            source.read_exact(&mut head).map_err(|_| unreadable())?;

            if head[2..4] != [5, 0] {
                return Err(unreadable());
            }

            let dictionary = u32::from_le_bytes([head[5], head[6], head[7], head[8]]);

            if dictionary > MAX_LZMA_DICTIONARY {
                return Err(unsupported("this entry needs more memory to decode than is allowed"));
            }

            Box::new(LzmaReader::new_with_props(source, size, head[4], dictionary, None).map_err(|_| unreadable())?)
        }
    })
}

/// A buffered reader over `source`, for the decoders that need one.
pub fn buffered<R: Read>(source: R) -> BufReader<R> {
    BufReader::with_capacity(64 * 1024, source)
}

/// Reads `inner` and, when it ends, compares the CRC-32 of everything read with `expected` (not at all when that is
/// `None`, as for AE-2 entries, which keep no CRC).
pub struct Checked<R> {
    inner: R,
    hasher: crc32fast::Hasher,
    expected: Option<u32>,
    done: bool,
}

impl<R> Checked<R> {
    pub fn new(inner: R, expected: Option<u32>) -> Self {
        Self {
            inner,
            hasher: crc32fast::Hasher::new(),
            expected,
            done: false,
        }
    }
}

impl<R: Read> Read for Checked<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.done {
            return Ok(0);
        }

        let count = self.inner.read(buffer)?;

        if count > 0 {
            self.hasher.update(&buffer[..count]);

            return Ok(count);
        }

        // A mismatch is an error on every further read too, never an end.
        if self.expected.is_some_and(|crc| crc != self.hasher.clone().finalize()) {
            return Err(refusal(
                io::ErrorKind::InvalidData,
                "the checksum of an entry does not match its data",
            ));
        }

        self.done = true;

        Ok(0)
    }
}

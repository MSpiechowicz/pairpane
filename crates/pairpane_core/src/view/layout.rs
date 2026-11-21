//! How the text of a file lies in its bytes: where a character can be cut, where a line ends, what ends it.

use std::io;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use memchr::memchr_iter;

use super::file::ViewFile;
use super::{MAX_SNAP, encoding};

/// The unit a newline is made of: a byte (UTF-8 and every legacy encoding, whose multi-byte characters never hold
/// the byte `0A`), or two bytes of UTF-16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Byte,
    Utf16Le,
    Utf16Be,
}

/// The layout of the text of one file read as one encoding.
#[derive(Debug, Clone, Copy)]
pub(super) struct Layout {
    pub(super) encoding: &'static Encoding,
    unit: Unit,
    /// Where the text starts: the length of the byte order mark that belongs to the encoding.
    pub(super) floor: u64,
}

impl Layout {
    pub(super) fn new(file: &ViewFile, encoding: &'static Encoding) -> io::Result<Self> {
        let unit = if encoding == UTF_16LE {
            Unit::Utf16Le
        } else if encoding == UTF_16BE {
            Unit::Utf16Be
        } else {
            Unit::Byte
        };

        let head = file.read_at(0, 3)?;

        Ok(Self {
            encoding,
            unit,
            floor: encoding::bom_len(&head, encoding) as u64,
        })
    }

    /// The bytes of one line ending.
    pub(super) fn width(&self) -> usize {
        if self.unit == Unit::Byte { 1 } else { 2 }
    }

    /// The indexes in `buf` (which starts at byte `base` of the file) where a line ending starts.
    pub(super) fn newlines<'a>(&self, buf: &'a [u8], base: u64) -> impl DoubleEndedIterator<Item = usize> + 'a {
        let unit = self.unit;

        memchr_iter(b'\n', buf)
            .filter(move |&at| match unit {
                Unit::Byte => true,
                Unit::Utf16Le => (base + at as u64).is_multiple_of(2) && buf.get(at + 1) == Some(&0),
                Unit::Utf16Be => !(base + at as u64).is_multiple_of(2) && at > 0 && buf[at - 1] == 0,
            })
            .map(move |at| if unit == Unit::Utf16Be { at - 1 } else { at })
    }

    /// The length of what `line` ends with that is not text: a carriage return (before a line feed).
    pub(super) fn carriage_return(&self, line: &[u8]) -> usize {
        let tail: &[u8] = match self.unit {
            Unit::Byte => b"\r",
            Unit::Utf16Le => b"\r\0",
            Unit::Utf16Be => b"\0\r",
        };

        if line.ends_with(tail) { tail.len() } else { 0 }
    }

    /// The index at or before `at` (and after `from`) where `buf` can be cut without splitting a character. `at` may
    /// be the end of the buffer, where an unfinished character is left for the next window. Encodings with multi-byte
    /// characters that are neither UTF-8 nor UTF-16 are cut where asked.
    pub(super) fn cut(&self, buf: &[u8], from: usize, at: usize) -> usize {
        if self.unit != Unit::Byte {
            return self.cut_utf16(buf, from, at);
        }

        if self.encoding != UTF_8 {
            return at;
        }

        // Inside the buffer: before the character the byte at `at` belongs to.
        if at < buf.len() {
            let mut cut = at;
            while cut > from && buf[cut] & 0xC0 == 0x80 {
                cut -= 1;
            }

            return cut;
        }

        // At the end: before a character that is not all there.
        for index in (at.saturating_sub(3).max(from)..at).rev() {
            let byte = buf[index];
            if byte & 0xC0 == 0x80 {
                continue;
            }

            let length = match byte {
                0xF0.. => 4,
                0xE0.. => 3,
                0xC0.. => 2,
                _ => 1,
            };

            return if index + length > at { index } else { at };
        }

        at
    }

    fn cut_utf16(&self, buf: &[u8], from: usize, at: usize) -> usize {
        let mut cut = at - at % 2;

        // Not between the two halves of a surrogate pair.
        if cut >= from + 4 {
            let pair = [buf[cut - 2], buf[cut - 1]];
            let unit = if self.unit == Unit::Utf16Le {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            };

            if (0xD800..0xDC00).contains(&unit) {
                cut -= 2;
            }
        }

        cut
    }

    /// The offset to start at for `offset`, within the text of a file of `size` bytes. With `snap`, the start of the
    /// line it falls in (looked for in the [`MAX_SNAP`] bytes before it; a longer line is entered where `offset` is).
    pub(super) fn place(&self, file: &ViewFile, offset: u64, snap: bool, size: u64) -> io::Result<u64> {
        let mut offset = offset.clamp(self.floor, size.max(self.floor));
        if self.unit != Unit::Byte {
            offset -= offset % 2;
        }

        if snap {
            return self.line_start(file, offset);
        }

        self.align(file, offset)
    }

    /// Moves `offset` forward off the middle of a UTF-8 character.
    fn align(&self, file: &ViewFile, offset: u64) -> io::Result<u64> {
        if self.encoding != UTF_8 {
            return Ok(offset);
        }

        let next = file.read_at(offset, 3)?;
        let skipped = next.iter().take_while(|byte| **byte & 0xC0 == 0x80).count();

        Ok(offset + skipped as u64)
    }

    fn line_start(&self, file: &ViewFile, offset: u64) -> io::Result<u64> {
        if offset <= self.floor {
            return Ok(self.floor);
        }

        let from = offset.saturating_sub(MAX_SNAP as u64).max(self.floor);
        let before = file.read_at(from, (offset - from) as usize)?;

        match self.newlines(&before, from).next_back() {
            Some(at) => Ok(from + (at + self.width()) as u64),
            None if from == self.floor => Ok(self.floor),
            None => self.align(file, offset),
        }
    }
}

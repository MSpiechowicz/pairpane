//! Rewriting a tar, plain or in a compression: the old stream is decoded block by block, the blocks of every entry that
//! stays are written to the new one as they are, and the new entries follow.
//!
//! What an entry is made of is read with the same code the listing uses (see [`head`]): the headers that describe it (a GNU
//! long name, PAX records) come before its own header and are kept with it, or dropped with it when the entry is left out; a
//! PAX global header describes all that follows, so it stays whatever becomes of the entries around it. The headers of one
//! entry are kept in memory, up to [`MAX_HEADERS`]: an entry with more is a refusal, not a guess. Its data is copied in
//! chunks, and its padding is written again as zeros. The blocks of zeros that end the old tar are not copied, [`TarWriter`]
//! writes its own after the new entries, and the stream is read to its very end first, so a damaged compressed stream (a
//! checksum that does not match) fails the rewrite instead of being carried over as a whole one. What follows the end
//! blocks must be zeros (a tar with another one appended is read by `tar` up to the first end only, and the rest would be
//! lost), and a gzip file must end with its first member: otherwise the rewrite is refused.
//!
//! The new tar is encoded by the codec of the old one at its default level.

use std::cell::Cell;
use std::io::{self, Read, Seek, Write};

use crate::archive::MAX_INDEX_SCAN;
use crate::archive::error::{check, data_after_end, unreadable};
use crate::archive::tar::walk::{BLOCK, Head, Pending, Tape, head, padded};
use crate::archive::tar::{Stream, stream};
use crate::archive::write::{Entry, TarWriter, pump};
use crate::model::PackLevel;
use crate::view::refusal;

use super::{Metered, Old};

/// The most bytes of the headers of one entry that are kept: a long name is 4 KiB at most, PAX records 64 KiB.
const MAX_HEADERS: usize = 256 * 1024;

impl<S: Stream> Stream for Metered<'_, S> {
    fn skip(&mut self, count: u64) -> io::Result<()> {
        self.inner.skip(count)?;

        (self.progress)(count)
    }
}

/// Writes the tar of `old` without its left-out entries, then `added`, into `out`, as the old one is encoded.
pub(super) fn rewrite<'a, W: Write + Seek>(out: W, old: Old<'a>, added: Vec<Entry<'a>>) -> io::Result<W> {
    let format = old.index.format();
    let reached = Cell::new(false);
    let source = stream::open_whole(format, old.file, old.cancel, &reached, MAX_INDEX_SCAN)?;
    let mut stream = Metered {
        inner: source,
        progress: &mut *old.progress,
    };
    let mut writer = TarWriter::with_format(format, out, PackLevel::Normal, old.cancel)?;

    let mut pending = Pending::default();
    let mut tape = Tape::new(MAX_HEADERS);
    let mut ordinal = 0_u32;
    let mut settings = false;

    loop {
        check(old.cancel)?;
        let before = tape.bytes().len();

        match head(&mut stream, &mut pending, old.cancel, Some(&mut tape))? {
            Head::End => break,
            Head::Described { global: false } => {}
            Head::Described { global: true } => {
                // It describes every entry from here on, so it is kept; the headers of the next entry stay on the tape.
                let global = tape.split_off(before);
                settings |= sets_more_than_notes(&global);
                writer.raw().write_all(&global)?;
            }
            Head::Entry { data, .. } => {
                if old.left_out.binary_search(&ordinal).is_ok() {
                    stream.skip(padded(data)?)?;
                } else {
                    writer.raw().write_all(tape.bytes())?;
                    copy_data(&mut stream, &mut writer, data, old.cancel)?;
                }

                tape.clear();
                ordinal += 1;
            }
        }
    }

    // Reading on to the end checks what the codec checks there (a gzip's CRC, an XZ's check), and that nothing but zeros
    // is left: a tar that goes on after its end blocks (another tar appended to it) would lose that data.
    pump(&mut stream, &mut Zeros, old.cancel, |_| {})?;
    if reached.get() {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the archive is too large to be changed",
        ));
    }

    // GNU tar applies a global header to the entries that follow it, the added ones included.
    if settings && !added.is_empty() {
        return Err(refusal(
            io::ErrorKind::Unsupported,
            "this archive has settings that would change the added files, so it can't be changed",
        ));
    }

    for entry in added {
        writer.add(entry)?;
    }

    writer.finish()
}

/// Whether the PAX global header `block` (its header block, its records and their padding) has a keyword that changes
/// how the entries after it are read, which is any but a comment and the charset. Records that cannot be read count.
fn sets_more_than_notes(block: &[u8]) -> bool {
    let Some((header, rest)) = block.split_at_checked(BLOCK as usize) else {
        return true;
    };
    let size = ::tar::Header::from_byte_slice(header).entry_size().ok();
    let Some(records) = size.and_then(|size| rest.get(..usize::try_from(size).ok()?)) else {
        return true;
    };

    ::tar::PaxExtensions::new(records)
        .any(|record| record.map_or(true, |record| !matches!(record.key_bytes(), b"comment" | b"charset")))
}

/// A sink that takes zeros only: the rest of a tar after its end blocks.
struct Zeros;

impl Write for Zeros {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.iter().any(|byte| *byte != 0) {
            return Err(data_after_end());
        }

        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Copies the `size` bytes of data of an entry from `stream` to the new tar, and the padding that follows: stepped over in
/// the old one, zeros in the new one.
fn copy_data<W: Write>(
    stream: &mut impl Stream,
    writer: &mut TarWriter<'_, W>,
    size: u64,
    cancel: &std::sync::atomic::AtomicBool,
) -> io::Result<()> {
    let copied = pump(&mut stream.by_ref().take(size), writer.raw(), cancel, |_| {})?;
    if copied != size {
        return Err(unreadable());
    }

    stream.skip(padded(size)? - size)?;

    let padding = (BLOCK - size % BLOCK) % BLOCK;
    writer.raw().write_all(&[0; BLOCK as usize][..padding as usize])
}

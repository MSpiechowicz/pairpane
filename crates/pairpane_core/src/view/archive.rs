//! Listing an archive: zip (the central directory only, see `zip_list`), tar and tar.gz. Nothing is extracted and
//! nothing but the names, sizes and times is looked at; an entry's data is skipped, never read (except to inflate past
//! it in a `.tar.gz`, up to [`MAX_GZ_SCAN`] bytes).

use std::cell::Cell;
use std::io::{self, BufReader, Read, Seek};
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::read::GzDecoder;

use crate::model::{ArchiveFormat, ArchiveListing};

use super::file::{FileReader, ViewFile};
use super::tar_list::{self, Stream};
use super::zip_list;
use super::{MAX_GZ_SCAN, cancelled, refusal};

/// The longest entry name kept, in bytes.
pub(super) const MAX_NAME: usize = 4096;

pub(super) fn unreadable() -> io::Error {
    refusal(io::ErrorKind::InvalidData, "the archive cannot be read")
}

/// The entries of the archive `file` of `format`, at most [`super::MAX_ARCHIVE_ENTRIES`] of them. An archive that cannot be
/// read, or whose central directory is over 64 MiB or declares more than 200,000 entries, is an error; a tar that
/// breaks after some entries, or a `.tar.gz` that is longer than [`MAX_GZ_SCAN`] inflated, lists what was read and
/// is marked truncated. A raised `cancel` stops the listing with an `Interrupted` error.
pub fn archive(file: &ViewFile, format: ArchiveFormat, cancel: &AtomicBool) -> io::Result<ArchiveListing> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    let (entries, truncated) = match format {
        ArchiveFormat::Zip => zip_list::list(file, cancel)?,
        ArchiveFormat::Tar => tar_list::list(&mut BufReader::new(file.reader()), None, cancel)?,
        ArchiveFormat::TarGz => {
            let reached = Cell::new(false);
            let mut inflated = Capped {
                inner: GzDecoder::new(BufReader::new(file.reader())),
                left: MAX_GZ_SCAN,
                reached: &reached,
                cancel,
            };

            tar_list::list(&mut inflated, Some(&reached), cancel)?
        }
    };

    Ok(ArchiveListing {
        format,
        entries,
        truncated,
    })
}

/// Reads at most `left` bytes of `inner`, then reports the end of the data and remembers it was cut. Once `cancel`
/// is raised a read fails with `Interrupted`.
pub(super) struct Capped<'a, R> {
    pub(super) inner: R,
    pub(super) left: u64,
    pub(super) reached: &'a Cell<bool>,
    pub(super) cancel: &'a AtomicBool,
}

impl<R: Read> Read for Capped<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        if self.left == 0 {
            self.reached.set(true);
            return Ok(0);
        }

        let room = usize::try_from(self.left).map_or(buffer.len(), |left| left.min(buffer.len()));
        let count = self.inner.read(&mut buffer[..room])?;
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

impl Stream for BufReader<FileReader<'_>> {
    fn skip(&mut self, count: u64) -> io::Result<()> {
        self.seek_relative(i64::try_from(count).map_err(|_| unreadable())?)?;

        // A seek past the end succeeds: data that the file does not hold is the file being cut short.
        if self.stream_position()? > self.get_ref().size()? {
            return Err(unreadable());
        }

        Ok(())
    }
}

/// An entry name as text: invalid bytes show as `\u{FFFD}`, see [`clean_name`].
pub(super) fn name_of(bytes: &[u8]) -> String {
    clean_name(&String::from_utf8_lossy(bytes))
}

/// `text` as shown for an entry name: a control character (C0, C1) or a bidirectional control that could make the
/// name read as another one becomes `\u{FFFD}`, and a name over [`MAX_NAME`] bytes is cut on a character.
pub(super) fn clean_name(text: &str) -> String {
    let mut name = String::with_capacity(text.len().min(MAX_NAME));

    for character in text.chars() {
        let shown = match character {
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}' => '\u{FFFD}',
            control if control.is_control() => '\u{FFFD}',
            other => other,
        };

        if name.len() + shown.len_utf8() > MAX_NAME {
            break;
        }

        name.push(shown);
    }

    name
}

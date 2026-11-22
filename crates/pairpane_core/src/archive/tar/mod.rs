//! Tar archives, plain and inside a compression. [`walk`] reads the headers of a [`Stream`] and calls a visitor with each
//! entry and its data; [`stream::open`] makes the stream of a format; [`TarSource`] is the [`EntrySource`] that extracts
//! the entries of a tar by walking it.

pub mod stream;
pub mod walk;

use std::cell::Cell;
use std::io;
use std::sync::atomic::AtomicBool;

use super::entries::{EntrySource, Flow, Visit};
use super::error::unreadable;
use super::file::ReadAt;
use crate::model::ArchiveFormat;

pub use stream::{Capped, Stream};
pub use walk::{Data, walk};

/// Extracts from a tar (plain or compressed), by walking it from its start.
pub struct TarSource<'a> {
    format: ArchiveFormat,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    index: &'a super::index::ArchiveIndex,
    scan: u64,
}

impl<'a> TarSource<'a> {
    /// A source for the archive of `index` in `file`; a compressed stream ends after `scan` decoded bytes.
    pub fn new(index: &'a super::index::ArchiveIndex, file: &'a dyn ReadAt, cancel: &'a AtomicBool, scan: u64) -> Self {
        Self {
            format: index.format(),
            file,
            cancel,
            index,
            scan,
        }
    }
}

impl EntrySource for TarSource<'_> {
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()> {
        let Some(last) = wanted.last().copied() else {
            return Ok(());
        };

        let reached = Cell::new(false);
        let mut stream = stream::open(self.format, self.file, self.cancel, &reached, self.scan)?;
        let mut ordinal = 0_u32;
        let mut ahead = wanted.iter().copied().peekable();

        let walked = walk(&mut stream, Some(&reached), self.cancel, |_, data| {
            let here = ordinal;
            ordinal += 1;

            if ahead.peek() != Some(&here) {
                return Ok(Flow::Continue);
            }

            ahead.next();
            let flow = visit(&self.index.entries()[here as usize], data)?;

            Ok(if here == last { Flow::Stop } else { flow })
        })?;

        // The archive ended before every wanted entry was there: it changed since it was indexed.
        if ahead.peek().is_some() && !walked.stopped {
            return Err(unreadable());
        }

        Ok(())
    }
}

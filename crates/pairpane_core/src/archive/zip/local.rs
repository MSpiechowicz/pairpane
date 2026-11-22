//! Where the data of a zip entry is. The central directory gives the offset of the entry's local header, and the header
//! gives the lengths of the name and extra field that sit between it and the data; those lengths are the header's own
//! (they are not always the directory's). Every range is checked against the file, and against the entries that follow:
//! a zip whose entries overlap (the "zip bomb" that reuses data for many entries) is refused whole.

use std::io;

use crate::archive::entries::Place;
use crate::archive::error::unreadable;
use crate::archive::file::ReadAt;
use crate::archive::index::IndexEntry;
use crate::view::refusal;

use super::ZipPlace;

/// The fixed part of a local header, in bytes.
const HEADER: u64 = 30;

const SIGNATURE: [u8; 4] = *b"PK\x03\x04";

fn overlap() -> io::Error {
    refusal(io::ErrorKind::InvalidData, "the entries of the archive overlap")
}

/// The local headers of a zip, as far as the central directory places them: sorted starts, so the room an entry has is
/// known.
pub struct Layout {
    starts: Vec<u64>,
}

impl Layout {
    /// The layout of the entries of an archive. Fails when two entries' ranges overlap: each takes at least its local
    /// header (30 bytes) and its stored size from its start, whatever its name and extra field turn out to be.
    pub fn new(entries: &[IndexEntry]) -> io::Result<Self> {
        let mut ranges: Vec<(u64, u64)> = entries
            .iter()
            .filter_map(|entry| match &entry.place {
                Place::Zip(place) => Some((
                    place.offset,
                    place.offset.saturating_add(HEADER).saturating_add(place.compressed),
                )),
                _ => None,
            })
            .collect();
        ranges.sort_unstable();

        if ranges.windows(2).any(|pair| pair[1].0 < pair[0].1) {
            return Err(overlap());
        }

        Ok(Self {
            starts: ranges.into_iter().map(|(start, _)| start).collect(),
        })
    }

    /// Where the stored data of `place` starts. It must lie inside `file` and end before the next entry's local header.
    pub fn data(&self, file: &dyn ReadAt, place: &ZipPlace) -> io::Result<u64> {
        let head = file.read_chunk(place.offset, HEADER as usize)?;

        if head.len() < HEADER as usize || head[..4] != SIGNATURE {
            return Err(unreadable());
        }

        let name = u64::from(u16::from_le_bytes([head[26], head[27]]));
        let extra = u64::from(u16::from_le_bytes([head[28], head[29]]));
        let start = place.offset + HEADER + name + extra;
        let end = start.checked_add(place.compressed).ok_or_else(unreadable)?;

        if end > file.size()? {
            return Err(unreadable());
        }

        let next = self.starts.partition_point(|start| *start <= place.offset);

        if self.starts.get(next).is_some_and(|following| *following < end) {
            return Err(overlap());
        }

        Ok(start)
    }
}

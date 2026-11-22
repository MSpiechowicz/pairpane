//! Rewriting a zip: every entry that stays is copied as it is, from its local header up to the next entry's (so the data
//! and the data descriptor it may have come along), and its record of the central directory is copied as it is too, with
//! the offset of the local header changed to where the copy landed. Nothing is decoded, checked or encrypted again: a
//! damaged entry stays as damaged as it was.
//!
//! The offset in a record is 32 bits, or, when the field says all ones, the last number of the zip64 extra field (after the
//! sizes that the record has there). A record that has it keeps it and gets the new number in it; one that does not gets
//! the field (or a number added to the one it has) only when the new offset no longer fits 32 bits.
//!
//! The comment of the archive is carried over to the new end record. A zip with data before its first entry (a
//! self-extracting program) is refused: that data would be lost.
//!
//! New entries are written by [`ZipWriter`] after the old ones, deflated at the normal level; when the old zip has an
//! encrypted entry they are encrypted with its password (AES-256, as the writer does for a pack).

use std::io::{self, Seek, SeekFrom, Write};
use std::ops::Range;

use crate::archive::error::{PasswordFault, password_error, unreadable};
use crate::archive::file::FileReader;
use crate::archive::write::{Entry, Limits, Options, ZipWriter};
use crate::archive::zip::local::Layout;
use crate::archive::zip::{ZipPlace, directory};
use crate::archive::{ArchiveIndex, Place};
use crate::model::PackLevel;
use crate::view::refusal;

use super::{Metered, Old, is_encrypted};

/// The fixed part of a local header and of a record of the central directory, in bytes.
const LOCAL: u64 = 30;
const RECORD: usize = 46;

/// The 32-bit number that says "it is in the zip64 field".
const WIDE_32: u64 = 0xFFFF_FFFF;

/// The version needed to extract an entry that has a zip64 field.
const VERSION_ZIP64: u16 = 45;

/// Where each entry of an old zip ends: at the next entry's local header, or at the central directory for the last one.
struct Ends {
    starts: Vec<u64>,
    directory: u64,
}

impl Ends {
    fn of(index: &ArchiveIndex) -> Self {
        let mut starts = Vec::new();
        let mut directory = u64::MAX;

        for entry in index.entries() {
            if let Place::Zip(place) = &entry.place {
                starts.push(place.offset);
                directory = directory.min(place.record_at);
            }
        }
        starts.sort_unstable();

        Self { starts, directory }
    }

    /// The bytes the entry `place` takes in the file: its local header, its data and what follows, up to the next entry.
    fn length_of(&self, place: &ZipPlace) -> io::Result<u64> {
        let next = self.starts.partition_point(|start| *start <= place.offset);
        let end = self
            .starts
            .get(next)
            .map_or(self.directory, |start| (*start).min(self.directory));
        let length = end.checked_sub(place.offset).ok_or_else(unreadable)?;

        // A local header and the data the directory says: anything less is an archive that is not what it says.
        if length < LOCAL.saturating_add(place.compressed) {
            return Err(unreadable());
        }

        Ok(length)
    }
}

/// Whether the first local header of the zip of `index` is not at the start of the file.
fn starts_late(index: &ArchiveIndex) -> bool {
    index
        .entries()
        .iter()
        .filter_map(|entry| match &entry.place {
            Place::Zip(place) => Some(place.offset),
            _ => None,
        })
        .min()
        .is_some_and(|first| first > 0)
}

/// Writes the zip of `old` without its left-out entries, then `added`, into `out`; `limits` say where a number becomes a
/// zip64 one (the format's own, except in tests).
pub(super) fn rewrite<'a, W: Write + Seek>(
    out: W,
    old: Old<'a>,
    added: Vec<Entry<'a>>,
    limits: Limits,
) -> io::Result<W> {
    // An archive whose entries overlap is refused whole, as the readers refuse it.
    Layout::new(old.index.entries())?;

    // What comes before the first entry (a self-extracting program) is not copied: refused, not dropped.
    if starts_late(old.index) {
        return Err(refusal(
            io::ErrorKind::Unsupported,
            "this archive has data before its entries, so it can't be changed",
        ));
    }

    let encrypted = is_encrypted(old.index);
    if encrypted && old.password.is_none() {
        return Err(password_error(PasswordFault::Needed));
    }

    let options = Options {
        level: PackLevel::Normal,
        password: old.password.filter(|_| encrypted).cloned(),
        encrypt_names: false,
    };
    let mut writer =
        ZipWriter::with_limits(out, &options, old.cancel, limits).with_comment(directory::comment(old.file)?);
    let ends = Ends::of(old.index);

    for entry in old.index.entries() {
        if old.leaves_out(entry.ordinal) {
            continue;
        }

        let Place::Zip(place) = &entry.place else {
            return Err(unreadable());
        };
        let length = ends.length_of(place)?;

        if old.file.read_chunk(place.offset, 4)? != b"PK\x03\x04" {
            return Err(unreadable());
        }
        let record = old.file.read_chunk(place.record_at, place.record_len as usize)?;

        let mut reader = FileReader::new(old.file);
        reader.seek(SeekFrom::Start(place.offset))?;
        let mut from = Metered {
            inner: reader,
            progress: &mut *old.progress,
        };

        writer.copy_entry(&mut from, length, |offset, limit| patched(&record, offset, limit))?;
    }

    for entry in added {
        writer.add(entry)?;
    }

    writer.finish()
}

/// The little-endian number of `width` bytes at `at` in `bytes`, which must be long enough.
fn number(bytes: &[u8], at: usize, width: usize) -> u64 {
    (0..width)
        .rev()
        .fold(0, |number, index| (number << 8) | u64::from(bytes[at + index]))
}

/// The record `record` of the central directory for an entry whose local header is now at `offset`; an offset from `limit`
/// on is a zip64 one.
fn patched(record: &[u8], offset: u64, limit: u64) -> io::Result<Vec<u8>> {
    if record.len() < RECORD || !record.starts_with(b"PK\x01\x02") {
        return Err(unreadable());
    }

    let name = number(record, 28, 2) as usize;
    let extra = number(record, 30, 2) as usize;
    let comment = number(record, 32, 2) as usize;
    if record.len() != RECORD + name + extra + comment {
        return Err(unreadable());
    }

    let extra_at = RECORD + name;
    let field = zip64_field(&record[extra_at..extra_at + extra]);
    // What the record says in its 32-bit numbers: the sizes that are in the zip64 field come before the offset in it.
    let wide_sizes = [number(record, 24, 4), number(record, 20, 4)]
        .iter()
        .filter(|size| **size == WIDE_32)
        .count();
    let mut patched = record.to_vec();

    if number(record, 42, 4) == WIDE_32 {
        // The offset has its place in the zip64 field already: it only changes.
        let range = field.as_ref().ok_or_else(unreadable)?;
        let at = extra_at + range.start + 8 * wide_sizes;

        if at + 8 > extra_at + range.end {
            return Err(unreadable());
        }
        patched[at..at + 8].copy_from_slice(&offset.to_le_bytes());

        return Ok(patched);
    }

    if offset < limit.min(WIDE_32) {
        patched[42..46].copy_from_slice(&(offset as u32).to_le_bytes());

        return Ok(patched);
    }

    widened(record, offset, field, wide_sizes)
}

/// The record with its offset (which no longer fits 32 bits) in a zip64 field: the one it has, or a new one at the end of
/// its extra data.
fn widened(record: &[u8], offset: u64, field: Option<Range<usize>>, wide_sizes: usize) -> io::Result<Vec<u8>> {
    let name = number(record, 28, 2) as usize;
    let extra_at = RECORD + name;
    let extra_end = extra_at + number(record, 30, 2) as usize;
    let mut extra = record[extra_at..extra_end].to_vec();

    match field {
        Some(range) => {
            let at = range.start + 8 * wide_sizes;
            if at > range.end {
                return Err(unreadable());
            }

            extra.splice(at..at, offset.to_le_bytes());
            let length = u16::try_from(range.len() + 8).map_err(|_| unreadable())?;
            extra[range.start - 2..range.start].copy_from_slice(&length.to_le_bytes());
        }
        None => {
            extra.extend_from_slice(&1_u16.to_le_bytes());
            extra.extend_from_slice(&8_u16.to_le_bytes());
            extra.extend_from_slice(&offset.to_le_bytes());
        }
    }

    let mut patched = record[..extra_at].to_vec();
    patched.extend_from_slice(&extra);
    patched.extend_from_slice(&record[extra_end..]);

    let length = u16::try_from(extra.len()).map_err(|_| unreadable())?;
    patched[30..32].copy_from_slice(&length.to_le_bytes());
    patched[42..46].copy_from_slice(&(WIDE_32 as u32).to_le_bytes());

    let needed = u16::from_le_bytes([patched[6], patched[7]]).max(VERSION_ZIP64);
    patched[6..8].copy_from_slice(&needed.to_le_bytes());

    Ok(patched)
}

/// Where the data of the zip64 field (id 1) is in the extra data `extra`, when it has one that is whole. Fields after a
/// broken one are not looked at.
fn zip64_field(extra: &[u8]) -> Option<Range<usize>> {
    let mut at = 0;

    while at + 4 <= extra.len() {
        let id = number(extra, at, 2);
        let length = number(extra, at + 2, 2) as usize;
        let end = (at + 4).checked_add(length).filter(|end| *end <= extra.len())?;

        if id == 1 {
            return Some(at + 4..end);
        }

        at = end;
    }

    None
}

//! Listing a tar by walking its headers one at a time. The `tar` crate's own iterator reads a GNU long name or a PAX
//! record whole, as large as the header declares, so a header can make it hold gigabytes; here at most
//! [`NAME_READ`] (or [`PAX_READ`]) bytes of such an entry are read and the rest of it is stepped over, never kept.

use std::cell::Cell;
use std::io::{self, Read};
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::model::ArchiveEntry;

use super::archive::{MAX_NAME, name_of, unreadable};
use super::{MAX_ARCHIVE_ENTRIES, cancelled};

/// The size of a tar header, and the unit its entries are padded to.
const BLOCK: u64 = 512;

/// The bytes of a long name that are read: the longest name kept, and room for the character it may cut in two.
const NAME_READ: usize = MAX_NAME + 4;

/// The bytes of a PAX record list that are read; records past them are not looked at.
const PAX_READ: usize = 64 * 1024;

/// Where a block of a GNU sparse header says another one follows, within that block.
const EXTENDED_FLAG: usize = 504;

/// A tar to read headers from, and to step over data of.
pub(super) trait Stream: Read {
    /// Moves on by `count` bytes without keeping any of them.
    fn skip(&mut self, count: u64) -> io::Result<()>;
}

/// What the entries before an entry said about it.
#[derive(Default)]
struct Pending {
    long_name: Option<Vec<u8>>,
    pax_name: Option<Vec<u8>>,
    pax_size: Option<u64>,
}

impl Pending {
    /// Takes the path and size from the records of a PAX entry.
    fn pax(&mut self, records: &[u8]) {
        for record in tar::PaxExtensions::new(records).flatten() {
            match record.key_bytes() {
                b"path" => self.pax_name = Some(record.value_bytes().to_vec()).filter(|name| !name.is_empty()),
                b"size" => self.pax_size = record.value().ok().and_then(|text| text.parse().ok()),
                _ => {}
            }
        }
    }
}

enum Step {
    /// The end of the archive.
    End,
    /// An entry that only describes the next one.
    Described,
    Entry(ArchiveEntry),
    /// An entry whose data the archive does not all hold: the last one that can be listed.
    Cut(ArchiveEntry),
}

/// The entries of the tar in `stream`, at most [`MAX_ARCHIVE_ENTRIES`], and whether the list is cut. A tar that
/// breaks after some entries, or that `cut` says was inflated only up to a limit, lists what was read and is marked
/// truncated. A raised `cancel` ends the walk with an `Interrupted` error.
pub(super) fn list<S: Stream>(
    stream: &mut S,
    cut: Option<&Cell<bool>>,
    cancel: &AtomicBool,
) -> io::Result<(Vec<ArchiveEntry>, bool)> {
    let mut listed = Vec::new();
    let mut pending = Pending::default();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        match step(stream, &mut pending, cancel) {
            Ok(Step::End) => break,
            Ok(Step::Described) => {}
            Ok(Step::Entry(entry)) => {
                if listed.len() == MAX_ARCHIVE_ENTRIES {
                    return Ok((listed, true));
                }

                listed.push(entry);
            }
            Ok(Step::Cut(entry)) => {
                if listed.len() < MAX_ARCHIVE_ENTRIES {
                    listed.push(entry);
                }

                return Ok((listed, true));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(_) => {
                // The inflated data ran out at the cap, or the archive is broken from here on.
                if listed.is_empty() && !cut.is_some_and(Cell::get) {
                    return Err(unreadable());
                }

                return Ok((listed, true));
            }
        }
    }

    Ok((listed, cut.is_some_and(Cell::get)))
}

fn step<S: Stream>(stream: &mut S, pending: &mut Pending, cancel: &AtomicBool) -> io::Result<Step> {
    let mut header = tar::Header::new_old();
    match fill(stream, header.as_mut_bytes(), cancel)? {
        0 => return Ok(Step::End),
        count if count < header.as_bytes().len() => return Err(unreadable()),
        _ => {}
    }

    // A block of zeros ends the archive.
    if header.as_bytes().iter().all(|byte| *byte == 0) {
        return Ok(Step::End);
    }

    if !checksum_matches(&header) {
        return Err(unreadable());
    }

    let size = header.entry_size()?;
    let kind = header.entry_type();
    let known = header.as_gnu().is_some() || header.as_ustar().is_some();

    if known && kind.is_gnu_longname() {
        let name = read_some(stream, size, NAME_READ, cancel)?;
        pending.long_name =
            Some(name.split(|byte| *byte == 0).next().unwrap_or_default().to_vec()).filter(|name| !name.is_empty());

        return Ok(Step::Described);
    }

    if known && kind.is_pax_local_extensions() {
        pending.pax(&read_some(stream, size, PAX_READ, cancel)?);

        return Ok(Step::Described);
    }

    if known && (kind.is_gnu_longlink() || kind.is_pax_global_extensions()) {
        stream.skip(padded(size)?)?;

        return Ok(Step::Described);
    }

    if kind.is_gnu_sparse() && header.as_gnu().is_some_and(tar::GnuHeader::is_extended) {
        skip_sparse_blocks(stream, cancel)?;
    }

    let pending = mem::take(pending);
    let data = pending.pax_size.unwrap_or(size);
    let modified = header
        .mtime()
        .ok()
        .filter(|seconds| *seconds != 0)
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| seconds.checked_mul(1000));
    // The PAX path is the newest word on the name, ahead of a GNU long name.
    let name = pending
        .pax_name
        .or(pending.long_name)
        .unwrap_or_else(|| header.path_bytes().into_owned());
    let shown = match pending.pax_size {
        Some(size) => size,
        None => header.size()?,
    };

    let entry = ArchiveEntry {
        name: name_of(&name[..name.len().min(NAME_READ)]),
        size: shown,
        // A regular file named like a folder is one (as `tar` extracts it).
        dir: kind.is_dir() || (kind.is_file() && name.ends_with(b"/")),
        modified,
    };

    match stream.skip(padded(data)?) {
        Ok(()) => Ok(Step::Entry(entry)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(_) => Ok(Step::Cut(entry)),
    }
}

fn checksum_matches(header: &tar::Header) -> bool {
    let bytes = header.as_bytes();
    let rest = || bytes[..148].iter().chain(&bytes[156..]);

    // The checksum field counts as eight spaces. Some writers summed the bytes as signed numbers: that is accepted too.
    let unsigned: i64 = rest().map(|byte| i64::from(*byte)).sum::<i64>() + 8 * 32;
    let signed: i64 = rest().map(|byte| i64::from(*byte as i8)).sum::<i64>() + 8 * 32;

    header
        .cksum()
        .is_ok_and(|stored| i64::from(stored) == unsigned || i64::from(stored) == signed)
}

/// The size of an entry's data in the archive: rounded up to whole blocks.
fn padded(size: u64) -> io::Result<u64> {
    size.checked_add(BLOCK - 1)
        .map(|size| size & !(BLOCK - 1))
        .ok_or_else(unreadable)
}

/// The first `limit` bytes of an entry of `size` bytes; the rest of it is stepped over.
fn read_some<S: Stream>(stream: &mut S, size: u64, limit: usize, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
    let wanted = usize::try_from(size).map_or(limit, |size| size.min(limit));
    let mut bytes = vec![0; wanted];

    if fill(stream, &mut bytes, cancel)? < wanted {
        return Err(unreadable());
    }

    stream.skip(padded(size)? - wanted as u64)?;

    Ok(bytes)
}

/// Steps over the blocks that follow a GNU sparse header which says so, up to the one that says it is the last.
fn skip_sparse_blocks<S: Stream>(stream: &mut S, cancel: &AtomicBool) -> io::Result<()> {
    let mut block = [0; BLOCK as usize];

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        if fill(stream, &mut block, cancel)? < block.len() {
            return Err(unreadable());
        }

        if block[EXTENDED_FLAG] == 0 {
            return Ok(());
        }
    }
}

/// Reads until `buffer` is full or the stream ends; how many bytes came. An `Interrupted` error is the stream's
/// answer to a raised `cancel`, unless the flag is down (a signal during a read), when the read is tried again.
fn fill<S: Read>(stream: &mut S, buffer: &mut [u8], cancel: &AtomicBool) -> io::Result<usize> {
    let mut filled = 0;

    while filled < buffer.len() {
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted && !cancel.load(Ordering::Relaxed) => {}
            Err(error) => return Err(error),
        }
    }

    Ok(filled)
}

//! Walking a tar by its headers one at a time. The `tar` crate's own iterator reads a GNU long name or a PAX record
//! whole, as large as the header declares, so a header can make it hold gigabytes; here at most [`NAME_READ`] (or
//! [`PAX_READ`]) bytes of such an entry are read and the rest of it is stepped over, never kept.
//!
//! The walk calls a visitor with the [`Record`] of each entry and a reader of its data; a visitor that lists ignores the
//! data and the walk steps over it, one that extracts reads it.

use std::cell::Cell;
use std::io::{self, Read};
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::archive::MAX_NAME_BYTES;
use crate::archive::entries::{EntryType, Flow, Place, Record, Walked, read_within};
use crate::archive::error::{cancelled, unreadable};

use super::stream::Stream;

/// The size of a tar header, and the unit its entries are padded to.
const BLOCK: u64 = 512;

/// The bytes of a long name that are read: the longest name kept, and room for the character it may cut in two.
const NAME_READ: usize = MAX_NAME_BYTES + 4;

/// The bytes of a PAX record list that are read; records past them are not looked at.
const PAX_READ: usize = 64 * 1024;

/// Where a block of a GNU sparse header says another one follows, within that block.
const EXTENDED_FLAG: usize = 504;

/// The data of the entry being visited: exactly the bytes the archive holds for it. It ends early with an error when
/// the archive does.
pub struct Data<'a, S> {
    stream: &'a mut S,
    left: u64,
}

impl<S: Read> Read for Data<'_, S> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buffer.is_empty() {
            return Ok(0);
        }

        let count = read_within(self.stream, self.left, buffer)?;

        if count == 0 {
            return Err(unreadable());
        }

        self.left -= count as u64;

        Ok(count)
    }
}

/// What the entries before an entry said about it.
#[derive(Default)]
struct Pending {
    long_name: Option<Vec<u8>>,
    pax_name: Option<Vec<u8>>,
    pax_link: Option<Vec<u8>>,
    pax_size: Option<u64>,
    /// A PAX record of a sparse file: its data is not the file's content.
    sparse: bool,
}

impl Pending {
    /// Takes the path, link, size and sparse marks from the records of a PAX entry.
    fn pax(&mut self, records: &[u8]) {
        for record in ::tar::PaxExtensions::new(records).flatten() {
            let key = record.key_bytes();

            match key {
                b"path" => self.pax_name = Some(record.value_bytes().to_vec()).filter(|name| !name.is_empty()),
                b"linkpath" => self.pax_link = Some(record.value_bytes().to_vec()).filter(|name| !name.is_empty()),
                b"size" => self.pax_size = record.value().ok().and_then(|text| text.parse().ok()),
                _ if key.starts_with(b"GNU.sparse.") => self.sparse = true,
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
    /// An entry that was visited.
    Entry(Flow),
    /// An entry that was visited and whose data the archive does not all hold: the last one there is.
    Cut(Flow),
    /// The visitor failed: its error ends the walk.
    Abort(io::Error),
}

/// Walks the tar in `stream`, calling `visit` with every entry and a reader of its data, until the end of the archive or
/// until it says [`Flow::Stop`]. A tar that breaks after some entries, or that `cut` says was inflated only up to a limit,
/// is visited as far as it goes and the result says it was cut; a tar that breaks before its first entry is an error. A
/// raised `cancel` ends the walk with an `Interrupted` error, and so does an error of the visitor, as it is.
pub fn walk<S, V>(stream: &mut S, cut: Option<&Cell<bool>>, cancel: &AtomicBool, mut visit: V) -> io::Result<Walked>
where
    S: Stream,
    V: FnMut(Record, &mut Data<'_, S>) -> io::Result<Flow>,
{
    let mut pending = Pending::default();
    let mut seen = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        match step(stream, &mut pending, cancel, &mut visit) {
            Ok(Step::End) => break,
            Ok(Step::Described) => {}
            Ok(Step::Entry(Flow::Continue)) => seen = true,
            Ok(Step::Entry(Flow::Stop)) => {
                return Ok(Walked {
                    stopped: true,
                    cut: false,
                });
            }
            Ok(Step::Cut(flow)) => {
                return Ok(Walked {
                    stopped: flow == Flow::Stop,
                    cut: true,
                });
            }
            Ok(Step::Abort(error)) => return Err(error),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(_) => {
                // The inflated data ran out at the cap, or the archive is broken from here on.
                if !seen && !cut.is_some_and(Cell::get) {
                    return Err(unreadable());
                }

                return Ok(Walked {
                    stopped: false,
                    cut: true,
                });
            }
        }
    }

    Ok(Walked {
        stopped: false,
        cut: cut.is_some_and(Cell::get),
    })
}

fn step<S, V>(stream: &mut S, pending: &mut Pending, cancel: &AtomicBool, visit: &mut V) -> io::Result<Step>
where
    S: Stream,
    V: FnMut(Record, &mut Data<'_, S>) -> io::Result<Flow>,
{
    let mut header = ::tar::Header::new_old();
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

    // A long link name is not read: where a link points is only shown, and links are never extracted.
    if known && (kind.is_gnu_longlink() || kind.is_pax_global_extensions()) {
        stream.skip(padded(size)?)?;

        return Ok(Step::Described);
    }

    if kind.is_gnu_sparse() && header.as_gnu().is_some_and(::tar::GnuHeader::is_extended) {
        skip_sparse_blocks(stream, cancel)?;
    }

    let pending = mem::take(pending);
    let data = pending.pax_size.unwrap_or(size);
    let shown = match pending.pax_size {
        Some(size) => size,
        None => header.size()?,
    };
    let record = record(&header, pending, shown);

    // The visitor takes what it wants of the data; the rest is stepped over, with the padding.
    let mut reader = Data {
        stream: &mut *stream,
        left: data,
    };
    let flow = match visit(record, &mut reader) {
        Ok(flow) => flow,
        Err(error) => return Ok(Step::Abort(error)),
    };
    let unread = reader.left;

    if flow == Flow::Stop {
        return Ok(Step::Entry(flow));
    }

    match stream.skip(unread + (padded(data)? - data)) {
        Ok(()) => Ok(Step::Entry(flow)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(_) => Ok(Step::Cut(flow)),
    }
}

/// The record of the entry whose header is `header`, after what `pending` said of it.
fn record(header: &::tar::Header, pending: Pending, size: u64) -> Record {
    let kind = header.entry_type();
    // The PAX path is the newest word on the name, ahead of a GNU long name.
    let name = pending
        .pax_name
        .or(pending.long_name)
        .unwrap_or_else(|| header.path_bytes().into_owned());
    let name = &name[..name.len().min(NAME_READ)];

    // A regular file named like a folder is one (as `tar` extracts it).
    let kind = if kind.is_gnu_sparse() || pending.sparse {
        EntryType::Special
    } else if kind.is_dir() || ((kind.is_file() || kind.is_contiguous()) && name.ends_with(b"/")) {
        EntryType::Dir
    } else if kind.is_file() || kind.is_contiguous() {
        EntryType::File
    } else if kind.is_symlink() {
        EntryType::Symlink
    } else if kind.is_hard_link() {
        EntryType::Hardlink
    } else {
        EntryType::Special
    };

    let link = match kind {
        EntryType::Symlink | EntryType::Hardlink => pending
            .pax_link
            .or_else(|| header.link_name_bytes().map(|link| link.into_owned()))
            .map(|link| String::from_utf8_lossy(&link[..link.len().min(NAME_READ)]).into_owned()),
        _ => None,
    };

    Record {
        name: String::from_utf8_lossy(name).into_owned(),
        kind,
        size,
        modified: header
            .mtime()
            .ok()
            .filter(|seconds| *seconds != 0)
            .and_then(|seconds| i64::try_from(seconds).ok())
            .and_then(|seconds| seconds.checked_mul(1000)),
        mode: header.mode().ok().map(|mode| mode & 0o7777),
        link,
        encrypted: false,
        place: Place::Walk,
    }
}

fn checksum_matches(header: &::tar::Header) -> bool {
    let bytes = header.as_bytes();
    let rest = || bytes[..148].iter().chain(&bytes[156..]);

    // The checksum field counts as eight spaces. Some writers summed the bytes as signed numbers: that is accepted too.
    let unsigned: i64 = rest().map(|byte| i64::from(*byte)).sum::<i64>() + 8 * 32;
    let signed: i64 = rest().map(|byte| i64::from(*byte as i8)).sum::<i64>() + 8 * 32;

    header
        .cksum()
        .is_ok_and(|stored| i64::from(stored) == unsigned || i64::from(stored) == signed)
}

/// Whether the first block of `head` is a tar header with the checksum it states.
pub fn header_is_valid(head: &[u8]) -> bool {
    let Some(block) = head.get(..BLOCK as usize) else {
        return false;
    };

    block.iter().any(|byte| *byte != 0) && checksum_matches(::tar::Header::from_byte_slice(block))
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

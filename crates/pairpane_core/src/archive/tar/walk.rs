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
use crate::view::refusal;

use super::stream::Stream;

/// The size of a tar header, and the unit its entries are padded to.
pub(crate) const BLOCK: u64 = 512;

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
pub(crate) struct Pending {
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

/// What one go at the headers of a tar finds (see [`head`]).
pub(crate) enum Head {
    /// The end of the archive.
    End,
    /// An entry that only describes the next one (a long name, PAX records), or, when `global`, all that follow.
    Described { global: bool },
    /// An entry: what it is, and how many bytes of data follow its header (before the padding).
    Entry { record: Record, data: u64 },
}

/// The bytes of the headers that [`head`] goes through, kept as they are, for the copy of a tar that has entries left out
/// (see `crate::archive::rewrite`). A header that would take more than the limit fails the copy: none of it is kept
/// without being read, so what is read is bounded by the limit.
pub(crate) struct Tape {
    bytes: Vec<u8>,
    limit: usize,
}

impl Tape {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    /// The bytes kept since the last [`Tape::clear`].
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Takes back what was kept after the first `len` bytes.
    pub(crate) fn split_off(&mut self, len: usize) -> Vec<u8> {
        self.bytes.split_off(len)
    }

    fn room(&self, more: u64) -> io::Result<()> {
        let kept = self.bytes.len() as u64;

        if kept.saturating_add(more) > self.limit as u64 {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "a header of the archive is too large to be kept",
            ));
        }

        Ok(())
    }

    fn keep(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.room(bytes.len() as u64)?;
        self.bytes.extend_from_slice(bytes);

        Ok(())
    }

    /// Reads `count` bytes of `stream` and keeps them.
    fn take<S: Read>(&mut self, stream: &mut S, count: u64, cancel: &AtomicBool) -> io::Result<()> {
        self.room(count)?;

        let at = self.bytes.len();
        self.bytes.resize(at + count as usize, 0);

        if fill(stream, &mut self.bytes[at..], cancel)? < count as usize {
            return Err(unreadable());
        }

        Ok(())
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
    let (record, data) = match head(stream, pending, cancel, None)? {
        Head::End => return Ok(Step::End),
        Head::Described { .. } => return Ok(Step::Described),
        Head::Entry { record, data } => (record, data),
    };

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

/// Reads the headers of the next entry of the tar in `stream`: one entry that describes another, or an entry with its
/// record, after what `pending` was told by those before it. The data of the entry is not read. With a `tape` every byte
/// that was read (or stepped over, which is then read) is also kept in it.
pub(crate) fn head<S: Stream>(
    stream: &mut S,
    pending: &mut Pending,
    cancel: &AtomicBool,
    mut tape: Option<&mut Tape>,
) -> io::Result<Head> {
    let mut header = ::tar::Header::new_old();
    match fill(stream, header.as_mut_bytes(), cancel)? {
        0 => return Ok(Head::End),
        count if count < header.as_bytes().len() => return Err(unreadable()),
        _ => {}
    }

    // A block of zeros ends the archive.
    if header.as_bytes().iter().all(|byte| *byte == 0) {
        return Ok(Head::End);
    }

    if !checksum_matches(&header) {
        return Err(unreadable());
    }

    if let Some(tape) = tape.as_deref_mut() {
        tape.keep(header.as_bytes())?;
    }

    let size = header.entry_size()?;
    let kind = header.entry_type();
    let known = header.as_gnu().is_some() || header.as_ustar().is_some();

    if known && kind.is_gnu_longname() {
        let name = read_some(stream, size, NAME_READ, cancel, tape)?;
        pending.long_name =
            Some(name.split(|byte| *byte == 0).next().unwrap_or_default().to_vec()).filter(|name| !name.is_empty());

        return Ok(Head::Described { global: false });
    }

    if known && kind.is_pax_local_extensions() {
        pending.pax(&read_some(stream, size, PAX_READ, cancel, tape)?);

        return Ok(Head::Described { global: false });
    }

    // A long link name is not read: where a link points is only shown, and links are never extracted.
    if known && (kind.is_gnu_longlink() || kind.is_pax_global_extensions()) {
        pass(stream, padded(size)?, cancel, tape)?;

        return Ok(Head::Described {
            global: kind.is_pax_global_extensions(),
        });
    }

    if kind.is_gnu_sparse() && header.as_gnu().is_some_and(::tar::GnuHeader::is_extended) {
        skip_sparse_blocks(stream, cancel, tape)?;
    }

    let pending = mem::take(pending);
    let data = pending.pax_size.unwrap_or(size);
    let shown = match pending.pax_size {
        Some(size) => size,
        None => header.size()?,
    };

    Ok(Head::Entry {
        record: record(&header, pending, shown),
        data,
    })
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
pub(crate) fn padded(size: u64) -> io::Result<u64> {
    size.checked_add(BLOCK - 1)
        .map(|size| size & !(BLOCK - 1))
        .ok_or_else(unreadable)
}

/// The first `limit` bytes of an entry of `size` bytes; the rest of it is stepped over.
fn read_some<S: Stream>(
    stream: &mut S,
    size: u64,
    limit: usize,
    cancel: &AtomicBool,
    mut tape: Option<&mut Tape>,
) -> io::Result<Vec<u8>> {
    let wanted = usize::try_from(size).map_or(limit, |size| size.min(limit));
    let mut bytes = vec![0; wanted];

    if fill(stream, &mut bytes, cancel)? < wanted {
        return Err(unreadable());
    }

    if let Some(tape) = tape.as_deref_mut() {
        tape.keep(&bytes)?;
    }
    pass(stream, padded(size)? - wanted as u64, cancel, tape)?;

    Ok(bytes)
}

/// Moves on by `count` bytes: stepped over, or, with a `tape`, read and kept.
fn pass<S: Stream>(stream: &mut S, count: u64, cancel: &AtomicBool, tape: Option<&mut Tape>) -> io::Result<()> {
    match tape {
        Some(tape) => tape.take(stream, count, cancel),
        None => stream.skip(count),
    }
}

/// Steps over the blocks that follow a GNU sparse header which says so, up to the one that says it is the last.
fn skip_sparse_blocks<S: Stream>(stream: &mut S, cancel: &AtomicBool, mut tape: Option<&mut Tape>) -> io::Result<()> {
    let mut block = [0; BLOCK as usize];

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        if fill(stream, &mut block, cancel)? < block.len() {
            return Err(unreadable());
        }

        if let Some(tape) = tape.as_deref_mut() {
            tape.keep(&block)?;
        }

        if block[EXTENDED_FLAG] == 0 {
            return Ok(());
        }
    }
}

/// Reads until `buffer` is full or the stream ends; how many bytes came. An `Interrupted` error is the stream's
/// answer to a raised `cancel`, unless the flag is down (a signal during a read), when the read is tried again.
pub(crate) fn fill<S: Read>(stream: &mut S, buffer: &mut [u8], cancel: &AtomicBool) -> io::Result<usize> {
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

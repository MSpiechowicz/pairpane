//! A gzip file that is not a tar, as an archive of one entry. The entry's name is the file name the gzip header holds
//! when that is a plain name, otherwise the archive's own name without `.gz`; its size is the `ISIZE` of the trailer
//! (the length modulo 4 GiB, so only a hint, see [`Place::Gz`]).

use std::io::{self, BufReader};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use flate2::read::GzDecoder;

use crate::listing::platform;
use crate::ops::name_fits;

use super::entries::{Cancellable, Declared, EntrySource, EntryType, Flow, Place, Record, Visit, Walked};
use super::error::unreadable;
use super::file::{FileReader, ReadAt};
use super::index::ArchiveIndex;
use super::names::is_hostile;

/// How much of the start of the file is looked at for the header: the fixed part, the extra field (65,535 bytes at most)
/// and room for the name that follows.
const HEADER_READ: usize = 10 + 2 + 65_535 + 4096;

const EXTRA: u8 = 1 << 2;
const NAME: u8 = 1 << 3;
const RESERVED: u8 = 0xE0;

/// Calls `visit` with the one entry of the gzip file `file`, whose own path (for the name it falls back to) is `path`.
/// A file that is not gzip, or is cut inside its header, is an error.
pub fn walk<F: ReadAt + ?Sized>(
    file: &F,
    path: Option<&Path>,
    mut visit: impl FnMut(Record) -> Flow,
) -> io::Result<Walked> {
    let head = file.read_chunk(0, HEADER_READ)?;

    if head.len() < 10 || head[..3] != [0x1F, 0x8B, 8] || head[3] & RESERVED != 0 {
        return Err(unreadable());
    }

    let flags = head[3];
    let modified = i64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
    let mut rest = &head[10..];

    if flags & EXTRA != 0 {
        let length = rest
            .get(..2)
            .map(|bytes| usize::from(u16::from_le_bytes([bytes[0], bytes[1]])));
        rest = rest.get(2 + length.ok_or_else(unreadable)?..).ok_or_else(unreadable)?;
    }

    let stored = (flags & NAME != 0)
        .then(|| rest.split(|byte| *byte == 0).next())
        .flatten()
        .filter(|_| rest.contains(&0))
        .map(text_of)
        .filter(|name| is_plain(name));

    let size = file.size()?;
    let trailer = file.read_chunk(size.saturating_sub(4), 4)?;
    let isize = match trailer.as_slice() {
        [a, b, c, d] if size >= 18 => u64::from(u32::from_le_bytes([*a, *b, *c, *d])),
        _ => return Err(unreadable()),
    };

    let name = stored.or_else(|| stem(path)).unwrap_or_else(|| "file".to_owned());

    let flow = visit(Record {
        name,
        kind: EntryType::File,
        size: isize,
        modified: (modified != 0).then_some(modified * 1000),
        mode: None,
        link: None,
        encrypted: false,
        place: Place::Gz,
    });

    Ok(Walked {
        stopped: flow == Flow::Stop,
        cut: false,
    })
}

/// A header name is Latin-1 by the format, but most writers put UTF-8 there: UTF-8 when it is valid, Latin-1 otherwise.
fn text_of(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().copied().map(char::from).collect(),
    }
}

/// The name of the archive file without `.gz`, when what is left is a plain name.
fn stem(path: Option<&Path>) -> Option<String> {
    let name = path?.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".gz")
        .map_or(name.as_str(), |_| &name[..name.len() - 3]);

    Some(stem.to_owned()).filter(|stem| is_plain(stem))
}

/// A single name (no folder part) that is safe to write.
fn is_plain(name: &str) -> bool {
    !name.contains(['/', '\\'])
        && platform::is_plain_name(name)
        && !platform::is_unsafe_name(name)
        && name_fits(name)
        && !name.chars().any(is_hostile)
}

/// Extracts the one entry of a gzip file.
pub struct GzSource<'a> {
    index: &'a ArchiveIndex,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    /// The most bytes the entry may inflate to.
    cap: u64,
}

impl<'a> GzSource<'a> {
    pub fn new(index: &'a ArchiveIndex, file: &'a dyn ReadAt, cancel: &'a AtomicBool, cap: u64) -> Self {
        Self {
            index,
            file,
            cancel,
            cap,
        }
    }
}

impl EntrySource for GzSource<'_> {
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()> {
        if wanted.is_empty() {
            return Ok(());
        }

        let entry = self.index.entries().first().ok_or_else(unreadable)?;
        let decoder = GzDecoder::new(BufReader::new(FileReader::new(self.file)));
        let mut inflated = Declared::at_most(
            Cancellable {
                inner: decoder,
                cancel: self.cancel,
            },
            self.cap,
        );
        visit(entry, &mut inflated)?;

        Ok(())
    }
}

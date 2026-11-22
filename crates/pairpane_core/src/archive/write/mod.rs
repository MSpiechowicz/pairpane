//! Writing archives: the writers of the formats a pack job makes (zip, tar.zst, 7z), one common way to feed them and
//! nothing about the disk: they write into a `Write` (`Write + Seek` for zip and 7z) that the caller gives them and know
//! nothing of paths, so no link is ever followed here. The pack job (in `ops`) walks the files, opens each without
//! following links and hands over a reader.
//!
//! # API
//!
//! - [`Packer::new`] starts an archive of a [`PackFormat`] in `out` with [`Options`] (level, password, whether 7z
//!   encrypts names) and a cancel flag; [`Packer::add`] adds one [`Entry`] (a folder, or a file of a declared size with a
//!   reader for its data); [`Packer::finish`] ends the archive and gives `out` back. Any failure, a raised cancel flag
//!   (`Interrupted`) included, is an `io::Error`; the archive written so far is then not complete and the caller
//!   deletes it.
//! - [`Lazy`]: a reader that opens its file at the first read and lets go at the end. 7z holds an entry's reader until
//!   the block it is in is written (about 256 MiB of data), so a reader that is open from the start would keep one file
//!   open per entry; hand over a lazy one.
//!
//! # Rules
//!
//! - Entry names are relative, `/` separated and have been through [`normalize`] already (a folder has no trailing
//!   slash); the writers check that again, and the number and size of the names against the limits the readers have.
//! - A file is read once, with the size it was declared with as its limit. A zip and a 7z take the size actually read when
//!   the file gave less, and fail ("a file grew while it was being packed") when it gives more: one byte past the
//!   declared size is read to know, no more. A tar (which writes the size before the data) fails when the file ended
//!   early, and fails the same way as the others when it grew.
//! - The cancel flag is checked between chunks of 64 KiB by a copy loop of our own, never `io::copy`, which tries a read
//!   again at every `Interrupted` and so would not stop.
//! - Modes are stored as `mode & 0o777`; on Windows a zip and a 7z store DOS attributes instead of a mode.
//! - Passwords: zip takes AES-256 (AE-2, a random salt for each entry), 7z AES-256 with a key of 2^19 rounds and random
//!   IV and salt (fresh IV for each block); tar.zst has no encryption and refuses a password.

use std::io::{self, Read, Seek, Write};
use std::sync::atomic::AtomicBool;

use crate::archive::error::check;
use crate::archive::{MAX_INDEX_ENTRIES, MAX_INDEX_NAME_BYTES, Password, normalize};
use crate::model::{PackFormat, PackLevel};
use crate::view::refusal;

mod dos;
mod sevenz;
mod tar;
mod zip;
mod zip_data;

#[cfg(test)]
mod roundtrip_tests;
#[cfg(test)]
mod sevenz_tests;
#[cfg(test)]
mod tar_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod zip_tests;

pub use sevenz::SevenZWriter;
pub use tar::TarWriter;
pub use zip::{Limits, ZipWriter};

/// How much data one chunk of the copy loop moves, and so how often the cancel flag is looked at.
const CHUNK: usize = 64 * 1024;

/// What a pack job asks of the writers.
#[derive(Debug, Clone)]
pub struct Options {
    pub level: PackLevel,
    /// Encrypts the content of every file: zip AES-256, 7z AES-256. Not for tar.zst.
    pub password: Option<Password>,
    /// 7z only: encrypts the headers (the names) too. Without a password it does nothing.
    pub encrypt_names: bool,
}

/// What an entry is.
pub enum Content<'a> {
    Folder,
    /// A file of `size` bytes (stat's: a zip and a 7z take what the reader really gives); an empty one has size 0 and its
    /// reader is never read.
    File {
        size: u64,
        reader: Box<dyn Read + 'a>,
    },
}

/// One entry to write.
pub struct Entry<'a> {
    /// Relative, `/` separated, no trailing slash, already normalized.
    pub name: String,
    /// The permission bits (`mode & 0o777` is stored).
    pub mode: u32,
    /// Milliseconds since the Unix epoch (what [`Record::modified`](crate::archive::Record) holds).
    pub modified: Option<i64>,
    pub content: Content<'a>,
}

impl<'a> Entry<'a> {
    pub fn folder(name: impl Into<String>, mode: u32, modified: Option<i64>) -> Self {
        Self {
            name: name.into(),
            mode,
            modified,
            content: Content::Folder,
        }
    }

    pub fn file(name: impl Into<String>, mode: u32, modified: Option<i64>, size: u64, reader: impl Read + 'a) -> Self {
        Self {
            name: name.into(),
            mode,
            modified,
            content: Content::File {
                size,
                reader: Box::new(reader),
            },
        }
    }
}

/// An archive under way, whatever its format.
pub enum Packer<'a, W: Write + Seek> {
    Zip(ZipWriter<'a, W>),
    TarZst(TarWriter<'a, W>),
    SevenZ(SevenZWriter<'a, W>),
}

impl<'a, W: Write + Seek> Packer<'a, W> {
    /// Starts an archive of `format` in `out`. A password for a tar.zst, or an empty one, is refused.
    pub fn new(format: PackFormat, out: W, options: &Options, cancel: &'a AtomicBool) -> io::Result<Self> {
        if options
            .password
            .as_ref()
            .is_some_and(|password| password.as_str().is_empty() || format == PackFormat::TarZst)
        {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "this archive cannot be encrypted with this password",
            ));
        }

        Ok(match format {
            PackFormat::Zip => Self::Zip(ZipWriter::new(out, options, cancel)),
            PackFormat::TarZst => Self::TarZst(TarWriter::new(out, options.level, cancel)?),
            PackFormat::SevenZ => Self::SevenZ(SevenZWriter::new(out, options, cancel)?),
        })
    }

    pub fn add(&mut self, entry: Entry<'a>) -> io::Result<()> {
        match self {
            Self::Zip(writer) => writer.add(entry),
            Self::TarZst(writer) => writer.add(entry),
            Self::SevenZ(writer) => writer.add(entry),
        }
    }

    /// Writes what the archive still lacks (the central directory, the last block) and gives `out` back.
    pub fn finish(self) -> io::Result<W> {
        match self {
            Self::Zip(writer) => writer.finish(),
            Self::TarZst(writer) => writer.finish(),
            Self::SevenZ(writer) => writer.finish(),
        }
    }
}

/// The deflate level of a zip entry (`None` is stored, not compressed).
pub fn deflate_level(level: PackLevel) -> Option<u32> {
    match level {
        PackLevel::Store => None,
        PackLevel::Fast => Some(1),
        PackLevel::Normal => Some(6),
        PackLevel::Maximum => Some(9),
    }
}

/// The Zstandard level of a tar.zst; it has no stored mode, so `Store` is the normal level.
pub fn zstd_level(level: PackLevel) -> i32 {
    match level {
        PackLevel::Fast => 1,
        PackLevel::Store | PackLevel::Normal => 3,
        PackLevel::Maximum => 19,
    }
}

/// The LZMA2 preset of a 7z (`None` is COPY, not compressed).
pub fn lzma_preset(level: PackLevel) -> Option<u32> {
    match level {
        PackLevel::Store => None,
        PackLevel::Fast => Some(1),
        PackLevel::Normal => Some(5),
        PackLevel::Maximum => Some(9),
    }
}

/// Counts the entries and names of an archive against what the readers take (see [`MAX_INDEX_ENTRIES`]), and checks each
/// name.
struct Budget {
    entries: usize,
    name_bytes: usize,
}

impl Budget {
    fn new() -> Self {
        Self {
            entries: 0,
            name_bytes: 0,
        }
    }

    /// Takes one more entry named `name`, or fails.
    fn admit(&mut self, name: &str) -> io::Result<()> {
        if !matches!(normalize(name), Ok(Some(ref plain)) if plain == name) {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "an entry has a name that cannot be stored in an archive",
            ));
        }

        self.entries += 1;
        self.name_bytes += name.len();

        if self.entries > MAX_INDEX_ENTRIES || self.name_bytes > MAX_INDEX_NAME_BYTES {
            return Err(refusal(
                io::ErrorKind::InvalidInput,
                "there are too many entries for one archive",
            ));
        }

        Ok(())
    }
}

/// Copies `reader` to `sink` in chunks until the reader ends, looking at `cancel` before each and giving each chunk to
/// `seen` first; returns the number of bytes. A read that fails fails the copy, `Interrupted` too.
pub(crate) fn pump(
    reader: &mut dyn Read,
    sink: &mut dyn Write,
    cancel: &AtomicBool,
    mut seen: impl FnMut(&[u8]),
) -> io::Result<u64> {
    let mut buffer = vec![0; CHUNK];
    let mut total = 0;

    loop {
        check(cancel)?;

        let count = reader.read(&mut buffer)?;

        if count == 0 {
            return Ok(total);
        }

        seen(&buffer[..count]);
        sink.write_all(&buffer[..count])?;
        total += count as u64;
    }
}

/// Appends numbers, little-endian (the byte order of the zip headers).
trait Put {
    fn put(&mut self, bytes: &[u8]) -> &mut Self;
    fn le16(&mut self, number: u16) -> &mut Self;
    fn le32(&mut self, number: u32) -> &mut Self;
    fn le64(&mut self, number: u64) -> &mut Self;
}

impl Put for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) -> &mut Self {
        self.extend_from_slice(bytes);

        self
    }

    fn le16(&mut self, number: u16) -> &mut Self {
        self.put(&number.to_le_bytes())
    }

    fn le32(&mut self, number: u32) -> &mut Self {
        self.put(&number.to_le_bytes())
    }

    fn le64(&mut self, number: u64) -> &mut Self {
        self.put(&number.to_le_bytes())
    }
}

/// A reader that opens its source at the first read and drops it at the end of the data, so an entry waiting for its
/// turn holds no file. If opening fails the read fails with that error, and the reader then reads as empty.
pub struct Lazy<R, F> {
    open: Option<F>,
    reader: Option<R>,
}

impl<R: Read, F: FnOnce() -> io::Result<R>> Lazy<R, F> {
    pub fn new(open: F) -> Self {
        Self {
            open: Some(open),
            reader: None,
        }
    }
}

impl<R: Read, F: FnOnce() -> io::Result<R>> Read for Lazy<R, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        if let Some(open) = self.open.take() {
            self.reader = Some(open()?);
        }

        let Some(reader) = self.reader.as_mut() else {
            return Ok(0);
        };
        let count = reader.read(buffer)?;

        if count == 0 {
            self.reader = None;
        }

        Ok(count)
    }
}

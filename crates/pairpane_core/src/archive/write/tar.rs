//! The tar writer: a tar of ours (GNU headers, long names through the GNU extension, owner 0 and no user or group
//! names, so nothing of the machine is in it) through a Zstandard encoder, or, for the rewrite of an archive (see
//! `crate::archive::rewrite`), through the encoder of any of the compressions a tar comes in, each at its default level.
//! Entries are never added from a path (`tar`'s `append_path` follows links and reads files itself): the header is written
//! by the builder, with the size the entry declares and no data, and the data follows through a copy loop of ours that
//! looks at the cancel flag.

use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;

use bzip2::write::BzEncoder;
use flate2::write::GzEncoder;
use lzma_rust2::{XzOptions, XzWriter};
use tar::{Builder, EntryType, Header};
use zstd::stream::write::Encoder;

use crate::archive::error::{check, unsupported};
use crate::model::{ArchiveFormat, PackLevel};
use crate::view::refusal;

use super::{Budget, Content, Entry, pump, zstd_level};

/// The size of a block of a tar: data is padded to a whole number of them.
const BLOCK: u64 = 512;

/// The preset of the XZ and the level of the gzip and bzip2 encoders of a rewritten tar: each codec's own default.
const DEFAULT_PRESET: u32 = 6;

/// Where a tar is written to: as it is, or through the encoder of its compression.
enum Sink<W: Write> {
    Plain(W),
    Gz(GzEncoder<W>),
    Xz(Box<XzWriter<W>>),
    Zst(Encoder<'static, W>),
    Bz2(BzEncoder<W>),
}

impl<W: Write> Sink<W> {
    /// The sink of the tar format `format` in `out`. Only Zstandard has a level that is chosen (by `level`).
    fn new(format: ArchiveFormat, out: W, level: PackLevel) -> io::Result<Self> {
        Ok(match format {
            ArchiveFormat::Tar => Self::Plain(out),
            ArchiveFormat::TarGz => Self::Gz(GzEncoder::new(out, flate2::Compression::new(DEFAULT_PRESET))),
            ArchiveFormat::TarXz => Self::Xz(Box::new(XzWriter::new(out, XzOptions::with_preset(DEFAULT_PRESET))?)),
            ArchiveFormat::TarZst => {
                let mut encoder = Encoder::new(out, zstd_level(level))?;
                encoder.include_checksum(true)?;

                Self::Zst(encoder)
            }
            ArchiveFormat::TarBz2 => Self::Bz2(BzEncoder::new(out, bzip2::Compression::new(DEFAULT_PRESET))),
            ArchiveFormat::Zip | ArchiveFormat::Gz | ArchiveFormat::SevenZ | ArchiveFormat::Rar => {
                return Err(unsupported("this is not a tar"));
            }
        })
    }

    /// Ends the compressed stream and gives `out` back.
    fn finish(self) -> io::Result<W> {
        match self {
            Self::Plain(out) => Ok(out),
            Self::Gz(encoder) => encoder.finish(),
            Self::Xz(encoder) => encoder.finish(),
            Self::Zst(encoder) => encoder.finish(),
            Self::Bz2(encoder) => encoder.finish(),
        }
    }
}

impl<W: Write> Write for Sink<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(out) => out.write(bytes),
            Self::Gz(encoder) => encoder.write(bytes),
            Self::Xz(encoder) => encoder.write(bytes),
            Self::Zst(encoder) => encoder.write(bytes),
            Self::Bz2(encoder) => encoder.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(out) => out.flush(),
            Self::Gz(encoder) => encoder.flush(),
            Self::Xz(encoder) => encoder.flush(),
            Self::Zst(encoder) => encoder.flush(),
            Self::Bz2(encoder) => encoder.flush(),
        }
    }
}

pub struct TarWriter<'a, W: Write> {
    builder: Builder<Sink<W>>,
    budget: Budget,
    cancel: &'a AtomicBool,
}

impl<'a, W: Write> TarWriter<'a, W> {
    /// A tar.zst in `out`, which Zstandard compresses at the level of `level`.
    pub fn new(out: W, level: PackLevel, cancel: &'a AtomicBool) -> io::Result<Self> {
        Self::with_format(ArchiveFormat::TarZst, out, level, cancel)
    }

    /// A tar of the format `format` (a tar, or a tar in one of its compressions) in `out`.
    pub fn with_format(format: ArchiveFormat, out: W, level: PackLevel, cancel: &'a AtomicBool) -> io::Result<Self> {
        let mut builder = Builder::new(Sink::new(format, out, level)?);
        // Nothing here adds from a path; a link that is somehow met is a link and is not read through.
        builder.follow_symlinks(false);

        Ok(Self {
            builder,
            budget: Budget::new(),
            cancel,
        })
    }

    pub fn add(&mut self, entry: Entry<'a>) -> io::Result<()> {
        self.budget.admit(&entry.name)?;
        check(self.cancel)?;

        let mut header = Header::new_gnu();
        header.set_mode(entry.mode & 0o777);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(
            entry
                .modified
                .map_or(0, |ms| u64::try_from(ms.div_euclid(1000)).unwrap_or(0)),
        );

        match entry.content {
            Content::Folder => {
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
                self.builder
                    .append_data(&mut header, format!("{}/", entry.name), io::empty())
            }
            Content::File { size, reader } => {
                header.set_entry_type(EntryType::Regular);
                header.set_size(size);
                // The header (and the long name's) with no data: the builder pads nothing for it.
                self.builder.append_data(&mut header, &entry.name, io::empty())?;

                self.data(size, reader)
            }
        }
    }

    /// Writes `size` bytes of `reader` and the padding. A file that ends early fails: the header already says its size.
    fn data(&mut self, size: u64, reader: Box<dyn Read + 'a>) -> io::Result<()> {
        let mut limited = reader.take(size);
        let sink = self.builder.get_mut();

        if pump(&mut limited, sink, self.cancel, |_| {})? != size {
            return Err(refusal(
                io::ErrorKind::UnexpectedEof,
                "a file changed while it was being packed",
            ));
        }

        let padding = (BLOCK - size % BLOCK) % BLOCK;
        sink.write_all(&[0; BLOCK as usize][..padding as usize])
    }

    /// Where bytes go as they are, past the encoder of the compression: what a rewrite copies of the entries it keeps.
    pub fn raw(&mut self) -> &mut dyn Write {
        self.builder.get_mut()
    }

    /// Writes the end of the tar and of the compressed stream, and gives `out` back.
    pub fn finish(self) -> io::Result<W> {
        self.builder.into_inner()?.finish()
    }
}

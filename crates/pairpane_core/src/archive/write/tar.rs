//! The tar.zst writer: a tar of ours (GNU headers, long names through the GNU extension, owner 0 and no user or group
//! names, so nothing of the machine is in it) through a Zstandard encoder. Entries are never added from a path (`tar`'s
//! `append_path` follows links and reads files itself): the header is written by the builder, with the size the entry
//! declares and no data, and the data follows through a copy loop of ours that looks at the cancel flag.

use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;

use tar::{Builder, EntryType, Header};
use zstd::stream::write::Encoder;

use crate::archive::error::check;
use crate::model::PackLevel;
use crate::view::refusal;

use super::{Budget, Content, Entry, pump, zstd_level};

/// The size of a block of a tar: data is padded to a whole number of them.
const BLOCK: u64 = 512;

pub struct TarWriter<'a, W: Write> {
    builder: Builder<Encoder<'static, W>>,
    budget: Budget,
    cancel: &'a AtomicBool,
}

impl<'a, W: Write> TarWriter<'a, W> {
    pub fn new(out: W, level: PackLevel, cancel: &'a AtomicBool) -> io::Result<Self> {
        let mut encoder = Encoder::new(out, zstd_level(level))?;
        encoder.include_checksum(true)?;

        let mut builder = Builder::new(encoder);
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

    /// Writes the end of the tar and of the Zstandard frame, and gives `out` back.
    pub fn finish(self) -> io::Result<W> {
        self.builder.into_inner()?.finish()
    }
}

//! The 7z writer, on `sevenz_rust2`'s `ArchiveWriter`. Files go into solid blocks of about 256 MiB of input (each entry's
//! reader waits in the writer until its block is written, so the readers should be [`Lazy`](super::Lazy)); folders and
//! empty files are entries with no stream. LZMA2 is at the preset of the level with a dictionary of at most 32 MiB
//! (what bounds the memory of a pack); the lowest level, `Store`, is COPY.
//!
//! Encryption: AES-256 with a key of 2^19 rounds (the library's own constructor takes 2^8 and panics when the system has no
//! random numbers). The salt is made once for the archive and a fresh IV for each block, and again before the headers are
//! written (they are encrypted too when `encrypt_names` says so); the library keeps the settings of its last call to
//! [`ArchiveWriter::set_content_methods`], so every block and the header ask for their own.

use std::io::{self, Read, Seek, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use sevenz_rust2::encoder_options::{AesEncoderOptions, EncoderOptions, Lzma2Options};
use sevenz_rust2::{
    Archive, ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, Error, NtTime, SourceReader,
};

use crate::archive::Password;
use crate::archive::error::{cancelled, check};
use crate::archive::sevenz::{
    ATTRIBUTE_DIRECTORY, ATTRIBUTE_UNIX_EXTENSION, EPOCH_GAP_MS, MODE_DIRECTORY, MODE_REGULAR, TICKS_PER_MILLISECOND,
};
use crate::view::refusal;

use super::{Budget, Content, Entry, Options, lzma_preset};

/// The input of a solid block: the entries wait until they add up to this, or the archive ends.
const BLOCK: u64 = 256 * 1024 * 1024;

/// The largest LZMA2 dictionary: level 9 asks 64 MiB, and a pack would take over 600 MiB of memory with it.
const DICTIONARY: u32 = 32 * 1024 * 1024;

/// The rounds of the key derivation, as a power of two (7-Zip's own is 19).
const CYCLES_POWER: u8 = 19;

/// The Windows "archive" attribute of a file.
const ATTRIBUTE_ARCHIVE: u32 = 0x20;

/// A reader that fails with `Interrupted` once `cancel` is raised, and fails when the file gives more than the size it was
/// declared with (it reads one byte more to know). The library reads it directly, no `io::copy` between, so the error ends
/// the block.
struct Checked<'a> {
    inner: Box<dyn Read + 'a>,
    cancel: &'a AtomicBool,
    /// How many bytes may still be read: the declared size and one more.
    left: u64,
}

impl Read for Checked<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        let room = usize::try_from(self.left).map_or(buffer.len(), |left| left.min(buffer.len()));
        let count = self.inner.read(&mut buffer[..room])?;
        self.left -= count as u64;

        if self.left == 0 {
            return Err(refusal(io::ErrorKind::Other, "a file grew while it was being packed"));
        }

        Ok(count)
    }
}

pub struct SevenZWriter<'a, W: Write + Seek> {
    writer: ArchiveWriter<W>,
    preset: Option<u32>,
    password: Option<Password>,
    salt: [u8; 16],
    /// The entries of the block that is being filled, and their readers.
    entries: Vec<ArchiveEntry>,
    readers: Vec<SourceReader<Checked<'a>>>,
    waiting: u64,
    /// How much input fills a block ([`BLOCK`]; the tests make it small).
    block: u64,
    budget: Budget,
    cancel: &'a AtomicBool,
}

impl<'a, W: Write + Seek> SevenZWriter<'a, W> {
    /// A 7z in `out`, which must be empty and at its start.
    pub fn new(out: W, options: &Options, cancel: &'a AtomicBool) -> io::Result<Self> {
        let mut salt = [0; 16];
        random(&mut salt)?;

        let mut writer = ArchiveWriter::new(out).map_err(|error| failure(error, cancel))?;
        writer.set_encrypt_header(options.encrypt_names);

        let mut this = Self {
            writer,
            preset: lzma_preset(options.level),
            password: options.password.clone(),
            salt,
            entries: Vec::new(),
            readers: Vec::new(),
            waiting: 0,
            block: BLOCK,
            budget: Budget::new(),
            cancel,
        };
        this.renew()?;

        Ok(this)
    }

    /// Closes a block at `block` bytes of input instead of [`BLOCK`], to have many blocks of a small input.
    #[cfg(test)]
    pub(crate) fn with_block(mut self, block: u64) -> Self {
        self.block = block;

        self
    }

    pub fn add(&mut self, entry: Entry<'a>) -> io::Result<()> {
        self.budget.admit(&entry.name)?;
        check(self.cancel)?;

        let (mut item, size, reader) = match entry.content {
            Content::Folder => {
                let item = ArchiveEntry::new_directory(&entry.name);

                (item, 0, Box::new(io::empty()) as Box<dyn Read + 'a>)
            }
            Content::File { size, reader } if size > 0 => (ArchiveEntry::new_file(&entry.name), size, reader),
            Content::File { .. } => {
                let mut item = ArchiveEntry::new_file(&entry.name);
                item.has_stream = false;

                (item, 0, Box::new(io::empty()) as Box<dyn Read + 'a>)
            }
        };

        item.has_windows_attributes = true;
        item.windows_attributes = attributes(item.is_directory, entry.mode);

        if let Some(ticks) = entry.modified.and_then(ticks) {
            item.has_last_modified_date = true;
            item.last_modified_date = NtTime::from(ticks);
        }

        self.entries.push(item);
        self.readers.push(SourceReader::new(Checked {
            inner: reader,
            cancel: self.cancel,
            left: size.saturating_add(1),
        }));
        self.waiting += size;

        if self.waiting >= self.block {
            self.flush()?;
        }

        Ok(())
    }

    /// Copies the block `block` of `archive` (which was read from `source`) as it is, with its entries: its packed bytes
    /// are not decoded. `false` when the library refuses the block (it has no checksum that the writer would have to make
    /// up): nothing was written then, and the caller adds its entries with [`SevenZWriter::add_decoded`].
    pub fn copy_block<R: Read + Seek>(&mut self, archive: &Archive, block: usize, source: &mut R) -> io::Result<bool> {
        self.flush()?;

        match self
            .writer
            .push_raw_block(archive, block, source, |entry| entry.name.clone())
        {
            Ok(_) => Ok(true),
            Err(Error::Other(_)) => Ok(false),
            Err(error) => Err(failure(error, self.cancel)),
        }
    }

    /// Adds an entry of another archive as it is (its attributes and times) with its decoded `data`, in a block of its own
    /// (with a password, under a new IV); an entry with no data is added as one with no stream.
    pub fn add_decoded(&mut self, item: ArchiveEntry, data: Option<&mut dyn Read>) -> io::Result<()> {
        self.flush()?;
        check(self.cancel)?;

        if data.is_some() {
            self.renew()?;
        }

        self.writer
            .push_archive_entry(item, data)
            .map_err(|error| failure(error, self.cancel))?;

        Ok(())
    }

    /// Writes the entries that wait: a block, or, when none of them has data, plain entries with no stream.
    fn flush(&mut self) -> io::Result<()> {
        let entries = std::mem::take(&mut self.entries);
        let readers = std::mem::take(&mut self.readers);
        self.waiting = 0;

        if entries.is_empty() {
            return Ok(());
        }

        if entries.iter().all(|entry| !entry.has_stream) {
            for entry in entries {
                self.writer
                    .push_archive_entry(entry, None::<io::Empty>)
                    .map_err(|error| failure(error, self.cancel))?;
            }

            return Ok(());
        }

        self.renew()?;
        self.writer
            .push_archive_entries(entries, readers)
            .map_err(|error| failure(error, self.cancel))?;

        Ok(())
    }

    /// Gives the library the methods for what it writes next: with a password, AES with a new IV and the archive's salt, over
    /// the compression.
    fn renew(&mut self) -> io::Result<()> {
        let compression = match self.preset {
            None => EncoderConfiguration::new(EncoderMethod::COPY),
            Some(preset) => {
                let mut options = Lzma2Options::from_level(preset);
                options.set_dictionary_size(dictionary(preset));

                EncoderConfiguration::new(EncoderMethod::LZMA2).with_options(EncoderOptions::Lzma2(options))
            }
        };

        let Some(password) = &self.password else {
            self.writer.set_content_methods(vec![compression]);

            return Ok(());
        };

        let mut iv = [0; 16];
        random(&mut iv)?;

        // Built field by field: the library's constructor weakens the key and panics without random numbers.
        let aes = AesEncoderOptions {
            password: sevenz_rust2::Password::from(password.as_str()),
            iv,
            salt: self.salt,
            num_cycles_power: CYCLES_POWER,
        };
        self.writer.set_content_methods(vec![aes.into(), compression]);

        Ok(())
    }

    /// Writes the last block and the headers, and gives the file back.
    pub fn finish(mut self) -> io::Result<W> {
        self.flush()?;
        // The headers are encrypted with the settings that were last given: a fresh IV for them too.
        self.renew()?;

        self.writer.finish()
    }
}

/// `bytes` of random numbers from the system, or an error (nothing is encrypted with a key that is not random).
fn random(bytes: &mut [u8]) -> io::Result<()> {
    getrandom::fill(bytes)
        .map_err(|_| refusal(io::ErrorKind::Other, "the system has no random numbers to encrypt with"))
}

/// The dictionary of the LZMA2 preset, at most [`DICTIONARY`].
fn dictionary(preset: u32) -> u32 {
    lzma_rust2::Lzma2Options::with_preset(preset)
        .lzma_options
        .dict_size
        .min(DICTIONARY)
}

/// The attributes of an entry: the Unix mode in the high half on Unix, as p7zip writes it, and the Windows bits.
fn attributes(folder: bool, mode: u32) -> u32 {
    let windows = if folder { ATTRIBUTE_DIRECTORY } else { ATTRIBUTE_ARCHIVE };

    if cfg!(windows) {
        return windows;
    }

    let kind = if folder { MODE_DIRECTORY } else { MODE_REGULAR };

    windows | ATTRIBUTE_UNIX_EXTENSION | ((kind | (mode & 0o777)) << 16)
}

/// `modified` (milliseconds since the Unix epoch) as 100 ns ticks since 1601; `None` before 1601.
fn ticks(modified: i64) -> Option<u64> {
    let milliseconds = modified.checked_add(EPOCH_GAP_MS)?;

    u64::try_from(milliseconds).ok()?.checked_mul(TICKS_PER_MILLISECOND)
}

/// An error of the library as ours: a raised cancel flag first, then the I/O error the library wrapped (our reader's
/// `Interrupted`, or the disk's), else a fixed text.
fn failure(error: Error, cancel: &AtomicBool) -> io::Error {
    if let Err(stopped) = check(cancel) {
        return stopped;
    }

    match error {
        Error::Io(source, _) => source,
        _ => refusal(io::ErrorKind::Other, "the 7z archive could not be written"),
    }
}

//! The zip writer. Each entry is written as its local header (CRC and sizes zero), its data, and then the header is
//! sought back to and patched with the CRC and sizes: no data descriptor (flag bit 3), so every reader finds the sizes
//! where it expects them. The central directory is kept in memory and written at the end.
//!
//! Zip64: a size or offset that does not fit 32 bits goes in a zip64 extra field of the central record (only the fields that
//! overflow, in the order size, compressed size, offset). The local header cannot grow once the data follows it, so an
//! entry whose size says it may need the field (stat's size at [`Limits::reserve`] or more) gets a zip64 slot in the local
//! header from the start, patched like the sizes. A file that outgrows 32 bits without a slot fails the pack. More
//! entries than fit 16 bits, or a directory that does not fit 32 bits, add the zip64 end record and its locator.
//!
//! Names are UTF-8 (flag bit 11), the time is the extended timestamp field (`0x5455`) next to the DOS fields (UTC).
//! Content is stored or deflated; with a password every file is WinZip AES-256 (AE-2: no CRC, the method in the `0x9901`
//! field, a random salt for each entry).

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;

use crc32fast::Hasher;

use crate::archive::Password;
use crate::archive::error::{check, unreadable};
use crate::archive::zip::winzip_aes::Sealing;
use crate::view::refusal;

use super::zip_data::{Counting, deflated};
use super::{Budget, Content, Entry, Options, Put, deflate_level, dos, pump};

/// The sentinel of a 32-bit field that is in the zip64 field instead, and of a 16-bit count.
const WIDE_32: u64 = 0xFFFF_FFFF;
const WIDE_16: u64 = 0xFFFF;

/// Flag bit 0: encrypted. Bit 11: the name is UTF-8.
const ENCRYPTED: u16 = 1;
const UTF8_NAME: u16 = 1 << 11;

const STORED: u16 = 0;
const DEFLATE: u16 = 8;
/// The method of a WinZip AES entry; the real one is in the extra field.
const AES_METHOD: u16 = 99;

/// What is needed to read an entry: the plain, deflate, zip64 and AES versions.
const VERSION_STORED: u16 = 10;
const VERSION_DEFLATE: u16 = 20;
const VERSION_ZIP64: u16 = 45;
const VERSION_AES: u16 = 51;

/// The host in the high byte of "version made by": Unix (3), or DOS (0), which is what a Windows writer says.
const HOST: u16 = if cfg!(windows) { 0 } else { 3 };

const DOS_FOLDER: u32 = 0x10;
const DOS_ARCHIVE: u32 = 0x20;
const UNIX_FOLDER: u32 = 0o040_000;
const UNIX_FILE: u32 = 0o100_000;

/// Where a number stops fitting its field. They are the format's own and are changed only by the tests, which make the
/// zip64 paths reachable with small files.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A size (and the size of the directory) at this value or more is written as zip64.
    pub size: u64,
    /// An offset of a local header or of the directory at this value or more is written as zip64.
    pub offset: u64,
    /// This many entries or more make the zip64 end record.
    pub entries: u64,
    /// A file this big (by its stat) gets a zip64 slot in its local header: it may be more than `size` once compressed
    /// and encrypted, and the header cannot be rewritten longer. Below `size` by the most a codec can add.
    pub reserve: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            size: WIDE_32,
            offset: WIDE_32,
            entries: WIDE_16,
            // Deflate adds a little to data that does not compress (0.03%), 128 MiB is far more than that for a file near 4 GiB.
            reserve: WIDE_32 - 128 * 1024 * 1024,
        }
    }
}

/// How an entry is written, known before its data is.
struct Plan {
    /// The name as stored (a folder's ends in `/`).
    name: Vec<u8>,
    folder: bool,
    mode: u32,
    modified: Option<i64>,
    /// The method of the data before it is encrypted.
    method: u16,
    encrypted: bool,
    /// Whether the local header has a zip64 slot.
    slot: bool,
    /// The fields both headers have: the timestamp and the AES one.
    shared: Vec<u8>,
}

/// The numbers only known after the data is written.
#[derive(Default)]
struct Outcome {
    crc: u32,
    size: u64,
    compressed: u64,
}

pub struct ZipWriter<'a, W: Write + Seek> {
    out: W,
    /// Where the next byte goes (the file is sought back from here and to here).
    position: u64,
    directory: Vec<u8>,
    entries: u64,
    deflate: Option<u32>,
    password: Option<Password>,
    limits: Limits,
    budget: Budget,
    cancel: &'a AtomicBool,
    /// The comment of the archive, written in the end record.
    comment: Vec<u8>,
}

impl<'a, W: Write + Seek> ZipWriter<'a, W> {
    /// A zip in `out`, which must be empty and at its start.
    pub fn new(out: W, options: &Options, cancel: &'a AtomicBool) -> Self {
        Self::with_limits(out, options, cancel, Limits::default())
    }

    pub fn with_limits(out: W, options: &Options, cancel: &'a AtomicBool, limits: Limits) -> Self {
        Self {
            out,
            position: 0,
            directory: Vec::new(),
            entries: 0,
            deflate: deflate_level(options.level),
            password: options.password.clone(),
            limits,
            budget: Budget::new(),
            cancel,
            comment: Vec::new(),
        }
    }

    /// Gives the archive the comment `comment` (the end record has 16 bits for its length: what does not fit is cut).
    #[must_use]
    pub fn with_comment(mut self, mut comment: Vec<u8>) -> Self {
        comment.truncate(usize::from(u16::MAX));
        self.comment = comment;

        self
    }

    pub fn add(&mut self, entry: Entry<'a>) -> io::Result<()> {
        self.budget.admit(&entry.name)?;
        check(self.cancel)?;

        let folder = matches!(entry.content, Content::Folder);
        let mut name = entry.name.into_bytes();
        if folder {
            name.push(b'/');
        }

        let (stat, body) = match entry.content {
            Content::Folder => (0, None),
            Content::File { size, reader } => (size, Some(reader)),
        };
        // A folder has no data and no password; an empty file has no data but is encrypted like the rest.
        let encrypted = body.is_some() && self.password.is_some();
        let deflate = self.deflate.filter(|_| stat > 0);

        let mut plan = Plan {
            name,
            folder,
            mode: entry.mode,
            modified: entry.modified,
            method: if deflate.is_some() { DEFLATE } else { STORED },
            encrypted,
            slot: stat >= self.limits.reserve,
            shared: timestamp(entry.modified),
        };
        if encrypted {
            // The AES field: AE-2, AES-256 and the method of the data.
            plan.shared
                .le16(0x9901)
                .le16(7)
                .le16(2)
                .put(b"AE")
                .put(&[3])
                .le16(plan.method);
        }

        let start = self.position;
        self.local_header(&plan)?;

        let done = match body {
            // One byte more than declared is read: a file that gives more has grown, and is not cut short.
            Some(reader) if stat > 0 => self.data(&mut reader.take(stat.saturating_add(1)), deflate, encrypted)?,
            Some(_) => self.data(&mut io::empty(), None, encrypted)?,
            None => Outcome::default(),
        };
        if done.size > stat {
            return Err(grew());
        }

        self.patch(start, &plan, &done)?;

        let record = self.central(&plan, start, &done);
        self.directory.extend_from_slice(&record);
        self.entries += 1;

        Ok(())
    }

    /// Adds an entry of another zip as it is: `length` bytes of `from`, which are its local header, its data and its data
    /// descriptor, if it has one. `record` makes the entry's record of the central directory from the offset the copy
    /// starts at (and the limit from which an offset is a zip64 one); it is the caller's to see that it matches the bytes.
    pub fn copy_entry(
        &mut self,
        from: &mut dyn Read,
        length: u64,
        record: impl FnOnce(u64, u64) -> io::Result<Vec<u8>>,
    ) -> io::Result<()> {
        check(self.cancel)?;

        let record = record(self.position, self.limits.offset)?;
        let copied = pump(&mut from.take(length), &mut self.out, self.cancel, |_| {})?;

        if copied != length {
            return Err(unreadable());
        }

        self.position += copied;
        self.directory.extend_from_slice(&record);
        self.entries += 1;

        Ok(())
    }

    /// The local header, with zeros (or ones for a zip64 slot) where the CRC and the sizes go.
    fn local_header(&mut self, plan: &Plan) -> io::Result<()> {
        let (date, time) = dos::fields(plan.modified);
        let wide = if plan.slot { WIDE_32 as u32 } else { 0 };

        let mut extra = Vec::new();
        if plan.slot {
            extra.le16(1).le16(16).le64(0).le64(0);
        }
        extra.put(&plan.shared);

        let mut local = b"PK\x03\x04".to_vec();
        local
            .le16(needed(plan.method, plan.encrypted, plan.slot))
            .le16(flags(plan.encrypted))
            .le16(code(plan))
            .le16(time)
            .le16(date)
            .le32(0)
            .le32(wide)
            .le32(wide)
            .le16(plan.name.len() as u16)
            .le16(extra.len() as u16)
            .put(&plan.name)
            .put(&extra);
        self.out.write_all(&local)?;
        self.position += local.len() as u64;

        Ok(())
    }

    /// Writes the data of an entry (deflated or as it is, then encrypted with the password if `encrypted`) and counts it.
    fn data(&mut self, reader: &mut dyn Read, deflate: Option<u32>, encrypted: bool) -> io::Result<Outcome> {
        let mut counted = Counting {
            inner: &mut self.out,
            written: 0,
        };
        let mut crc = Hasher::new();

        let size = match self.password.as_ref().filter(|_| encrypted) {
            Some(password) => {
                let mut sealing = Sealing::start(&mut counted, password)?;
                let size = deflated(reader, &mut sealing, deflate, self.cancel, &mut crc)?;
                sealing.finish()?;

                size
            }
            None => deflated(reader, &mut counted, deflate, self.cancel, &mut crc)?,
        };

        self.position += counted.written;

        Ok(Outcome {
            crc: crc.finalize(),
            size,
            compressed: counted.written,
        })
    }

    /// Seeks back to the local header at `start` to write the CRC and the sizes (and the zip64 slot), then to where the
    /// file ended.
    fn patch(&mut self, start: u64, plan: &Plan, done: &Outcome) -> io::Result<()> {
        let wide = WIDE_32 as u32;

        if !plan.slot && (done.size >= self.limits.size || done.compressed >= self.limits.size) {
            return Err(grew());
        }

        let mut fields = Vec::new();
        fields
            // AE-2 has no CRC.
            .le32(if plan.encrypted { 0 } else { done.crc })
            .le32(if plan.slot { wide } else { done.compressed as u32 })
            .le32(if plan.slot { wide } else { done.size as u32 });
        self.out.seek(SeekFrom::Start(start + 14))?;
        self.out.write_all(&fields)?;

        if plan.slot {
            let mut sizes = Vec::new();
            sizes.le64(done.size).le64(done.compressed);
            // Past the header, the name and the field's own header.
            self.out
                .seek(SeekFrom::Start(start + 30 + plan.name.len() as u64 + 4))?;
            self.out.write_all(&sizes)?;
        }

        self.out.seek(SeekFrom::Start(self.position))?;

        Ok(())
    }

    /// The record of the entry for the central directory.
    fn central(&self, plan: &Plan, start: u64, done: &Outcome) -> Vec<u8> {
        let (date, time) = dos::fields(plan.modified);

        // What does not fit is in the zip64 field instead, in this order.
        let mut wide = Vec::new();
        let size = fit(&mut wide, done.size, self.limits.size);
        let compressed = fit(&mut wide, done.compressed, self.limits.size);
        let offset = fit(&mut wide, start, self.limits.offset);

        let mut extra = Vec::new();
        if !wide.is_empty() {
            extra.le16(1).le16(wide.len() as u16).put(&wide);
        }
        extra.put(&plan.shared);

        let mut record = b"PK\x01\x02".to_vec();
        record
            .le16(HOST << 8 | VERSION_ZIP64)
            .le16(needed(plan.method, plan.encrypted, plan.slot || !wide.is_empty()))
            .le16(flags(plan.encrypted))
            .le16(code(plan))
            .le16(time)
            .le16(date)
            .le32(if plan.encrypted { 0 } else { done.crc })
            .le32(compressed)
            .le32(size)
            .le16(plan.name.len() as u16)
            .le16(extra.len() as u16)
            // No comment, disk 0, no internal attributes.
            .put(&[0; 6])
            .le32(attributes(plan))
            .le32(offset)
            .put(&plan.name)
            .put(&extra);

        record
    }

    /// Writes the central directory and the end records, and gives the file back.
    pub fn finish(mut self) -> io::Result<W> {
        let start = self.position;
        let size = self.directory.len() as u64;
        let wide = self.entries >= self.limits.entries || size >= self.limits.size || start >= self.limits.offset;

        let mut end = std::mem::take(&mut self.directory);

        if wide {
            let record = start + size;

            // The zip64 end record (its length counts what follows the length), and the locator that points to it.
            end.put(b"PK\x06\x06")
                .le64(44)
                .le16(HOST << 8 | VERSION_ZIP64)
                .le16(VERSION_ZIP64)
                .le32(0)
                .le32(0)
                .le64(self.entries)
                .le64(self.entries)
                .le64(size)
                .le64(start)
                .put(b"PK\x06\x07")
                .le32(0)
                .le64(record)
                .le32(1);
        }

        // The end record says it with ones where the zip64 one has the number.
        let (entries, size, start) = if wide {
            (WIDE_16 as u16, WIDE_32 as u32, WIDE_32 as u32)
        } else {
            (self.entries as u16, size as u32, start as u32)
        };
        end.put(b"PK\x05\x06")
            .le32(0)
            .le16(entries)
            .le16(entries)
            .le32(size)
            .le32(start)
            .le16(self.comment.len() as u16)
            .put(&self.comment);

        self.out.write_all(&end)?;
        self.out.flush()?;

        Ok(self.out)
    }
}

/// The error of a file that gave more than the size it was declared with.
fn grew() -> io::Error {
    refusal(io::ErrorKind::Other, "a file grew while it was being packed")
}

/// The flags of an entry.
fn flags(encrypted: bool) -> u16 {
    if encrypted { UTF8_NAME | ENCRYPTED } else { UTF8_NAME }
}

/// The method the headers say: that of a WinZip AES entry when it is encrypted.
fn code(plan: &Plan) -> u16 {
    if plan.encrypted { AES_METHOD } else { plan.method }
}

/// The external attributes: a Unix mode (and the DOS folder bit) on Unix, DOS attributes on Windows.
fn attributes(plan: &Plan) -> u32 {
    let permissions = plan.mode & 0o777;

    match (cfg!(windows), plan.folder) {
        (true, true) => DOS_FOLDER,
        (true, false) => DOS_ARCHIVE,
        (false, true) => (UNIX_FOLDER | permissions) << 16 | DOS_FOLDER,
        (false, false) => (UNIX_FILE | permissions) << 16,
    }
}

/// `value` as a 32-bit field: itself when it is below `limit`, else the ones that say "see the zip64 field", with
/// the number appended to `wide`.
fn fit(wide: &mut Vec<u8>, value: u64, limit: u64) -> u32 {
    if value < limit {
        return value as u32;
    }

    wide.le64(value);

    WIDE_32 as u32
}

/// The extended timestamp field (`0x5455`) with the modification time, when it fits 32 bits; nothing otherwise.
fn timestamp(modified: Option<i64>) -> Vec<u8> {
    let mut field = Vec::new();

    if let Some(seconds) = modified.and_then(|ms| i32::try_from(ms.div_euclid(1000)).ok()) {
        field.le16(0x5455).le16(5).put(&[1]).put(&seconds.to_le_bytes());
    }

    field
}

/// The version needed to extract an entry.
fn needed(method: u16, aes: bool, zip64: bool) -> u16 {
    if aes {
        VERSION_AES
    } else if zip64 {
        VERSION_ZIP64
    } else if method == DEFLATE {
        VERSION_DEFLATE
    } else {
        VERSION_STORED
    }
}

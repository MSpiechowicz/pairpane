//! Helpers shared by the tests of the archive code and of the viewer's listing: archives built by hand.

use std::io::{self, Cursor, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use flate2::Compression;
use flate2::write::GzEncoder;
use sevenz_rust2::encoder_options::AesEncoderOptions;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, NtTime, SourceReader};

use crate::model::ArchiveFormat;

use super::file::ReadAt;
use super::{ArchiveIndex, Input, build_index, open_source};

/// 2021-03-04 05:06:08 as a zip stores it (MS-DOS date and time).
pub(crate) const DOS_DATE: u16 = 21092;
pub(crate) const DOS_TIME: u16 = 10436;
/// The same moment in milliseconds since the epoch, read as UTC.
pub(crate) const MOMENT: i64 = 1_614_834_368_000;

/// A zip of stored (not compressed) entries, written by hand: `(name, content)`; a name ending in `/` is a folder.
pub(crate) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let members: Vec<Member> = entries
        .iter()
        .map(|(name, content)| Member::stored(name, content))
        .collect();

    zip_members(&members)
}

/// One entry of a zip as its two headers say it, whatever the data is: tests make entries that are encrypted, compressed
/// or inconsistent with it.
pub(crate) struct Member<'a> {
    pub name: &'a str,
    pub method: u16,
    pub flags: u16,
    pub crc: u32,
    /// The size of the content, as declared.
    pub size: u32,
    /// The extra field, in both headers.
    pub extra: Vec<u8>,
    /// What the archive holds after the local header.
    pub data: Vec<u8>,
    /// The offset the central directory states, when it is not the real one.
    pub offset: Option<u32>,
}

impl<'a> Member<'a> {
    /// A stored entry with `content`.
    pub(crate) fn stored(name: &'a str, content: &[u8]) -> Self {
        Self {
            name,
            method: 0,
            flags: 0,
            crc: crc32fast::hash(content),
            size: content.len() as u32,
            extra: Vec::new(),
            data: content.to_vec(),
            offset: None,
        }
    }
}

/// A zip of `members`, each with the headers it states.
pub(crate) fn zip_members(members: &[Member]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut directory = Vec::new();

    for member in members {
        let offset = bytes.len() as u32;
        let compressed = member.data.len() as u32;
        // With the sizes after the data (flag bit 3) the local header holds zeros for them.
        let known = member.flags & 8 == 0;
        let (crc, sizes) = if known {
            (member.crc, [compressed, member.size])
        } else {
            (0, [0, 0])
        };

        bytes.extend_from_slice(b"PK\x03\x04");
        for field in [20, member.flags, member.method, DOS_TIME, DOS_DATE] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        for field in [crc, sizes[0], sizes[1]] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes.extend_from_slice(&(member.name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(member.extra.len() as u16).to_le_bytes());
        bytes.extend_from_slice(member.name.as_bytes());
        bytes.extend_from_slice(&member.extra);
        bytes.extend_from_slice(&member.data);

        directory.extend_from_slice(
            &Central {
                flags: member.flags,
                method: member.method,
                crc: member.crc,
                compressed,
                size: member.size,
                offset: member.offset.unwrap_or(offset),
                extra: &member.extra,
                ..Central::new(member.name.as_bytes())
            }
            .bytes(),
        );
    }

    let start = bytes.len() as u32;
    bytes.extend_from_slice(&directory);
    bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    bytes.extend_from_slice(&(members.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(members.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&start.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    bytes
}

/// A tar of `(path, content)`, a path ending in `/` being a folder, all modified at `MOMENT`.
pub(crate) fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());

    for (path, content) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mtime((MOMENT / 1000) as u64);
        header.set_mode(0o644);
        header.set_entry_type(if path.ends_with('/') {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        });
        builder.append_data(&mut header, path, *content).expect("append");
    }

    builder.into_inner().expect("a tar")
}

pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).expect("compress");

    encoder.finish().expect("finish")
}

/// A record of a central directory, field by field; [`Central::new`] has the plain values of a stored file.
pub(crate) struct Central<'a> {
    pub name: &'a [u8],
    pub made_by: u16,
    pub flags: u16,
    pub method: u16,
    pub date: u16,
    pub crc: u32,
    pub compressed: u32,
    pub size: u32,
    pub external: u32,
    pub offset: u32,
    pub extra: &'a [u8],
}

impl<'a> Central<'a> {
    pub(crate) fn new(name: &'a [u8]) -> Self {
        Self {
            name,
            made_by: 20,
            flags: 0,
            method: 0,
            date: DOS_DATE,
            crc: 0,
            compressed: 0,
            size: 0,
            external: 0,
            offset: 0,
            extra: &[],
        }
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut bytes = b"PK\x01\x02".to_vec();

        for field in [self.made_by, 20, self.flags, self.method, DOS_TIME, self.date] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        for field in [self.crc, self.compressed, self.size] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes.extend_from_slice(&(self.name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(self.extra.len() as u16).to_le_bytes());
        // No comment, disk 0, no internal attributes.
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&self.external.to_le_bytes());
        bytes.extend_from_slice(&self.offset.to_le_bytes());
        bytes.extend_from_slice(self.name);
        bytes.extend_from_slice(self.extra);

        bytes
    }
}

/// A record of a central directory with a name, general purpose `flags`, a 32-bit `size` (the compressed size too),
/// `extra` data and the DOS `date`.
pub(crate) fn record(name: &[u8], flags: u16, size: u32, extra: &[u8], date: u16) -> Vec<u8> {
    Central {
        flags,
        date,
        compressed: size,
        size,
        extra,
        ..Central::new(name)
    }
    .bytes()
}

/// A zip that is only a central directory of `records` and its end record, which declares `entries` entries.
pub(crate) fn directory_of(records: &[Vec<u8>], entries: u16) -> Vec<u8> {
    let directory = records.concat();
    let mut bytes = directory.clone();

    bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    bytes.extend_from_slice(&entries.to_le_bytes());
    bytes.extend_from_slice(&entries.to_le_bytes());
    bytes.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    bytes
}

/// An extra data field `id` holding `data`.
pub(crate) fn field(id: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = id.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
    bytes.extend_from_slice(data);

    bytes
}

/// A byte buffer is a file: tests read archives from memory.
impl super::file::ReadAt for Vec<u8> {
    fn size(&self) -> std::io::Result<u64> {
        Ok(self.len() as u64)
    }

    fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
        let Some(rest) = usize::try_from(offset).ok().and_then(|at| self.get(at..)) else {
            return Ok(0);
        };
        let count = rest.len().min(buffer.len());
        buffer[..count].copy_from_slice(&rest[..count]);

        Ok(count)
    }
}

/// A file in memory that counts the bytes read from it.
pub(crate) struct Counting {
    bytes: Vec<u8>,
    read: AtomicU64,
}

impl Counting {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            read: AtomicU64::new(0),
        }
    }

    pub(crate) fn read(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }
}

impl ReadAt for Counting {
    fn size(&self) -> io::Result<u64> {
        self.bytes.size()
    }

    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        let count = self.bytes.read_at(buffer, offset)?;
        self.read.fetch_add(count as u64, Ordering::Relaxed);

        Ok(count)
    }
}

/// A cancel flag that is down.
pub(crate) fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

/// The index of `file` read as an archive of `format`, without a password.
pub(crate) fn index_of(file: &dyn ReadAt, format: ArchiveFormat) -> io::Result<ArchiveIndex> {
    build_index(format, &Input { file, path: None }, &idle(), None)
}

/// The names in the root folder of `index`, with whether each is a folder.
pub(crate) fn root(index: &ArchiveIndex) -> Vec<(String, bool)> {
    index
        .list("")
        .expect("root")
        .iter()
        .map(|child| (child.name.to_owned(), child.dir))
        .collect()
}

/// What the source gives for the entries `wanted`: path and content.
pub(crate) fn extract(index: &ArchiveIndex, file: &dyn ReadAt, wanted: &[u32]) -> io::Result<Vec<(String, Vec<u8>)>> {
    let cancel = idle();
    let mut got = Vec::new();

    open_source(index, &Input { file, path: None }, &cancel, None)?.extract(wanted, &mut |entry, data| {
        let mut content = Vec::new();
        data.read_to_end(&mut content)?;
        got.push((entry.path.clone(), content));

        Ok(super::Flow::Continue)
    })?;

    Ok(got)
}

/// A small project: a folder, a nested folder, three files.
pub(crate) fn project() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("src/", b""),
        ("src/main.rs", b"fn main() {}\n"),
        ("src/lib/util.rs", b"pub fn util() {}\n"),
        ("README", b"read me"),
    ]
}

/// `bytes` as an XZ file.
pub(crate) fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut writer =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).expect("an xz writer");
    writer.write_all(bytes).expect("compress");

    writer.finish().expect("finish")
}

/// `bytes` as a Zstandard frame.
pub(crate) fn zstd(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

/// `bytes` as a bzip2 file.
pub(crate) fn bzip2(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(bytes).expect("compress");

    encoder.finish().expect("finish")
}

/// Noise that does not compress: `length` bytes of a fixed xorshift sequence.
pub(crate) fn noise(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;

    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;

            state as u8
        })
        .collect()
}

/// An entry of a 7z archive to write: `name` ending in `/` is a folder, empty `content` makes an empty file, and it is
/// modified at `MOMENT`.
pub(crate) fn sevenz_entry(name: &str, content: &[u8]) -> ArchiveEntry {
    let mut entry = match name.strip_suffix('/') {
        Some(folder) => ArchiveEntry::new_directory(folder),
        None => ArchiveEntry::new_file(name),
    };
    entry.has_stream = !entry.is_directory && !content.is_empty();
    entry.has_last_modified_date = true;
    // 100 ns ticks since 1601.
    entry.last_modified_date = NtTime::from((MOMENT as u64 + 11_644_473_600_000) * 10_000);

    entry
}

/// How a 7z archive is written: all in one solid block or one block each, and whether its content (and, if the flag is
/// set, its headers too) is encrypted with a password.
#[derive(Clone, Copy, Default)]
pub(crate) struct SevenZ<'a> {
    pub solid: bool,
    pub password: Option<&'a str>,
    pub encrypted_headers: bool,
}

impl SevenZ<'_> {
    /// The archive of `entries` (see [`sevenz_entry`]) with their contents.
    pub(crate) fn write(self, entries: Vec<(ArchiveEntry, &[u8])>) -> Vec<u8> {
        let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).expect("a 7z writer");

        if let Some(password) = self.password {
            let aes: EncoderConfiguration = AesEncoderOptions::new(password.into()).into();
            writer.set_content_methods(vec![aes, EncoderConfiguration::new(EncoderMethod::LZMA2)]);
            writer.set_encrypt_header(self.encrypted_headers);
        }

        if self.solid {
            let (entries, contents): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
            let readers = contents.into_iter().map(SourceReader::new).collect();
            writer.push_archive_entries(entries, readers).expect("a solid block");
        } else {
            for (entry, content) in entries {
                let content = (!content.is_empty()).then_some(content);
                writer.push_archive_entry(entry, content).expect("an entry");
            }
        }

        writer.finish().expect("finish").into_inner()
    }

    /// The archive of `(path, content)` pairs, a path ending in `/` being a folder.
    pub(crate) fn of(self, entries: &[(&str, &[u8])]) -> Vec<u8> {
        self.write(
            entries
                .iter()
                .map(|(name, content)| (sevenz_entry(name, content), *content))
                .collect(),
        )
    }
}

/// Checks that extraction of the first three entries of `index` in `file` ends when the visitor says stop (after the
/// first entry it is given) and ends with the visitor's own error when it fails.
pub(crate) fn assert_stop_and_failure(index: &ArchiveIndex, file: &dyn ReadAt) {
    let cancel = idle();
    let input = Input { file, path: None };
    let mut seen = Vec::new();

    open_source(index, &input, &cancel, None)
        .expect("a source")
        .extract(&[0, 1, 2], &mut |entry, _| {
            seen.push(entry.path.clone());

            Ok(super::Flow::Stop)
        })
        .expect("stopped");
    assert_eq!(seen, [index.entries()[0].path.clone()]);

    let error = open_source(index, &input, &cancel, None)
        .expect("a source")
        .extract(&[1], &mut |_, _| Err(io::Error::other("no space")))
        .expect_err("failed");
    assert_eq!(error.to_string(), "no space");
}

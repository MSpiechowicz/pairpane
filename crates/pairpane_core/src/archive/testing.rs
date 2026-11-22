//! Helpers shared by the tests of the archive code and of the viewer's listing: archives built by hand.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;

/// 2021-03-04 05:06:08 as a zip stores it (MS-DOS date and time).
pub(crate) const DOS_DATE: u16 = 21092;
pub(crate) const DOS_TIME: u16 = 10436;
/// The same moment in milliseconds since the epoch, read as UTC.
pub(crate) const MOMENT: i64 = 1_614_834_368_000;

/// A zip of stored (not compressed) entries, written by hand: `(name, content)`; a name ending in `/` is a folder.
pub(crate) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut directory = Vec::new();

    for (name, content) in entries {
        let offset = bytes.len() as u32;
        let crc = crc32fast::hash(content);
        let size = content.len() as u32;

        bytes.extend_from_slice(b"PK\x03\x04");
        for field in [20u16, 0, 0, DOS_TIME, DOS_DATE] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        for field in [crc, size, size] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(content);

        directory.extend_from_slice(b"PK\x01\x02");
        for field in [20u16, 20, 0, 0, DOS_TIME, DOS_DATE] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        for field in [crc, size, size] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        for field in [0u16, 0, 0, 0] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }

    let start = bytes.len() as u32;
    bytes.extend_from_slice(&directory);
    bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
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

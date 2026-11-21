//! Listing a zip from its central directory, with a reader of our own. A zip library sizes its tables from the entry
//! count it finds in the end records (and falls back to earlier end records when one looks wrong), so a crafted file
//! can make it ask for gigabytes before it reads anything. Here the sizes are checked against the file first, the
//! directory is read record by record through a window of exactly its declared size, and nothing is allocated from a
//! declared count.

use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};

use memchr::memmem;

use crate::model::ArchiveEntry;

use super::archive::{MAX_NAME, clean_name, unreadable};
use super::file::ViewFile;
use super::{MAX_ARCHIVE_ENTRIES, cancelled, refusal};

/// The largest central directory of a zip that is listed.
const MAX_ZIP_DIRECTORY: u64 = 64 * 1024 * 1024;

/// The most entries a zip may declare to be listed at all (the first [`MAX_ARCHIVE_ENTRIES`] are shown).
const MAX_ZIP_ENTRIES: u64 = 200_000;

/// The smallest record of a central directory, in bytes.
const RECORD: usize = 46;

/// The end record, in bytes (without its comment), and how far from the end of a zip it is looked for.
const EOCD: usize = 22;
const EOCD_SEARCH: u64 = 22 + 65_535;

/// The zip64 end record, and the locator that points to it, in bytes.
const ZIP64_EOCD: usize = 56;
const LOCATOR: usize = 20;

/// The bytes of a name that are looked at: the longest name kept, and room for the character it may cut in two.
const NAME_READ: usize = MAX_NAME + 4;

/// The flag of an entry that says its name is UTF-8.
const UTF8_NAME: u16 = 1 << 11;

/// Where the central directory is, and what the end records declare of it.
struct Directory {
    entries: u64,
    size: u64,
    start: u64,
}

/// The entries of the zip `file`, at most [`MAX_ARCHIVE_ENTRIES`], and whether the list is cut.
pub(super) fn list(file: &ViewFile, cancel: &AtomicBool) -> io::Result<(Vec<ArchiveEntry>, bool)> {
    let directory = locate(file)?;

    let mut reader = file.reader();
    reader.seek(SeekFrom::Start(directory.start))?;
    let mut reader = BufReader::new(reader.take(directory.size));

    let mut listed = Vec::new();
    let mut variable = Vec::new();

    for _ in 0..directory.entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        if listed.len() == MAX_ARCHIVE_ENTRIES {
            return Ok((listed, true));
        }

        listed.push(record(&mut reader, &mut variable)?);
    }

    Ok((listed, false))
}

/// Finds the central directory through the last end record in the file, and checks that it fits in the file.
fn locate(file: &ViewFile) -> io::Result<Directory> {
    let size = file.size()?;
    let length = size.min(EOCD_SEARCH);
    let base = size - length;
    let tail = file.read_at(base, usize::try_from(length).map_err(|_| unreadable())?)?;

    // The last signature with a whole record after it.
    let room = tail.len().checked_sub(EOCD - 4).ok_or_else(unreadable)?;
    let at = memmem::rfind(&tail[..room], b"PK\x05\x06").ok_or_else(unreadable)?;
    let end_record = &tail[at..at + EOCD];

    let mut entries = le(end_record, 10, 2);
    let mut bytes = le(end_record, 12, 4);
    let mut offset = le(end_record, 16, 4);
    // Where the directory ends: at the end record, or at the zip64 one before it.
    let mut end = base + at as u64;

    // What does not fit in the end record is in the zip64 record that the locator before it points to. A zip with
    // exactly 65,535 entries has no locator: its numbers are the real ones.
    let locator = at.checked_sub(LOCATOR).and_then(|from| tail.get(from..at));
    if let Some(locator) = locator.filter(|locator| locator.starts_with(b"PK\x06\x07"))
        && (entries == u64::from(u16::MAX) || bytes == u64::from(u32::MAX) || offset == u64::from(u32::MAX))
    {
        let (at, wide) = zip64_record(file, end - LOCATOR as u64, locator)?;

        if entries == u64::from(u16::MAX) {
            entries = le(&wide, 32, 8);
        }
        if bytes == u64::from(u32::MAX) {
            bytes = le(&wide, 40, 8);
        }

        if offset == u64::from(u32::MAX) {
            offset = le(&wide, 48, 8);
        }

        end = at;
    }

    if bytes > MAX_ZIP_DIRECTORY {
        return Err(refusal(
            io::ErrorKind::InvalidData,
            "the archive's directory is too large to list",
        ));
    }

    if entries > MAX_ZIP_ENTRIES || entries.saturating_mul(RECORD as u64) > bytes {
        return Err(refusal(
            io::ErrorKind::InvalidData,
            "the archive declares too many entries to list",
        ));
    }

    // The stored offset counts from the start of the zip, which data may precede (a self-extracting program): the
    // directory is where its size says, ending at the record that follows it, and it must not start before the offset.
    if offset.saturating_add(bytes) > end {
        return Err(unreadable());
    }

    Ok(Directory {
        entries,
        size: bytes,
        start: end - bytes,
    })
}

/// The zip64 end record that the `locator` (which is at `locator_at`) points to, and where it is. The offset in a
/// locator counts from the start of the zip, so when data precedes the zip the record is looked for just before the
/// locator too.
fn zip64_record(file: &ViewFile, locator_at: u64, locator: &[u8]) -> io::Result<(u64, Vec<u8>)> {
    let before = locator_at.saturating_sub(ZIP64_EOCD as u64);

    for at in [le(locator, 8, 8), before] {
        if at.saturating_add(ZIP64_EOCD as u64) > locator_at {
            continue;
        }

        let record = file.read_at(at, ZIP64_EOCD)?;
        if record.len() == ZIP64_EOCD && record.starts_with(b"PK\x06\x06") {
            return Ok((at, record));
        }
    }

    Err(unreadable())
}

/// The next record of the central directory; `variable` is a buffer for its name, extra data and comment.
fn record<R: Read>(reader: &mut R, variable: &mut Vec<u8>) -> io::Result<ArchiveEntry> {
    let mut head = [0; RECORD];
    read_exact(reader, &mut head)?;

    if !head.starts_with(b"PK\x01\x02") {
        return Err(unreadable());
    }

    let flags = le(&head, 8, 2) as u16;
    let name_length = le(&head, 28, 2) as usize;
    let extra_length = le(&head, 30, 2) as usize;
    let comment_length = le(&head, 32, 2) as usize;

    variable.clear();
    variable.resize(name_length + extra_length + comment_length, 0);
    read_exact(reader, variable)?;

    let name = &variable[..name_length];
    let extra = &variable[name_length..name_length + extra_length];

    let mut size = le(&head, 24, 4);
    if size == u64::from(u32::MAX)
        && let Some(wide) = extra_field(extra, 0x0001)
    {
        size = le(wide, 0, 8);
    }

    Ok(ArchiveEntry {
        name: name_text(name, flags),
        size,
        dir: name.ends_with(b"/"),
        modified: unix_time(extra).or_else(|| dos_millis(le(&head, 14, 2) as u16, le(&head, 12, 2) as u16)),
    })
}

/// Reads exactly `buffer`; a directory that ends early is a broken zip.
fn read_exact<R: Read>(reader: &mut R, buffer: &mut [u8]) -> io::Result<()> {
    reader.read_exact(buffer).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => unreadable(),
        _ => error,
    })
}

/// The little-endian number of `width` bytes (at most 8) at `at`; the bytes that are not there count as zero.
fn le(bytes: &[u8], at: usize, width: usize) -> u64 {
    (0..width).rev().fold(0, |number, index| {
        (number << 8) | u64::from(bytes.get(at + index).copied().unwrap_or(0))
    })
}

/// The data of the first field `id` in the extra data of a record.
fn extra_field(extra: &[u8], id: u16) -> Option<&[u8]> {
    let mut rest = extra;

    while rest.len() >= 4 {
        let length = le(rest, 2, 2) as usize;
        let data = rest.get(4..4 + length)?;

        if le(rest, 0, 2) == u64::from(id) {
            return Some(data);
        }

        rest = &rest[4 + length..];
    }

    None
}

/// The modification time in the extended timestamp field (a Unix time, so no time zone to guess), in milliseconds.
fn unix_time(extra: &[u8]) -> Option<i64> {
    let data = extra_field(extra, 0x5455).filter(|data| data.len() >= 5 && data[0] & 1 == 1)?;
    let seconds = i64::from(le(data, 1, 4) as u32 as i32);

    Some(seconds * 1000)
}

/// An entry name as text. A name with the UTF-8 flag is UTF-8; without it, a name that is valid UTF-8 is taken as such
/// (what most tools write) and any other is CP437, as the format says.
fn name_text(raw: &[u8], flags: u16) -> String {
    let raw = &raw[..raw.len().min(NAME_READ)];

    // A name cut in the middle of a character at its end is still UTF-8.
    let utf8 =
        flags & UTF8_NAME != 0 || std::str::from_utf8(raw).map_or_else(|error| error.error_len().is_none(), |_| true);

    if utf8 {
        return clean_name(&String::from_utf8_lossy(raw));
    }

    let text: String = raw
        .iter()
        .map(|byte| match byte.checked_sub(0x80) {
            Some(high) => CP437[usize::from(high)],
            None => char::from(*byte),
        })
        .collect();

    clean_name(&text)
}

/// A zip time (MS-DOS fields, no time zone) as milliseconds since the Unix epoch. The fields are read as UTC: a zip
/// does not say what zone they are in. Fields that are no moment (a zero date, a 31st of April) give `None`.
fn dos_millis(date: u16, time: u16) -> Option<i64> {
    let year = 1980 + i64::from(date >> 9);
    let month = i64::from((date >> 5) & 15);
    let day = i64::from(date & 31);
    let (hour, minute, second) = (
        i64::from(time >> 11),
        i64::from((time >> 5) & 63),
        i64::from(time & 31) * 2,
    );

    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let longest = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };

    if day < 1 || day > longest || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let days = days_from_civil(year, month, day);

    Some((days * 86_400 + hour * 3600 + minute * 60 + second) * 1000)
}

/// The number of days from 1970-01-01 to the given date of the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

/// The characters of the bytes 0x80 to 0xFF of code page 437, the encoding of a zip name without the UTF-8 flag.
const CP437: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û',
    'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡',
    '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─',
    '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█',
    '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±', '≥',
    '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

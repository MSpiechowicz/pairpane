//! The reading of the technical listing of `unrar vt`, as the records of the archive's entries.
//!
//! The listing is the banner, `Archive: <path>`, `Details: RAR 5[, solid][, volume 1]`, and then one block of lines per
//! entry, each `<label right-aligned to 12 characters>: <value>`:
//!
//! ```text
//!         Name: dir/file.txt
//!         Type: File
//!         Size: 8
//!  Packed size: 8
//!        Ratio: 100%
//!        mtime: 2023-11-14 22:13:20,000000000
//!   Attributes: -rw-r--r--
//!        CRC32: 5B6D4005
//!      Host OS: Unix
//!  Compression: RAR 5.0(v50) -m0 -md=128k
//! ```
//!
//! `Type` is `File`, `Directory`, `Unix symbolic link`, `Hard link`, ... (a link also has a `Target:` line); a file
//! with encrypted content has a `Flags: encrypted` line. A name that holds a line break runs over several lines: what
//! stands between the `Name:` line and the `Type:` line is the name, and the line break makes it a name that
//! [`names::normalize`](super::super::names::normalize) refuses. The format cannot tell such a name that imitates a
//! `Type:` line from the real one, so an archive made to do that can only add or lose entries of its own, never change
//! what is extracted: extraction asks `unrar` for names that the index holds and reads only the files it finds for them.
//!
//! The reader never trusts the amount of output: lines are capped, the whole listing is capped, and so is the number of
//! entries; past a cap the listing is [`cut`](Listing::cut).

use std::io::{self, BufRead, BufReader, Read};

use crate::archive::entries::{EntryType, Place, Record};
use crate::archive::error::unreadable;
use crate::archive::{MAX_INDEX_ENTRIES, MAX_NAME_BYTES};

/// The most bytes of `unrar`'s output that are read.
pub const MAX_LIST_BYTES: u64 = 256 * 1024 * 1024;

/// The longest line that is kept; the rest of a longer one is dropped. A name is cut here and so is refused as too long.
const MAX_LINE: usize = MAX_NAME_BYTES + 64;

/// The width the labels are aligned to.
const LABEL_WIDTH: usize = 12;

/// What the listing of an archive says.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub records: Vec<Record>,
    /// The output was longer than the caps allow, or `unrar` said the archive ends early; `records` is what was read.
    pub cut: bool,
    /// The archive is one volume of a set.
    pub volume: bool,
}

/// The fields of the block being read.
#[derive(Default)]
struct Block {
    name: String,
    /// How many lines the name has run over (a name with a line break has more than one).
    lines: u32,
    kind: Option<String>,
    target: Option<String>,
    size: u64,
    modified: Option<i64>,
    attributes: String,
    encrypted: bool,
}

/// The listing in `input`, within the caps. A text that has no `Details: RAR` line (a file `unrar` does not take for an
/// archive prints only a message) is not a listing: `InvalidData`.
pub fn parse(input: impl Read) -> io::Result<Listing> {
    parse_within(input, MAX_LIST_BYTES, MAX_INDEX_ENTRIES + 1)
}

/// [`parse`] reading at most `bytes` and stopping after `entries` entries.
pub fn parse_within(input: impl Read, bytes: u64, entries: usize) -> io::Result<Listing> {
    let mut reader = BufReader::new(input.take(bytes));
    let mut listing = Listing::default();
    let mut block: Option<Block> = None;
    let mut archive = false;
    let mut line = Vec::new();

    loop {
        line.clear();

        if !read_line(&mut reader, &mut line)? {
            break;
        }

        let text = text_of(&line);

        if let Some(details) = text.strip_prefix("Details: ") {
            archive |= details.starts_with("RAR ");
            listing.volume |= details.contains("volume");

            continue;
        }

        match block.as_mut() {
            // Between the name and the type, anything is part of the name.
            Some(open) if open.kind.is_none() => match field(&text) {
                Some(("Type", value)) => open.kind = Some(value.to_owned()),
                _ => open.add_line(&text),
            },
            Some(open) => match field(&text) {
                Some(("Name", value)) => {
                    listing.records.extend(open_block(&mut block));
                    block = Some(Block::named(value));
                }
                Some((label, value)) => open.set(label, value),
                None => {}
            },
            None => {
                if let Some(("Name", value)) = field(&text) {
                    block = Some(Block::named(value));
                }
            }
        }

        if listing.records.len() >= entries {
            listing.cut = true;
            block = None;

            break;
        }
    }

    if reader.get_ref().limit() == 0 {
        listing.cut = true;
    }

    // What was read of a block when a cap came is partial: it is dropped.
    if !listing.cut {
        listing.records.extend(open_block(&mut block));
    }

    if !archive {
        return Err(unreadable());
    }

    Ok(listing)
}

fn open_block(block: &mut Option<Block>) -> Option<Record> {
    block.take().and_then(Block::into_record)
}

/// `(label, value)` of a field line: the label right-aligned to [`LABEL_WIDTH`] characters, then `: `.
fn field(line: &str) -> Option<(&str, &str)> {
    let (head, value) = line.split_once(": ")?;
    let label = head.trim_start_matches(' ');

    let aligned = head.len() == LABEL_WIDTH
        && !label.is_empty()
        && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b' ');

    aligned.then_some((label, value))
}

/// The text of a line: UTF-8 (`unrar` is started in a UTF-8 locale), with what is not that replaced.
fn text_of(line: &[u8]) -> String {
    let text = String::from_utf8_lossy(line);

    // Windows ends its lines with a carriage return; elsewhere one is a part of a name and is kept (and refused).
    if cfg!(windows) {
        text.strip_suffix('\r').unwrap_or(&text).to_owned()
    } else {
        text.into_owned()
    }
}

/// Reads a line without its end, keeping at most [`MAX_LINE`] bytes of it. `false` at the end of the input.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<bool> {
    let mut read = 0;
    let mut ended = false;

    loop {
        let chunk = reader.fill_buf()?;

        if chunk.is_empty() {
            break;
        }

        let (used, found) = match chunk.iter().position(|byte| *byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (chunk.len(), false),
        };
        let room = MAX_LINE.saturating_sub(line.len());
        let end = if found { used - 1 } else { used };

        line.extend_from_slice(&chunk[..end.min(room)]);
        reader.consume(used);
        read += used;

        if found {
            ended = true;

            break;
        }
    }

    Ok(read > 0 || ended)
}

impl Block {
    fn named(value: &str) -> Self {
        let mut block = Self::default();
        block.add_line(value);

        block
    }

    /// Adds a line of the name (the first one, or a continuation after a line break).
    fn add_line(&mut self, text: &str) {
        if self.lines > 0 {
            self.name.push('\n');
        }

        self.lines += 1;

        if self.name.len() <= MAX_NAME_BYTES {
            self.name.push_str(text);
        }
    }

    fn set(&mut self, label: &str, value: &str) {
        match label {
            "Target" => self.target = Some(value.to_owned()),
            "Size" => self.size = value.trim().parse().unwrap_or(0),
            "mtime" => self.modified = time_of(value),
            "Attributes" => value.trim().clone_into(&mut self.attributes),
            "Flags" => self.encrypted |= value.split(", ").any(|flag| flag.trim() == "encrypted"),
            _ => {}
        }
    }

    fn into_record(self) -> Option<Record> {
        let kind = kind_of(self.kind.as_deref()?, &self.attributes);

        // Windows prints the system's separator; an archive made there is read the same on every system.
        let name = if cfg!(windows) {
            self.name.replace('\\', "/")
        } else {
            self.name
        };

        Some(Record {
            name,
            kind,
            size: self.size,
            modified: self.modified,
            mode: mode_of(&self.attributes),
            link: self.target,
            encrypted: self.encrypted,
            place: Place::Walk,
        })
    }
}

/// What the `Type:` and the attributes say an entry is.
fn kind_of(kind: &str, attributes: &str) -> EntryType {
    match kind {
        // Archives of RAR 3 store a link as a file whose attributes say so.
        "File" => match attributes.chars().next() {
            Some('l') => EntryType::Symlink,
            Some('c' | 'b' | 'p' | 's') => EntryType::Special,
            _ => EntryType::File,
        },
        "Directory" => EntryType::Dir,
        other if other.contains("ymbolic link") => EntryType::Symlink,
        other if other.contains("ard link") => EntryType::Hardlink,
        _ => EntryType::Special,
    }
}

/// The permission bits of a Unix attribute string (`-rwsr-xr-x`); `None` for the attributes of other systems
/// (`..A....`).
fn mode_of(attributes: &str) -> Option<u32> {
    let bytes = attributes.as_bytes();

    if bytes.len() != 10 || !b"-dlcbps".contains(&bytes[0]) {
        return None;
    }

    let mut mode = 0;

    for (triple, shift) in [6, 3, 0].into_iter().enumerate() {
        let [read, write, execute] = [bytes[1 + 3 * triple], bytes[2 + 3 * triple], bytes[3 + 3 * triple]];

        match read {
            b'r' => mode |= 0o4 << shift,
            b'-' => {}
            _ => return None,
        }

        match write {
            b'w' => mode |= 0o2 << shift,
            b'-' => {}
            _ => return None,
        }

        // The special bit that shares the execute position: setuid, setgid, sticky.
        let special = 0o4000 >> triple;

        match (execute, triple) {
            (b'x', _) => mode |= 0o1 << shift,
            (b's', 0 | 1) | (b't', 2) => mode |= (0o1 << shift) | special,
            (b'S', 0 | 1) | (b'T', 2) => mode |= special,
            (b'-', _) => {}
            _ => return None,
        }
    }

    Some(mode)
}

/// `2023-11-14 22:13:20,000000000` (read as UTC: `unrar` is started with `TZ=UTC`) in milliseconds since the epoch.
fn time_of(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let separators = [(4, '-'), (7, '-'), (10, ' '), (13, ':'), (16, ':')];

    if separators
        .iter()
        .any(|(at, separator)| text.chars().nth(*at) != Some(*separator))
    {
        return None;
    }

    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let millis = text
        .get(20..23)
        .and_then(|digits| digits.parse::<i64>().ok())
        .unwrap_or(0);
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;

    Some(seconds * 1000 + millis)
}

/// Days from 1970-01-01 to the date (the proleptic Gregorian calendar).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

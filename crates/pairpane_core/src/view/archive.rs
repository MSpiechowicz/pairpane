//! Listing an archive for the viewer: the entries of whatever [`crate::archive`] reads, shown as names, sizes and times.
//! Nothing is extracted and nothing but the names, sizes and times is looked at; an entry's data is skipped, never
//! read (except to decode past it in a compressed tar, up to [`MAX_GZ_SCAN`] bytes).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::archive::names::is_hostile;
use crate::archive::{EntryType, Flow, Input, MAX_NAME_BYTES, Record, records};
use crate::model::{ArchiveEntry, ArchiveFormat, ArchiveListing};

use super::file::ViewFile;
use super::{MAX_ARCHIVE_ENTRIES, MAX_GZ_SCAN, cancelled};

/// The entries of the archive `file` of `format`, at most [`super::MAX_ARCHIVE_ENTRIES`] of them. An archive that cannot be
/// read, or whose central directory is over 64 MiB or declares more than 200,000 entries, is an error; a tar that
/// breaks after some entries, or a compressed tar that is longer than [`MAX_GZ_SCAN`] decoded, lists what was read and
/// is marked truncated. A raised `cancel` stops the listing with an `Interrupted` error.
pub fn archive(file: &ViewFile, format: ArchiveFormat, cancel: &AtomicBool) -> io::Result<ArchiveListing> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    let mut listed = Listed::default();
    let input = Input { file, path: None };
    let cut = records(format, &input, cancel, MAX_GZ_SCAN, None, &mut |record| {
        listed.push(record)
    })?;

    Ok(ArchiveListing {
        format,
        entries: listed.entries,
        truncated: listed.truncated || cut,
    })
}

/// The listing as it is collected from the records of a walk.
#[derive(Default)]
pub(super) struct Listed {
    pub(super) entries: Vec<ArchiveEntry>,
    pub(super) truncated: bool,
}

impl Listed {
    /// Adds the next entry, or stops the walk (and says the list is cut) when [`MAX_ARCHIVE_ENTRIES`] are there.
    pub(super) fn push(&mut self, record: Record) -> Flow {
        if self.entries.len() == MAX_ARCHIVE_ENTRIES {
            self.truncated = true;

            return Flow::Stop;
        }

        self.entries.push(ArchiveEntry {
            name: clean_name(&record.name),
            size: record.size,
            dir: record.kind == EntryType::Dir,
            modified: record.modified,
        });

        Flow::Continue
    }
}

/// `text` as shown for an entry name: a control character (C0, C1) or a bidirectional control that could make the
/// name read as another one becomes `\u{FFFD}`, and a name over [`MAX_NAME_BYTES`] bytes is cut on a character.
pub(super) fn clean_name(text: &str) -> String {
    let mut name = String::with_capacity(text.len().min(MAX_NAME_BYTES));

    for character in text.chars() {
        let shown = if is_hostile(character) { '\u{FFFD}' } else { character };

        if name.len() + shown.len_utf8() > MAX_NAME_BYTES {
            break;
        }

        name.push(shown);
    }

    name
}

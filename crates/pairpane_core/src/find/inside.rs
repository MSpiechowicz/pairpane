//! The names inside archives: a zip, tar (plain or compressed) or 7z file that the walk meets is read for its entries'
//! names, sizes and dates, and the ones that fit come back as results with the archive's path and the entry's.
//!
//! Only the archive's own listing is read; no entry is decompressed or extracted. RAR (it needs a program of the
//! system) and a single gzip file are not looked into. An archive that cannot be read (damaged, or its names are
//! encrypted), one that is cut short or one past a limit counts as one skipped; an entry whose name is not safe (see
//! [`normalize`]) counts as one skipped too.

use std::path::Path;

use crate::archive::{EntryType, Flow, Input, Record, by_extension, normalize, records};
use crate::model::{ArchiveFormat, EntryKind, FindHit};
use crate::ops::open_listed;

use super::filters::Facts;
use super::out::Run;
use super::{ARCHIVE_SCAN, Compiled, MAX_ARCHIVE_RECORDS};

/// The format of the archive file called `name`, when its entries are read here.
pub(crate) fn format(name: &str) -> Option<ArchiveFormat> {
    by_extension(name).filter(|format| !matches!(format, ArchiveFormat::Rar | ArchiveFormat::Gz))
}

/// Reads the archive at `path` (below `root`, opened without following a link) and calls `found` with each entry that
/// fits. Returns how many things were skipped.
pub(crate) fn scan(
    root: &Path,
    path: &str,
    format: ArchiveFormat,
    compiled: &Compiled,
    run: &Run<'_>,
    found: &mut dyn FnMut(FindHit),
) -> u64 {
    let Ok((file, _)) = open_listed(root, path) else {
        return 1;
    };

    let input = Input {
        file: &file,
        path: None,
    };
    let (mut seen, mut skipped) = (0_usize, 0_u64);

    let ended = records(format, &input, run.flag(), ARCHIVE_SCAN, None, &mut |record| {
        seen += 1;
        if run.raised() || seen > MAX_ARCHIVE_RECORDS {
            skipped += u64::from(seen > MAX_ARCHIVE_RECORDS);
            return Flow::Stop;
        }

        match hit(path, record, compiled) {
            Ok(Some(hit)) => found(hit),
            Ok(None) => {}
            Err(()) => skipped += 1,
        }

        Flow::Continue
    });

    match ended {
        Ok(cut) => skipped + u64::from(cut),
        Err(_) if run.raised() => skipped,
        Err(_) => skipped + 1,
    }
}

/// The result `record` makes, when it fits every test. `Err` for a name that is not safe.
fn hit(archive: &str, record: Record, compiled: &Compiled) -> Result<Option<FindHit>, ()> {
    let kind = match record.kind {
        EntryType::File => EntryKind::File,
        EntryType::Dir => EntryKind::Dir,
        _ => return Ok(None),
    };
    let Some(inner) = normalize(&record.name).map_err(drop)? else {
        return Ok(None);
    };

    let hidden = inner.split('/').any(|name| name.starts_with('.'));
    if compiled.skip_hidden && hidden {
        return Ok(None);
    }

    let name = inner.rsplit('/').next().unwrap_or(&inner);
    let facts = Facts {
        file: kind == EntryKind::File,
        size: if kind == EntryKind::File { record.size } else { 0 },
        modified: record.modified,
        hidden,
        readonly: record.mode.is_some_and(|mode| mode & 0o222 == 0),
        executable: kind == EntryKind::File && record.mode.is_some_and(|mode| mode & 0o111 != 0),
    };

    if !compiled.names.fits(name, &format!("{archive}/{inner}")) || !compiled.filters.fits(&facts) {
        return Ok(None);
    }

    Ok(Some(FindHit {
        path: archive.to_owned(),
        inner: Some(inner),
        kind,
        link: false,
        size: facts.size,
        modified: facts.modified,
        matches: None,
        line: None,
        line_number: None,
    }))
}

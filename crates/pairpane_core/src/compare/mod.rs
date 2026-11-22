//! Compare two folders: a walk of both trees side by side that pairs the names and tells, for each path, which side has it
//! and which side is newer. What a synchronization is built from; it changes nothing on disk.
//!
//! # API
//!
//! [`run`] compares two [`Side`]s (a folder on disk, or a folder inside an archive whose index the caller made) on the
//! calling thread and calls `emit` with `rows` batches (every 100 ms or 256 rows), `progress` and, last, exactly one
//! `done`. It returns when the comparison is over; an `Err` (before any event) means a side cannot be compared: not a folder,
//! not readable, the same folder or one inside the other. It stops, within one entry or one chunk of a file read for
//! content, once `cancel` is set. [`status`] is the rule that decides a pair of files.
//!
//! # Rules
//!
//! - Names are paired as the file system compares them (folded to lower case on Windows and macOS). Of two names of one
//!   side that fold together, the first is paired and the other is counted as skipped.
//! - Two files are the same when their sizes are and their times are within [`TOLERANCE_MS`] (some file systems keep times
//!   to two seconds only); otherwise the later one is newer, and equal times with different sizes differ. With `content`,
//!   two files of one size on disk are read and compared by their bytes: equal bytes make them the same, whatever their
//!   times say, and different bytes make the later one newer, or, at equal times, make them differ. Folders are compared
//!   by presence only.
//! - A link is never followed: it is a row of kind `other` with `link` set. A name that is not text, one the system reads
//!   as something else, an entry that cannot be read and a folder nested more than 1024 levels are counted as
//!   skipped.
//! - A folder only one side has is one row with the number of files in it and their bytes (hidden ones too: a copy of the
//!   folder copies them), never one row for each file. A folder both sides have is a row only without `subfolders`.
//! - At most [`MAX_ROWS`] rows are delivered, equal ones included; past that `done` says `truncated`.
//! - Every error text holds no path.

mod content;
mod out;
mod side;
mod walk;

#[cfg(test)]
mod archive_tests;
#[cfg(test)]
mod content_tests;
#[cfg(test)]
mod status_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod walk_tests;

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::listing::platform;
use crate::model::{CompareEvent, CompareFacts, CompareOptions, CompareStatus, EntryKind};
use crate::ops::{is_inside, real};
use crate::view::refusal;

use out::Scan;
pub use side::Side;
use side::{Cursor, refused_below};

/// How far apart the times of two files may be for them to count as the same: some file systems (FAT, some network shares)
/// keep times to two seconds only.
pub const TOLERANCE_MS: u64 = 2_000;

/// The most rows a comparison delivers; the walk stops there and `done` says `truncated`.
pub const MAX_ROWS: u64 = 100_000;

/// What a test sets other than the options: the cap on rows, whether names are paired ignoring case, and how often events
/// go out.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) rows: u64,
    pub(crate) fold: bool,
    pub(crate) tick: Duration,
}

impl Limits {
    pub(crate) const DEFAULT: Self = Self {
        rows: MAX_ROWS,
        fold: platform::CASE_INSENSITIVE,
        tick: out::BATCH_TIME,
    };
}

/// Compares `left` with `right`, sending the events to `emit` on this thread, and returns once `done` was sent. See the
/// module's rules.
///
/// # Errors
///
/// Before any event, when a side cannot be compared: a folder that is not a plain absolute local path, is not a folder or
/// cannot be read, an archive folder that is not in the archive, or two folders that are one or one is inside the other.
/// The text of the error holds no path.
pub fn run(
    left: &Side,
    right: &Side,
    options: CompareOptions,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(CompareEvent),
) -> io::Result<()> {
    run_limited(left, right, options, cancel, Limits::DEFAULT, emit)
}

/// [`run`] within `limits`.
pub(crate) fn run_limited(
    left: &Side,
    right: &Side,
    options: CompareOptions,
    cancel: &AtomicBool,
    limits: Limits,
    emit: &mut dyn FnMut(CompareEvent),
) -> io::Result<()> {
    let (left, right) = (resolved(left)?, resolved(right)?);
    if let (Side::Folder(one), Side::Folder(other)) = (&left, &right) {
        refuse_same(one, other)?;
    }

    let mut scan = Scan::new(cancel, emit, options, limits);
    let cut = skipped_by_archives(&mut scan, [&left, &right]);

    scan.folder(&Cursor::of(&left), &Cursor::of(&right), "", 0);
    scan.finish(cut);

    Ok(())
}

/// The side as it can be read: a folder resolved and checked to be a folder, an archive folder checked to be in the index.
fn resolved(side: &Side) -> io::Result<Side> {
    match side {
        Side::Folder(path) => {
            platform::check_folder(path).map_err(|text| refusal(io::ErrorKind::InvalidInput, text))?;

            let actual = platform::local_path(path)?;
            if !fs::metadata(&actual)?.is_dir() {
                return Err(refusal(io::ErrorKind::NotADirectory, "this is not a folder"));
            }
            fs::read_dir(&actual)?;

            Ok(Side::Folder(actual))
        }
        Side::Archive { index, dir } => {
            if !index.has_dir(dir) {
                return Err(refusal(io::ErrorKind::NotFound, "this folder is not in the archive"));
            }

            Ok(side.clone())
        }
    }
}

/// Refuses two folders that are one, or one inside the other, judged after their links are resolved.
fn refuse_same(one: &Path, other: &Path) -> io::Result<()> {
    let (one, other) = (real(one)?, real(other)?);

    if is_inside(&one, &other) || is_inside(&other, &one) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "The two folders are the same, or one is inside the other.",
        ));
    }

    Ok(())
}

/// Counts what the archive sides leave out (entries with a refused name below the folder), and says whether an index was
/// cut short.
fn skipped_by_archives(scan: &mut Scan<'_>, sides: [&Side; 2]) -> bool {
    let mut cut = false;

    for side in sides {
        if let Side::Archive { index, dir } = side {
            scan.skipped = scan.skipped.saturating_add(refused_below(index, dir));
            cut |= index.truncated();
        }
    }

    cut
}

/// How a pair of entries that both sides have stands. `content_equal` is what reading both files said (`None`: not read).
#[must_use]
pub fn status(left: &CompareFacts, right: &CompareFacts, content_equal: Option<bool>) -> CompareStatus {
    let both_files = left.kind == EntryKind::File && right.kind == EntryKind::File && !left.link && !right.link;

    if both_files {
        return file_status(left, right, content_equal);
    }

    // Folders by presence, links and special files by what they are: a file and a folder, or a link and a file, differ.
    if left.kind == right.kind && left.link == right.link {
        CompareStatus::Equal
    } else {
        CompareStatus::Differs
    }
}

fn file_status(left: &CompareFacts, right: &CompareFacts, content_equal: Option<bool>) -> CompareStatus {
    if content_equal == Some(true) {
        return CompareStatus::Equal;
    }

    if let (Some(one), Some(other)) = (left.modified, right.modified)
        && one.abs_diff(other) > TOLERANCE_MS
    {
        return if one > other {
            CompareStatus::LeftNewer
        } else {
            CompareStatus::RightNewer
        };
    }

    if left.size != right.size || content_equal == Some(false) {
        CompareStatus::Differs
    } else {
        CompareStatus::Equal
    }
}

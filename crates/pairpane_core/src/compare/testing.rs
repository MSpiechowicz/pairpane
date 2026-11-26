//! Helpers shared by the tests of a comparison: trees on disk and a runner that collects the events.

use std::fs::{self, File, FileTimes};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

use tempfile::TempDir;

use crate::archive::{ArchiveIndex, Builder, EntryType, Flow, Place, Record};
use crate::model::{ArchiveFormat, CompareEvent, CompareOptions, CompareRow, CompareStatus};

use super::{Limits, MAX_ROWS, Side, run_limited};

pub(super) fn temp() -> TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

/// Writes `bytes` to `rel` (names joined with `/`) below `root`, making the folders, and sets its time to `millis`.
pub(super) fn put(root: &Path, rel: &str, bytes: &[u8], millis: u64) {
    let path = root.join(rel);

    fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    fs::write(&path, bytes).expect("write");
    set_time(&path, millis);
}

pub(super) fn set_time(path: &Path, millis: u64) {
    let file = File::options().write(true).open(path).expect("open");

    file.set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(millis)))
        .expect("times");
}

/// A link at `at/name` to the folder `real` of `elsewhere`, which holds a file.
#[cfg(unix)]
pub(super) fn link_to_real(elsewhere: &Path, at: &Path, name: &str) {
    put(elsewhere, "real/file.txt", b"1", 10_000);
    std::os::unix::fs::symlink(elsewhere.join("real"), at.join(name)).expect("link");
}

pub(super) fn folder(root: &Path, rel: &str) {
    fs::create_dir_all(root.join(rel)).expect("folder");
}

/// Options that look in the folders both sides have, not at content, not at hidden entries.
pub(super) fn plain() -> CompareOptions {
    CompareOptions {
        subfolders: true,
        content: false,
        hidden: false,
    }
}

/// The default limits, names paired as the system pairs them.
pub(super) fn normal() -> Limits {
    Limits {
        rows: MAX_ROWS,
        ..Limits::DEFAULT
    }
}

/// What a comparison delivered.
pub(super) struct Compared {
    pub(super) events: Vec<CompareEvent>,
}

impl Compared {
    /// The bytes the last `progress` event counted.
    pub(super) fn bytes(&self) -> Option<u64> {
        self.events.iter().rev().find_map(|event| match event {
            CompareEvent::Progress { bytes, .. } => Some(*bytes),
            _ => None,
        })
    }

    /// Every row, ordered by path.
    pub(super) fn rows(&self) -> Vec<CompareRow> {
        let mut rows: Vec<CompareRow> = self
            .events
            .iter()
            .flat_map(|event| match event {
                CompareEvent::Rows { rows } => rows.clone(),
                _ => Vec::new(),
            })
            .collect();
        rows.sort_by(|a, b| a.path.cmp(&b.path));

        rows
    }

    /// `path: status` of every row, ordered by path.
    pub(super) fn statuses(&self) -> Vec<(String, CompareStatus)> {
        self.rows().into_iter().map(|row| (row.path, row.status)).collect()
    }

    /// The `done` event: rows, skipped, truncated, cancelled.
    pub(super) fn done(&self) -> (u64, u64, bool, bool) {
        let ended: Vec<_> = self
            .events
            .iter()
            .filter_map(|event| match event {
                CompareEvent::Done {
                    rows,
                    skipped,
                    truncated,
                    cancelled,
                } => Some((*rows, *skipped, *truncated, *cancelled)),
                _ => None,
            })
            .collect();

        assert_eq!(ended.len(), 1, "exactly one done: {:?}", self.events);
        assert!(
            matches!(self.events.last(), Some(CompareEvent::Done { .. })),
            "done is last"
        );

        ended[0]
    }
}

/// Compares `left` with `right`; `watch` sees every event as it is sent and may raise the flag it is given.
pub(super) fn compare_with(
    (left, right): (Side, Side),
    options: CompareOptions,
    limits: Limits,
    watch: impl Fn(&CompareEvent, &AtomicBool),
) -> io::Result<Compared> {
    let cancel = AtomicBool::new(false);
    let mut events = Vec::new();

    run_limited(&left, &right, options, &cancel, limits, &mut |event| {
        watch(&event, &cancel);
        events.push(event);
    })?;

    Ok(Compared { events })
}

/// The folders `left` and `right` as the two sides of a comparison.
pub(super) fn folders(left: &Path, right: &Path) -> (Side, Side) {
    (Side::Folder(left.to_owned()), Side::Folder(right.to_owned()))
}

/// The folder `dir` of the archive of `index` against the folder `other`.
pub(super) fn archive_and(index: Arc<ArchiveIndex>, dir: &str, other: &Path) -> (Side, Side) {
    (
        Side::Archive {
            index,
            dir: dir.to_owned(),
        },
        Side::Folder(other.to_owned()),
    )
}

/// A comparison that must run, with nobody watching it.
pub(super) fn compared(sides: (Side, Side), options: CompareOptions, limits: Limits) -> Compared {
    compare_with(sides, options, limits, |_, _| {}).expect("a comparison")
}

/// A comparison of the sides that must be refused before any event, with the usual options.
pub(super) fn refused(sides: (Side, Side)) -> io::Error {
    compare_with(sides, plain(), normal(), |_, _| {})
        .err()
        .expect("refused")
}

/// Compares the folders `left` and `right`.
pub(super) fn compare(left: &Path, right: &Path, options: CompareOptions) -> Compared {
    compared(folders(left, right), options, normal())
}

/// The index of a tar-like archive with these entries (path, kind, size, time), for the side of a comparison.
pub(crate) fn index_of(entries: &[(&str, EntryType, u64, i64)]) -> Arc<ArchiveIndex> {
    let mut builder = Builder::new(ArchiveFormat::Tar);

    for (name, kind, size, modified) in entries {
        let record = Record {
            name: (*name).to_owned(),
            kind: *kind,
            size: *size,
            modified: Some(*modified),
            mode: Some(0o644),
            link: None,
            encrypted: false,
            place: Place::Walk,
        };

        assert_eq!(builder.push(record), Flow::Continue);
    }

    Arc::new(builder.finish(false))
}

/// The index of a tar-like archive that holds symbolic links: each is a path and where it leads.
#[cfg(unix)]
pub(super) fn index_of_links(links: &[(&str, &str)]) -> Arc<ArchiveIndex> {
    let mut builder = Builder::new(ArchiveFormat::Tar);

    for (name, target) in links {
        let record = Record {
            name: (*name).to_owned(),
            kind: EntryType::Symlink,
            size: 0,
            modified: Some(10_000),
            mode: Some(0o777),
            link: Some((*target).to_owned()),
            encrypted: false,
            place: Place::Walk,
        };

        assert_eq!(builder.push(record), Flow::Continue);
    }

    Arc::new(builder.finish(false))
}

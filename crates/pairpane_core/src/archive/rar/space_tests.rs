//! The disk `unrar` is given: what is counted before it starts, the reserve kept while it runs, and the version check.

use std::fs;
use std::io;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::run::keep_reserve;
use super::testing::index_of;
use super::{RarSource, Unrar, version};
use crate::archive::entries::{EntrySource, Flow};
use crate::view::is_refusal;

/// Unpacks entry number `wanted` of `entries` with a program that unpacks nothing, in a folder that is also the staging.
fn unpack_of(entries: &[(&str, u64)], wanted: u32) -> (io::Result<()>, Vec<String>) {
    let folder = TempDir::new().expect("folder");
    let archive = folder.path().join("test.rar");
    fs::write(&archive, b"").expect("archive");
    let index = index_of(entries);
    let never = std::sync::atomic::AtomicBool::new(false);
    let mut source = RarSource::new(&index, &archive, Unrar::at(PathBuf::from("/usr/bin/true")), &never)
        .staging_in(folder.path().to_owned());

    let result = source.extract(&[wanted], &mut |_, _| Ok(Flow::Continue));
    let left = fs::read_dir(folder.path())
        .expect("folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    (result, left)
}

#[test]
fn what_unrar_may_write_for_a_name_counts_entries_that_differ_in_case_or_come_twice() {
    // `unrar` matches names without regard to case, so the big one is written when the small one is asked for.
    for (entries, wanted) in [
        ([("Data/Huge.BIN", 1_u64 << 62), ("data/huge.bin", 1)], 1),
        ([("data/huge.bin", 1), ("data/huge.bin", 1 << 62)], 0),
    ] {
        let (result, left) = unpack_of(&entries, wanted);

        assert_eq!(
            result.expect_err("too big").kind(),
            io::ErrorKind::StorageFull,
            "{entries:?}"
        );
        assert_eq!(left, ["test.rar"]);
    }
}

#[test]
fn the_free_space_of_the_staging_volume_is_kept_above_the_reserve() {
    let folder = TempDir::new().expect("folder");

    assert!(keep_reserve(folder.path(), 0).is_ok());

    let error = keep_reserve(folder.path(), u64::MAX).expect_err("no volume has that much");
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(is_refusal(&error));
}

#[test]
fn the_unrar_of_this_system_is_recent_enough_to_be_used() {
    // Skips itself where `unrar` is not installed (or is older than 7, which is the error under test elsewhere).
    let Ok(unrar) = Unrar::locate() else {
        return;
    };

    assert!(version::is_recent(&unrar.program));
}

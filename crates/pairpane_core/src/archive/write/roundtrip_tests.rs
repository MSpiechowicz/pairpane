//! Every format at every level, packed by the writers and read back through the readers of the crate (and `bsdtar`).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::archive::testing::{index_of, noise};
use crate::archive::{MAX_INDEX_ENTRIES, Place};
use crate::model::{ArchiveFormat, PackFormat, PackLevel};

use super::testing::{
    Endless, FORMATS, LEVELS, Raising, assert_bsdtar_reads, bsdtar_has, expected, options, pack, packer, read_back,
    sample, store,
};
use super::{Budget, Entry, Lazy, Packer, deflate_level, lzma_preset, zstd_level};

#[test]
fn every_format_and_level_reads_back_as_it_was_packed() {
    let items = sample();

    for (format, kind) in FORMATS {
        for level in LEVELS {
            let bytes = pack(format, &options(level, None), &items).expect("packed");

            assert_eq!(
                read_back(&bytes, kind, None).expect("read back"),
                expected(format, &items),
                "{format:?} at {level:?}"
            );
        }
    }
}

#[test]
fn the_level_decides_how_small_the_archive_is() {
    let items = sample();
    let size =
        |format: PackFormat, level: PackLevel| pack(format, &options(level, None), &items).expect("packed").len();

    for format in [PackFormat::Zip, PackFormat::SevenZ] {
        // The text of the sample is 88 KB that shrinks to almost nothing; the noise next to it does not.
        for level in [PackLevel::Fast, PackLevel::Normal, PackLevel::Maximum] {
            assert!(
                size(format, level) + 60_000 < size(format, PackLevel::Store),
                "{format:?} at {level:?}"
            );
        }
    }

    // A tar.zst has no stored level: asking for it is the normal one.
    assert_eq!(
        size(PackFormat::TarZst, PackLevel::Store),
        size(PackFormat::TarZst, PackLevel::Normal)
    );
    assert!(
        size(PackFormat::TarZst, PackLevel::Normal) + 60_000
            < items
                .iter()
                .map(|item| item.content.as_ref().map_or(0, Vec::len))
                .sum::<usize>()
    );
}

#[test]
fn a_zip_is_stored_at_the_lowest_level_and_deflated_above_it() {
    let items = sample();

    for level in LEVELS {
        let bytes = pack(PackFormat::Zip, &options(level, None), &items).expect("packed");
        let index = index_of(&bytes, ArchiveFormat::Zip).expect("an index");

        for (entry, item) in index.entries().iter().zip(&items) {
            let Place::Zip(place) = &entry.place else {
                panic!("not a zip place: {:?}", entry.place);
            };
            // Folders and the empty file have no data to deflate.
            let deflated =
                level != PackLevel::Store && item.content.as_ref().is_some_and(|content| !content.is_empty());

            assert_eq!(place.method, if deflated { 8 } else { 0 }, "{} at {level:?}", item.name);
        }
    }
}

#[test]
fn the_level_mappings_are_those_of_the_plan() {
    let all = LEVELS;

    assert_eq!(all.map(deflate_level), [None, Some(1), Some(6), Some(9)]);
    assert_eq!(all.map(zstd_level), [3, 1, 3, 19]);
    assert_eq!(all.map(lzma_preset), [None, Some(1), Some(5), Some(9)]);
}

#[test]
fn bsdtar_reads_the_zip_and_the_tar_zst() {
    let items = sample();
    let folder = tempfile::tempdir().expect("a folder");

    for (name, format, libraries) in [
        ("a.zip", PackFormat::Zip, &["zlib"][..]),
        ("a.tar.zst", PackFormat::TarZst, &["libzstd"][..]),
    ] {
        let bytes = pack(format, &options(PackLevel::Normal, None), &items).expect("packed");

        if bsdtar_has(libraries) {
            assert_bsdtar_reads(&store(folder.path(), name, &bytes), &items, None);
        }
    }
}

#[test]
fn names_that_could_escape_or_are_not_plain_are_refused() {
    for (format, _) in FORMATS {
        for name in ["../x", "a/../b", "/abs", "a/", "a//b", "./a", "", "a\\b", "a\0b"] {
            let cancel = AtomicBool::new(false);
            let mut packer = packer(format, &cancel);
            let error = packer.add(Entry::folder(name, 0o755, None)).expect_err("refused");

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{format:?} {name:?}");
        }
    }
}

#[test]
fn an_archive_takes_no_more_entries_than_the_readers_index() {
    let mut budget = Budget::new();

    for number in 0..MAX_INDEX_ENTRIES {
        budget.admit(&format!("f{number}")).expect("within the limit");
    }

    assert_eq!(
        budget.admit("one-too-many").expect_err("over").kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn a_password_is_refused_for_a_tar_zst_and_when_empty() {
    let cancel = AtomicBool::new(false);

    for (format, password) in [
        (PackFormat::TarZst, "secret"),
        (PackFormat::Zip, ""),
        (PackFormat::SevenZ, ""),
    ] {
        let result = Packer::new(
            format,
            io::Cursor::new(Vec::new()),
            &options(PackLevel::Normal, Some(password)),
            &cancel,
        );

        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(io::ErrorKind::InvalidInput),
            "{format:?}"
        );
    }
}

#[test]
fn a_cancel_raised_in_the_middle_of_a_file_ends_the_pack_as_interrupted() {
    let data = noise(300_000);

    for (format, _) in FORMATS {
        let cancel = AtomicBool::new(false);
        let mut packer = packer(format, &cancel);
        let reader = Raising {
            bytes: &data,
            cancel: &cancel,
        };

        // A 7z reads its files when the block is written, at the end.
        let error = packer
            .add(Entry::file("big.bin", 0o644, None, data.len() as u64, reader))
            .and_then(|()| packer.finish().map(drop))
            .expect_err("cancelled");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{format:?}");
    }
}

#[test]
fn a_cancel_that_is_up_before_an_entry_stops_it() {
    for (format, _) in FORMATS {
        let cancel = AtomicBool::new(true);
        let mut packer = packer(format, &cancel);

        let error = packer.add(Entry::folder("a", 0o755, None)).expect_err("cancelled");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{format:?}");
        cancel.store(false, Ordering::Relaxed);
    }
}

#[test]
fn a_lazy_reader_opens_at_its_first_read_and_lets_go_at_its_end() {
    let opened = std::cell::Cell::new(0);
    let mut lazy = Lazy::new(|| {
        opened.set(opened.get() + 1);

        Ok(&b"data"[..])
    });
    let mut content = Vec::new();

    assert_eq!(opened.get(), 0);
    io::Read::read_to_end(&mut lazy, &mut content).expect("read");
    assert_eq!((content.as_slice(), opened.get()), (&b"data"[..], 1));
    assert_eq!(io::Read::read(&mut lazy, &mut [0; 8]).expect("at the end"), 0);
    assert_eq!(opened.get(), 1);
}

#[test]
fn a_zip_or_7z_file_that_gives_more_than_its_declared_size_fails_after_one_byte_past_it() {
    for format in [PackFormat::Zip, PackFormat::SevenZ] {
        let cancel = AtomicBool::new(false);
        let given = std::cell::Cell::new(0);
        let mut packer = packer(format, &cancel);

        // Stat said 10 bytes; the file never ends.
        let added = packer.add(Entry::file("grew.bin", 0o644, None, 10, Endless { given: &given }));
        let error = added.and_then(|()| packer.finish().map(|_| ())).expect_err("fails");

        assert_eq!(error.to_string(), "a file grew while it was being packed", "{format:?}");
        assert_eq!(given.get(), 11, "{format:?}: it is read up to one byte past the size");
    }
}

//! Rewriting a zip: the entries that stay are the same bytes, the offsets in the records follow them (zip64 where they no
//! longer fit), and the new entries of an encrypted zip are encrypted.

use std::io::Cursor;

use pretty_assertions::assert_eq;

use crate::archive::testing::idle;
use crate::archive::write::testing::{Item, assert_bsdtar_reads, bsdtar_decrypts, sample, store};
use crate::archive::write::{Limits, Options, ZipWriter};
use crate::archive::{EntryType, PasswordFault, Place, password_fault};
use crate::model::{ArchiveFormat, PackLevel};

use super::testing::{SECRET, archive, changed, index, ordinal, rewrite_by, rewrite_with, secret, seen, wanted};
use super::zip;

const ZIP: ArchiveFormat = ArchiveFormat::Zip;

fn added() -> Vec<Item> {
    vec![
        Item::folder("fresh", 0o755),
        Item::file("fresh/new.txt", 0o644, b"brand new"),
        Item::file("src/extra.rs", 0o600, b"pub fn extra() {}\n"),
    ]
}

/// The local header and the data of every entry of the zip `bytes`, by path, as they are in the file.
fn local_parts(bytes: &Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let index = index(bytes, ZIP, Some(&secret())).expect("an index");
    let mut parts = Vec::new();

    for entry in index.entries() {
        let Place::Zip(place) = &entry.place else {
            panic!("a zip entry");
        };
        let at = place.offset as usize;
        let name = usize::from(u16::from_le_bytes([bytes[at + 26], bytes[at + 27]]));
        let extra = usize::from(u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]));
        let length = 30 + name + extra + place.compressed as usize;

        parts.push((entry.path.clone(), bytes[at..at + length].to_vec()));
    }

    parts.sort();

    parts
}

/// How many records of the central directory of the zip `bytes` say their offset is in the zip64 field.
fn wide_offsets(bytes: &Vec<u8>) -> usize {
    let index = index(bytes, ZIP, None).expect("an index");

    index
        .entries()
        .iter()
        .filter(|entry| {
            let Place::Zip(place) = &entry.place else {
                return false;
            };
            let at = place.record_at as usize;

            bytes[at + 42..at + 46] == [0xff; 4]
        })
        .count()
}

#[test]
fn the_entries_that_stay_are_the_same_bytes_and_those_left_out_and_added_read_back() {
    let items = sample();
    let old = archive(ZIP, &items, None, false);
    let gone = ordinal(&old, ZIP, "text.txt");

    let new = rewrite_with(&old, ZIP, (&[gone], &added()), None).expect("rewritten");

    assert_eq!(
        seen(&new, ZIP, None),
        wanted(ZIP, &changed(&items, &["text.txt"], &added()))
    );

    let (before, after) = (local_parts(&old), local_parts(&new));
    for (path, bytes) in before.iter().filter(|(path, _)| path != "text.txt") {
        assert_eq!(
            after.iter().find(|(name, _)| name == path).map(|(_, b)| b),
            Some(bytes),
            "{path}"
        );
    }
}

#[test]
fn nothing_left_out_and_nothing_added_is_an_archive_with_the_same_entries() {
    let items = sample();
    let old = archive(ZIP, &items, None, false);

    let new = rewrite_with(&old, ZIP, (&[], &[]), None).expect("rewritten");

    assert_eq!(seen(&new, ZIP, None), wanted(ZIP, &items));
    assert_eq!(local_parts(&new), local_parts(&old));
}

#[test]
fn the_new_entries_of_an_encrypted_zip_are_encrypted_with_its_password() {
    let items = sample();
    let old = archive(ZIP, &items, Some(SECRET), false);
    let password = secret();

    let new = rewrite_with(&old, ZIP, (&[], &added()), Some(&password)).expect("rewritten");

    assert_eq!(
        seen(&new, ZIP, Some(&password)),
        wanted(ZIP, &changed(&items, &[], &added()))
    );
    let index = index(&new, ZIP, Some(&password)).expect("an index");
    assert!(
        index
            .entries()
            .iter()
            .filter(|entry| entry.kind == EntryType::File)
            .all(|entry| entry.encrypted)
    );
    assert_eq!(local_parts(&old).len() + added().len(), local_parts(&new).len());
}

#[test]
fn an_encrypted_zip_cannot_be_rewritten_without_its_password() {
    let old = archive(ZIP, &sample(), Some(SECRET), false);

    let error = rewrite_with(&old, ZIP, (&[], &added()), None).expect_err("refused");

    assert_eq!(password_fault(&error), Some(PasswordFault::Needed));
}

#[test]
fn offsets_that_no_longer_fit_are_zip64_and_those_that_were_keep_their_field() {
    let items = sample();
    let wide = Limits {
        offset: 100,
        ..Limits::default()
    };

    // A zip whose offsets are 32 bits, rewritten where they no longer fit.
    let old = archive(ZIP, &items, None, false);
    let gone = ordinal(&old, ZIP, "src");
    let new = rewrite_by(&old, ZIP, (&[gone], &added()), None, |out, old, added| {
        zip::rewrite(out, old, added, wide)
    })
    .expect("rewritten");
    assert_eq!(seen(&new, ZIP, None), wanted(ZIP, &changed(&items, &["src"], &added())));
    assert_eq!(wide_offsets(&old), 0);
    assert!(
        wide_offsets(&new) >= items.len() - 2,
        "the offsets from 100 on are zip64"
    );

    // A zip that has the zip64 field in every record, rewritten where they would fit (the first entry is left out, so every
    // offset changes).
    let cancel = idle();
    let options = Options {
        level: PackLevel::Normal,
        password: None,
        encrypt_names: false,
    };
    let mut writer = ZipWriter::with_limits(Cursor::new(Vec::new()), &options, &cancel, wide);
    for item in &items {
        writer.add(item.entry()).expect("added");
    }
    let old = writer.finish().expect("a zip").into_inner();
    let gone = ordinal(&old, ZIP, "src");
    assert!(wide_offsets(&old) > 0);

    let new = rewrite_with(&old, ZIP, (&[gone], &[]), None).expect("rewritten");
    assert_eq!(
        wide_offsets(&new),
        wide_offsets(&old),
        "a record keeps the field it has"
    );

    assert_eq!(seen(&new, ZIP, None), wanted(ZIP, &changed(&items, &["src"], &[])));
}

#[test]
fn bsdtar_lists_and_extracts_the_rewritten_zips_plain_and_encrypted() {
    let folder = tempfile::TempDir::new().expect("temp dir");
    let items = sample();

    let plain = archive(ZIP, &items, None, false);
    let new = rewrite_with(&plain, ZIP, (&[ordinal(&plain, ZIP, "text.txt")], &added()), None).expect("rewritten");
    let path = store(folder.path(), "plain.zip", &new);
    let all = changed(&items, &["text.txt"], &added());
    if crate::archive::write::testing::bsdtar(&["--version"]).is_some() {
        assert_bsdtar_reads(&path, &all, None);
    }

    if bsdtar_decrypts() {
        let locked = archive(ZIP, &items, Some(SECRET), false);
        let new = rewrite_with(&locked, ZIP, (&[], &added()), Some(&secret())).expect("rewritten");
        let path = store(folder.path(), "locked.zip", &new);

        assert_bsdtar_reads(&path, &changed(&items, &[], &added()), Some(SECRET));
    }
}

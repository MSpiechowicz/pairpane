//! The zip writer: the headers it writes, AES, and the zip64 paths (reached with low limits).

use std::io::{self, Cursor};
use std::sync::atomic::AtomicBool;

use crate::archive::testing::{DOS_TIME, MOMENT, idle, index_of, noise};
use crate::archive::zip::ZipPlace;
use crate::archive::{EntryType, Password, PasswordFault, Place, password_fault};
use crate::model::{ArchiveFormat, PackFormat, PackLevel};

use super::testing::{
    Item, assert_bsdtar_reads, bsdtar_decrypts, bsdtar_has, expected, options, pack, read_back, sample, store,
};
use super::{Limits, Options, ZipWriter};

fn places(bytes: &Vec<u8>) -> Vec<(String, ZipPlace)> {
    index_of(bytes, ArchiveFormat::Zip)
        .expect("an index")
        .entries()
        .iter()
        .map(|entry| match &entry.place {
            Place::Zip(place) => (entry.path.clone(), (**place).clone()),
            other => panic!("not a zip place: {other:?}"),
        })
        .collect()
}

/// `items` in a zip with the zip64 `limits`.
fn pack_limited(items: &[Item], options: &Options, limits: Limits) -> io::Result<Vec<u8>> {
    let cancel = idle();
    let mut writer = ZipWriter::with_limits(Cursor::new(Vec::new()), options, &cancel, limits);

    for item in items {
        writer.add(item.entry())?;
    }

    Ok(writer.finish()?.into_inner())
}

fn contains(bytes: &[u8], pattern: &[u8]) -> bool {
    bytes.windows(pattern.len()).any(|window| window == pattern)
}

/// Where the data of the entry starts: past its local header, name and extra field.
fn data_start(bytes: &[u8], place: &ZipPlace) -> usize {
    let at = place.offset as usize;

    at + 30
        + usize::from(u16::from_le_bytes([bytes[at + 26], bytes[at + 27]]))
        + usize::from(u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]))
}

#[test]
fn an_entry_has_a_utf8_name_no_descriptor_and_its_time_and_mode_in_the_headers() {
    let bytes = pack(PackFormat::Zip, &options(PackLevel::Normal, None), &sample()).expect("packed");

    for (path, place) in places(&bytes) {
        assert_eq!(place.flags & 0x800, 0x800, "{path}");
        assert_eq!(place.flags & 8, 0, "{path}");
        assert_eq!(place.dos_time, DOS_TIME, "{path}");
        assert_eq!(place.made_by >> 8, if cfg!(windows) { 0 } else { 3 }, "{path}");
    }

    let index = index_of(&bytes, ArchiveFormat::Zip).expect("an index");
    let run = index
        .entries()
        .iter()
        .find(|entry| entry.path == "run.bin")
        .expect("run.bin");

    assert_eq!((run.kind, run.modified), (EntryType::File, Some(MOMENT)));
    assert_eq!(run.mode, cfg!(unix).then_some(0o755));
}

#[test]
fn a_zip_with_a_password_holds_aes_entries_each_with_a_salt_of_its_own() {
    let items = [
        Item::file("a.txt", 0o644, b"the same content"),
        Item::file("b.txt", 0o644, b"the same content"),
        Item::file("empty.txt", 0o644, b""),
        Item::folder("folder", 0o755),
    ];
    let bytes = pack(PackFormat::Zip, &options(PackLevel::Normal, Some("secret")), &items).expect("packed");
    let found = places(&bytes);

    for (path, place) in &found[..3] {
        let aes = place.aes.expect("an AES field");

        assert_eq!((place.method, place.crc, place.flags & 1), (99, 0, 1), "{path}");
        assert_eq!((aes.version, aes.strength), (2, 3), "{path}");
        assert_eq!(aes.method, if path == "empty.txt" { 0 } else { 8 }, "{path}");
    }
    assert_eq!((found[3].1.method, found[3].1.aes), (0, None));

    // The salt is the first 16 bytes of the data.
    let salt = |place: &ZipPlace| bytes[data_start(&bytes, place)..][..16].to_vec();
    assert_ne!(salt(&found[0].1), salt(&found[1].1));

    let password = Password::new("secret".to_owned());
    assert_eq!(
        read_back(&bytes, ArchiveFormat::Zip, Some(&password)).expect("opened"),
        expected(PackFormat::Zip, &items)
    );

    let needed = read_back(&bytes, ArchiveFormat::Zip, None).expect_err("needs the password");
    assert_eq!(password_fault(&needed), Some(PasswordFault::Needed));

    let wrong = Password::new("not it".to_owned());
    let refused = read_back(&bytes, ArchiveFormat::Zip, Some(&wrong)).expect_err("wrong password");
    assert_eq!(password_fault(&refused), Some(PasswordFault::Wrong));
}

#[test]
fn bsdtar_opens_a_zip_with_a_password() {
    if !bsdtar_decrypts() {
        return;
    }

    let items = sample();
    let folder = tempfile::tempdir().expect("a folder");
    let bytes = pack(PackFormat::Zip, &options(PackLevel::Normal, Some("secret")), &items).expect("packed");

    assert_bsdtar_reads(&store(folder.path(), "locked.zip", &bytes), &items, Some("secret"));
}

/// Entries of noise, so deflate keeps their size: some small, three that reach the reserve and the size limit.
fn crowd() -> Vec<Item> {
    let mut items = vec![Item::folder("d", 0o755)];

    for number in 0..9 {
        let size = if number % 3 == 2 { 1500 } else { 100 };
        items.push(Item::file(&format!("d/f{number}.bin"), 0o644, &noise(size + number)));
    }

    items
}

const LOW: Limits = Limits {
    size: 1000,
    offset: 4000,
    entries: 5,
    reserve: 800,
};

#[test]
fn numbers_past_the_limits_make_a_zip64_that_reads_back() {
    let items = crowd();

    for level in [PackLevel::Store, PackLevel::Fast] {
        let bytes = pack_limited(&items, &options(level, None), LOW).expect("packed");

        // The end records are zip64 and so are the entries past 4000 bytes.
        assert!(
            contains(&bytes, b"PK\x06\x06") && contains(&bytes, b"PK\x06\x07"),
            "{level:?}"
        );
        assert!(
            places(&bytes).iter().any(|(_, place)| place.offset >= LOW.offset),
            "{level:?}"
        );
        assert_eq!(
            read_back(&bytes, ArchiveFormat::Zip, None).expect("read back"),
            expected(PackFormat::Zip, &items),
            "{level:?}"
        );
    }
}

#[test]
fn bsdtar_reads_a_zip64() {
    if !bsdtar_has(&["zlib"]) {
        return;
    }

    let items = crowd();
    let folder = tempfile::tempdir().expect("a folder");
    let bytes = pack_limited(&items, &options(PackLevel::Store, None), LOW).expect("packed");

    assert_bsdtar_reads(&store(folder.path(), "wide.zip", &bytes), &items, None);
}

#[test]
fn zip64_and_aes_go_together() {
    let items = crowd();
    let bytes = pack_limited(&items, &options(PackLevel::Normal, Some("secret")), LOW).expect("packed");
    let password = Password::new("secret".to_owned());

    assert_eq!(
        read_back(&bytes, ArchiveFormat::Zip, Some(&password)).expect("opened"),
        expected(PackFormat::Zip, &items)
    );
}

#[test]
fn a_file_that_outgrows_the_limit_it_was_not_given_room_for_fails_the_pack() {
    let content = noise(2000);
    let cancel = AtomicBool::new(false);
    let mut writer = ZipWriter::with_limits(Cursor::new(Vec::new()), &options(PackLevel::Store, None), &cancel, LOW);

    // Stat said 10 bytes, so no zip64 slot was made; the file then gave 2000.
    let error = writer
        .add(super::Entry::file("grew.bin", 0o644, None, 10, content.as_slice()))
        .expect_err("fails");

    assert_eq!(error.kind(), io::ErrorKind::Other);
}

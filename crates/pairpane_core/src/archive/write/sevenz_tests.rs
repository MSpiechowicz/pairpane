//! The 7z writer: blocks, encryption (content and headers), the dictionary cap and what it needs of the reader's files.

use std::io::Cursor;

use sevenz_rust2::{Archive, Block};

use crate::archive::testing::{idle, noise};
use crate::archive::{Password, PasswordFault, password_fault};
use crate::model::{ArchiveFormat, PackFormat, PackLevel};

use super::testing::{Item, expected, options, pack, read_back, sample};
use super::{Options, SevenZWriter};

const AES: &[u8] = &[0x06, 0xF1, 0x07, 0x01];
const LZMA2: &[u8] = &[0x21];

fn secret() -> Password {
    Password::new("secret ✓".to_owned())
}

/// The blocks of the archive in `bytes`.
fn blocks(bytes: &[u8], password: &str) -> Vec<Block> {
    Archive::read(&mut Cursor::new(bytes), &sevenz_rust2::Password::from(password))
        .expect("the headers")
        .blocks
}

/// The properties of the first coder of `block` with the method `id`.
fn coder<'a>(block: &'a Block, id: &[u8]) -> &'a [u8] {
    block
        .coders
        .iter()
        .find(|coder| coder.encoder_method_id() == id)
        .expect("a coder of the method")
        .properties()
}

/// `items` in a 7z that closes a block at 1 KB of input.
fn pack_in_small_blocks(items: &[Item], options: &Options) -> Vec<u8> {
    let cancel = idle();
    let mut writer = SevenZWriter::new(Cursor::new(Vec::new()), options, &cancel)
        .expect("a writer")
        .with_block(1000);

    for item in items {
        writer.add(item.entry()).expect("added");
    }

    writer.finish().expect("finished").into_inner()
}

#[test]
fn a_7z_with_a_password_and_encrypted_names_needs_it_even_to_list() {
    let items = sample();
    let options = options(PackLevel::Normal, Some("secret ✓"));
    let bytes = pack(PackFormat::SevenZ, &options, &items).expect("packed");

    assert_eq!(
        read_back(&bytes, ArchiveFormat::SevenZ, Some(&secret())).expect("opened"),
        expected(PackFormat::SevenZ, &items)
    );

    let needed = read_back(&bytes, ArchiveFormat::SevenZ, None).expect_err("needs the password");
    assert_eq!(password_fault(&needed), Some(PasswordFault::Needed));

    let wrong = Password::new("not it".to_owned());
    let refused = read_back(&bytes, ArchiveFormat::SevenZ, Some(&wrong)).expect_err("wrong password");
    assert_eq!(password_fault(&refused), Some(PasswordFault::Wrong));
}

#[test]
fn a_7z_with_a_password_and_open_names_lists_without_it_but_not_extracts() {
    let items = sample();
    let options = Options {
        encrypt_names: false,
        ..options(PackLevel::Normal, Some("secret ✓"))
    };
    let bytes = pack(PackFormat::SevenZ, &options, &items).expect("packed");
    let index = crate::archive::testing::index_of(&bytes, ArchiveFormat::SevenZ).expect("the names are open");

    assert_eq!(index.entries().len(), items.len());
    assert!(index.entries().iter().any(|entry| entry.encrypted));

    let needed = read_back(&bytes, ArchiveFormat::SevenZ, None).expect_err("needs the password");
    assert_eq!(password_fault(&needed), Some(PasswordFault::Needed));
    assert_eq!(
        read_back(&bytes, ArchiveFormat::SevenZ, Some(&secret())).expect("opened"),
        expected(PackFormat::SevenZ, &items)
    );
}

#[test]
fn every_block_has_an_iv_of_its_own_and_the_archive_one_salt_and_a_strong_key() {
    let items: Vec<Item> = (0..4)
        .map(|number| Item::file(&format!("f{number}.bin"), 0o644, &noise(1500 + number)))
        .collect();
    let bytes = pack_in_small_blocks(&items, &options(PackLevel::Fast, Some("secret ✓")));

    assert_eq!(
        read_back(&bytes, ArchiveFormat::SevenZ, Some(&secret())).expect("opened"),
        expected(PackFormat::SevenZ, &items)
    );

    let blocks = blocks(&bytes, "secret ✓");
    assert_eq!(blocks.len(), 4);

    let properties: Vec<&[u8]> = blocks.iter().map(|block| coder(block, AES)).collect();

    for props in &properties {
        // 2^19 rounds, a 16-byte salt and a 16-byte IV (the two sizes are in the next byte).
        assert_eq!((props.len(), props[0] & 0x3F), (34, 19));
    }

    assert!(
        properties.iter().all(|props| props[2..18] == properties[0][2..18]),
        "one salt"
    );

    let mut ivs: Vec<&[u8]> = properties.iter().map(|props| &props[18..34]).collect();
    ivs.sort_unstable();
    ivs.dedup();
    assert_eq!(ivs.len(), 4, "an IV for each block");
}

#[test]
fn the_dictionary_is_capped_at_32_mib_and_store_is_copy() {
    let items = [Item::file(
        "a.txt",
        0o644,
        b"some text to compress, some text to compress",
    )];

    // The LZMA2 property byte: the dictionary is (2 | b & 1) << (b / 2 + 11).
    let property = |level| {
        let bytes = pack(PackFormat::SevenZ, &options(level, None), &items).expect("packed");

        coder(&blocks(&bytes, "")[0], LZMA2)[0]
    };
    assert_eq!(property(PackLevel::Maximum), 26, "level 9 asks for 64 MiB and gets 32");
    assert_eq!(property(PackLevel::Normal), 22, "8 MiB");

    let bytes = pack(PackFormat::SevenZ, &options(PackLevel::Store, None), &items).expect("packed");
    assert_eq!(blocks(&bytes, "")[0].coders[0].encoder_method_id(), &[0x00][..]);
}

#[test]
fn folders_and_empty_files_alone_make_an_archive_with_no_block() {
    let items = [Item::folder("a", 0o755), Item::file("a/empty", 0o644, b"")];
    let bytes = pack(PackFormat::SevenZ, &options(PackLevel::Normal, None), &items).expect("packed");

    assert!(blocks(&bytes, "").is_empty());
    assert_eq!(
        read_back(&bytes, ArchiveFormat::SevenZ, None).expect("read back"),
        expected(PackFormat::SevenZ, &items)
    );
}

#[test]
fn an_archive_of_blocks_and_empty_entries_keeps_their_order() {
    let items = vec![
        Item::folder("d", 0o755),
        Item::file("d/one.bin", 0o644, &noise(1200)),
        Item::file("d/empty", 0o644, b""),
        Item::file("d/two.bin", 0o644, &noise(1300)),
        Item::folder("e", 0o755),
    ];
    let bytes = pack_in_small_blocks(&items, &options(PackLevel::Normal, None));

    assert_eq!(
        read_back(&bytes, ArchiveFormat::SevenZ, None).expect("read back"),
        expected(PackFormat::SevenZ, &items)
    );
}

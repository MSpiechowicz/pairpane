//! Rewriting a 7z: the blocks that no left-out entry is in are copied as they are (their packed bytes are in the new
//! archive unchanged), the others are written again, and the encryption of the old archive (content and headers) carries
//! over to what is written.

use std::io::{self, Cursor};
use std::sync::atomic::{AtomicUsize, Ordering};

use pretty_assertions::assert_eq;
use sevenz_rust2::Archive;

use crate::archive::sevenz::open;
use crate::archive::testing::idle;
use crate::archive::write::testing::{Item, sample};
use crate::archive::{PasswordFault, ReadAt, password_fault};
use crate::model::ArchiveFormat;

use super::Old;
use super::testing::{SECRET, archive, changed, index, ordinal, rewrite_with, secret, seen, sevenz_in_blocks, wanted};

const SEVENZ: ArchiveFormat = ArchiveFormat::SevenZ;

fn added() -> Vec<Item> {
    vec![
        Item::folder("fresh", 0o755),
        Item::file("fresh/new.txt", 0o644, b"brand new"),
        Item::file("src/extra.rs", 0o600, b"pub fn extra() {}\n"),
    ]
}

/// The packed bytes of each block of the 7z `bytes` (opened with `password`), in block order.
fn packed(bytes: &[u8], password: &str) -> Vec<Vec<u8>> {
    let archive = Archive::read(&mut Cursor::new(bytes), &sevenz_rust2::Password::from(password)).expect("the headers");
    let map = &archive.stream_map;
    let mut blocks = Vec::new();

    let firsts = map.block_first_pack_stream_index();

    for (block, first) in firsts.iter().enumerate() {
        let mut stream = Vec::new();
        let end = firsts.get(block + 1).copied().unwrap_or(archive.pack_sizes().len());

        for at in *first..end {
            let start = (32 + archive.pack_pos() + map.pack_stream_offsets()[at]) as usize;
            stream.extend_from_slice(&bytes[start..start + archive.pack_sizes()[at] as usize]);
        }
        blocks.push(stream);
    }

    blocks
}

/// Whether `inside` has `part` somewhere in it.
fn has(inside: &[u8], part: &[u8]) -> bool {
    inside.windows(part.len()).any(|window| window == part)
}

#[test]
fn blocks_without_a_left_out_entry_are_copied_and_the_others_written_again() {
    let items = sample();
    // Three blocks: the first four files, the noise, and the last two.
    let old = sevenz_in_blocks(&items, None, false);
    let before = packed(&old, "");
    assert_eq!(before.len(), 3);

    // One entry of the first block, and the whole of the second.
    let gone = [ordinal(&old, SEVENZ, "text.txt"), ordinal(&old, SEVENZ, "noise.bin")];
    let new = rewrite_with(&old, SEVENZ, (&gone, &added()), None).expect("rewritten");

    assert_eq!(
        seen(&new, SEVENZ, None),
        wanted(SEVENZ, &changed(&items, &["text.txt", "noise.bin"], &added()))
    );
    assert!(has(&new, &before[2]), "the last block is the same bytes");
    assert!(!has(&new, &before[0]), "the first block was written again");
}

#[test]
fn nothing_left_out_keeps_every_block_as_it_was() {
    let items = sample();
    let old = sevenz_in_blocks(&items, None, false);

    let new = rewrite_with(&old, SEVENZ, (&[], &added()), None).expect("rewritten");

    for block in packed(&old, "") {
        assert!(has(&new, &block));
    }
    assert_eq!(
        seen(&new, SEVENZ, None),
        wanted(SEVENZ, &changed(&items, &[], &added()))
    );
}

#[test]
fn an_encrypted_7z_keeps_its_blocks_and_encrypts_what_it_writes() {
    let items = sample();
    let password = secret();
    let old = sevenz_in_blocks(&items, Some(SECRET), false);
    let before = packed(&old, SECRET);
    let gone = [ordinal(&old, SEVENZ, "text.txt")];

    let new = rewrite_with(&old, SEVENZ, (&gone, &added()), Some(&password)).expect("rewritten");

    assert_eq!(
        seen(&new, SEVENZ, Some(&password)),
        wanted(SEVENZ, &changed(&items, &["text.txt"], &added()))
    );
    assert!(has(&new, &before[1]) && has(&new, &before[2]));
    // Every file's content is encrypted, the new ones and the ones written again too.
    let index = index(&new, SEVENZ, Some(&password)).expect("an index");
    assert!(
        index
            .entries()
            .iter()
            .filter(|entry| entry.size > 0 && entry.kind == crate::archive::EntryType::File)
            .all(|entry| entry.encrypted)
    );
}

#[test]
fn encrypted_headers_stay_encrypted() {
    let items = sample();
    let password = secret();
    let old = archive(SEVENZ, &items, Some(SECRET), true);

    let new = rewrite_with(&old, SEVENZ, (&[], &added()), Some(&password)).expect("rewritten");

    let error = index(&new, SEVENZ, None).expect_err("the names need the password");
    assert_eq!(password_fault(&error), Some(PasswordFault::Needed));
    assert_eq!(
        seen(&new, SEVENZ, Some(&password)),
        wanted(SEVENZ, &changed(&items, &[], &added()))
    );
}

#[test]
fn an_encrypted_7z_cannot_be_rewritten_without_its_password() {
    let old = archive(SEVENZ, &sample(), Some(SECRET), false);

    let error = rewrite_with(&old, SEVENZ, (&[], &added()), None).expect_err("refused");

    assert_eq!(password_fault(&error), Some(PasswordFault::Needed));
}

#[test]
fn folders_and_empty_files_stay() {
    let items = sample();
    let old = archive(SEVENZ, &items, None, false);

    let new = rewrite_with(&old, SEVENZ, (&[ordinal(&old, SEVENZ, "src")], &[]), None).expect("rewritten");

    assert_eq!(
        seen(&new, SEVENZ, None),
        wanted(SEVENZ, &changed(&items, &["src"], &[]))
    );
}

/// A file in memory of which one read, the `bad`th (from 0), fails with a disk error.
struct Flaky<'a> {
    bytes: &'a Vec<u8>,
    reads: AtomicUsize,
    bad: usize,
}

impl ReadAt for Flaky<'_> {
    fn size(&self) -> io::Result<u64> {
        self.bytes.size()
    }

    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        if self.reads.fetch_add(1, Ordering::Relaxed) == self.bad {
            return Err(io::Error::from_raw_os_error(5));
        }

        self.bytes.read_at(buffer, offset)
    }
}

#[test]
fn a_read_that_fails_while_the_headers_are_checked_for_encryption_fails_the_rewrite() {
    let bytes = archive(SEVENZ, &sample(), Some(SECRET), true);
    let password = secret();
    let cancel = idle();

    // The reads that opening the archive takes: the check of the headers does the same ones after them.
    let counting = Flaky {
        bytes: &bytes,
        reads: AtomicUsize::new(0),
        bad: usize::MAX,
    };
    open(&counting, &cancel, Some(&password)).expect("opened");
    let flaky = Flaky {
        bytes: &bytes,
        reads: AtomicUsize::new(0),
        bad: counting.reads.load(Ordering::Relaxed),
    };

    let index = index(&bytes, SEVENZ, Some(&password)).expect("an index");
    let mut progress = |_: u64| Ok(());
    let old = Old {
        index: &index,
        file: &flaky,
        cancel: &cancel,
        password: Some(&password),
        left_out: &[],
        progress: &mut progress,
    };

    let error = super::sevenz::rewrite(Cursor::new(Vec::new()), old, Vec::new()).expect_err("a disk error");

    assert_eq!(error.raw_os_error(), Some(5));
}

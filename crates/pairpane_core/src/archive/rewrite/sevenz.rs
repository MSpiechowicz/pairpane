//! Rewriting a 7z: the library's `push_raw_block` copies a block (its packed bytes, coders and checksums) as it is, so a
//! block that no left-out entry is in is carried over without being decoded. A block that has one (or that the library
//! refuses to copy, as it lacks the checksums it would have to make up) is decoded, and each of its other entries is written
//! again, in a block of its own, from the stream the block decoder gives. Entries with no stream (folders, empty files) are
//! written as they are. The new entries are written last, in a new block.
//!
//! Encryption follows the old archive: when any block is encrypted (or the headers are) the password is needed, and every
//! block that is written (re-encoded or new) is AES-256 with a fresh IV, as the writer does for a pack; the headers are
//! encrypted again when they were. The blocks that are copied keep the IV and the salt they had.

use std::io::{self, Read, Seek, Write};
use std::sync::atomic::AtomicBool;

use sevenz_rust2::{ArchiveEntry, BlockDecoder};

use crate::archive::error::{PasswordFault, check, password_error, unreadable};
use crate::archive::sevenz::{Entry as Decoded, MEMORY_KB, Source, failure, headers_are_encrypted, listed, open};
use crate::archive::write::{Entry, Options, SevenZWriter};
use crate::model::PackLevel;

use super::{Metered, Old};

/// Writes the 7z of `old` without its left-out entries, then `added`, into `out`.
pub(super) fn rewrite<'a, W: Write + Seek>(out: W, old: Old<'a>, added: Vec<Entry<'a>>) -> io::Result<W> {
    let opened = open(old.file, old.cancel, old.password)?;
    let archive = &opened.archive;

    // The ordinals of the index are the positions among the listed entries: the archive must still hold what it did.
    let numbered = listed(archive);
    if numbered.len() != old.index.entries().len() {
        return Err(unreadable());
    }

    let mut left = vec![false; archive.files.len()];
    for ordinal in old.left_out {
        left[*numbered.get(*ordinal as usize).ok_or_else(unreadable)?] = true;
    }

    // The members of each block: the files that have a stream in it.
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); archive.blocks.len()];
    for (file, entry) in archive.files.iter().enumerate() {
        if let (true, Some(block)) = (entry.has_stream, archive.stream_map.file_block_index[file]) {
            members.get_mut(block).ok_or_else(unreadable)?.push(file);
        }
    }

    let headers = headers_are_encrypted(old.file, old.cancel);
    let secured = headers || (0..archive.files.len()).any(|file| archive.is_file_encrypted(file));
    if secured && old.password.is_none() {
        return Err(password_error(PasswordFault::Needed));
    }

    let options = Options {
        level: PackLevel::Normal,
        password: old.password.filter(|_| secured).cloned(),
        encrypt_names: headers,
    };
    let mut writer = SevenZWriter::new(out, &options, old.cancel)?;
    let mut source = Metered {
        inner: Source::new(old.file, old.cancel),
        progress: &mut *old.progress,
    };
    let mut carried = vec![false; archive.blocks.len()];

    for (file, entry) in archive.files.iter().enumerate() {
        check(old.cancel)?;

        match archive.stream_map.file_block_index[file] {
            _ if entry.is_anti_item() => {}
            Some(block) if entry.has_stream => {
                if std::mem::replace(&mut carried[block], true) {
                    continue;
                }

                let whole = members[block].iter().all(|member| !left[*member]);

                // A block that stays whole is copied; the library refusing it leaves it to be written again.
                if !(whole && writer.copy_block(archive, block, &mut source)?) {
                    let first = archive.stream_map.block_first_file_index[block];
                    let encrypted = archive.is_file_encrypted(file);
                    let decoder =
                        BlockDecoder::new_with_memory_limit(1, MEMORY_KB, block, archive, &opened.secret, &mut source);

                    write_again(&mut writer, decoder, &left, first, (old.cancel, encrypted))?;
                }
            }
            _ if !left[file] => writer.add_decoded(entry.clone(), None)?,
            _ => {}
        }
    }

    for entry in added {
        writer.add(entry)?;
    }

    writer.finish()
}

/// Decodes the block of `decoder` (whose first file is `first`) and writes each of its entries that has data and stays, in
/// a block of its own. `settings` say whether the block is encrypted and, with the cancel flag, how a failure is told.
fn write_again<W: Write + Seek, R: Read + Seek>(
    writer: &mut SevenZWriter<'_, W>,
    decoder: BlockDecoder<'_, R>,
    left: &[bool],
    first: usize,
    (cancel, encrypted): (&AtomicBool, bool),
) -> io::Result<()> {
    let mut here = first;
    let mut failed = None;

    let decoded = decoder.for_each_entries(&mut |entry: &ArchiveEntry, data: &mut dyn Read| {
        let current = here;
        here += 1;

        // An entry that is left out is read past by the decoder; one with no stream is written by the caller.
        if !entry.has_stream || left[current] {
            return Ok(true);
        }

        let mut data = Decoded {
            inner: data,
            cancel,
            encrypted,
        };

        Ok(match writer.add_decoded(entry.clone(), Some(&mut data)) {
            Ok(()) => true,
            Err(error) => {
                failed = Some(error);

                false
            }
        })
    });

    if let Some(error) = failed {
        return Err(error);
    }

    decoded.map(|_| ()).map_err(|error| failure(error, cancel, encrypted))
}

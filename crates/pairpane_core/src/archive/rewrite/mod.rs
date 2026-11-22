//! Rewriting an archive with entries added and some left out: the old archive is read, what stays is carried over as it is
//! where the format allows, and the new entries are written after it. The result is a whole new archive in `out`; the old
//! file is never written (the caller puts the new one in its place when it is complete).
//!
//! - zip ([`zip`]): the local header, the data and the central record of every entry that stays are copied byte for byte,
//!   with the offset in the record patched (and the zip64 field made when the offset no longer fits 32 bits). Nothing is
//!   decoded or encrypted again.
//! - tar, and a tar in a compression ([`tar`]): the old stream is decoded, the header blocks of every entry that stays
//!   (the headers that describe it, a GNU long name or PAX records, included) and its data are copied as they are, the
//!   zero blocks that end the tar are left out, the new entries follow, and the whole is encoded again by the same codec at
//!   its default level.
//! - 7z ([`sevenz`]): a block of packed streams that no left-out entry is in is copied as it is, without being decoded; a
//!   block with such an entry (or one the library cannot copy) is decoded and its other entries are written again, each in
//!   a block of its own.
//!
//! Every read of the old archive reports the bytes it moved to the caller's `progress` function, which can end the rewrite
//! with an error (a cancel). A raised `cancel` flag ends it too, with `Interrupted`.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;

use crate::archive::error::unsupported;
use crate::archive::{ArchiveIndex, EntryType, Password, ReadAt};
use crate::archive::{Place, write::Entry};
use crate::model::ArchiveFormat;

pub mod sevenz;
pub mod tar;
pub mod zip;

#[cfg(test)]
mod sevenz_tests;
#[cfg(test)]
mod tar_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod zip_tests;

/// The function a rewrite calls with the bytes it takes from the old archive (and between its chunks, so a job can stop it).
pub type Progress<'a> = &'a mut dyn FnMut(u64) -> io::Result<()>;

/// The old archive of a rewrite, and what is to be left out of it.
pub struct Old<'a> {
    pub index: &'a ArchiveIndex,
    pub file: &'a dyn ReadAt,
    pub cancel: &'a AtomicBool,
    /// The password of the archive: what opens the entries of a 7z, and what the new entries of a zip with encrypted
    /// entries are encrypted with.
    pub password: Option<&'a Password>,
    /// The ordinals (ascending) of the entries of the index that are left out.
    pub left_out: &'a [u32],
    pub progress: Progress<'a>,
}

impl Old<'_> {
    /// Whether the entry `ordinal` is left out.
    fn leaves_out(&self, ordinal: u32) -> bool {
        self.left_out.binary_search(&ordinal).is_ok()
    }
}

/// Writes the archive of `old` without its left-out entries, then `added`, into `out`, in the format of the old one, and
/// gives `out` back. An archive that has encrypted entries needs its password in `old`.
///
/// # Errors
///
/// Any failure reading the old archive or writing `out` (the old one is read as the readers of this crate read it: one
/// that they refuse is refused here), and a format that is not written (a RAR, a single gzip file).
pub fn rewrite<'a, W: Write + Seek>(out: W, old: Old<'a>, added: Vec<Entry<'a>>) -> io::Result<W> {
    match old.index.format() {
        ArchiveFormat::Zip => zip::rewrite(out, old, added, crate::archive::write::Limits::default()),
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst
        | ArchiveFormat::TarBz2 => tar::rewrite(out, old, added),
        ArchiveFormat::SevenZ => sevenz::rewrite(out, old, added),
        ArchiveFormat::Gz | ArchiveFormat::Rar => Err(unsupported("this archive cannot be changed")),
    }
}

/// How many bytes a rewrite of the archive of `index` without the entries `left_out` is about to move, for a progress bar
/// (the readers' own counts differ by what is compressed, so the bar is ended by the caller when the copy is done).
#[must_use]
pub fn bytes_to_copy(index: &ArchiveIndex, left_out: &[u32]) -> u64 {
    index
        .entries()
        .iter()
        .filter(|entry| left_out.binary_search(&entry.ordinal).is_err())
        .map(|entry| match &entry.place {
            Place::Zip(place) => place.compressed,
            _ if entry.kind == EntryType::File => entry.size,
            _ => 0,
        })
        .fold(0, u64::saturating_add)
}

/// Whether any file of the archive has encrypted content.
#[must_use]
pub fn is_encrypted(index: &ArchiveIndex) -> bool {
    index
        .entries()
        .iter()
        .any(|entry| entry.encrypted && entry.kind == EntryType::File)
}

/// A reader that tells `progress` what it read (after it read it), and fails when `progress` does.
pub(super) struct Metered<'p, R> {
    pub(super) inner: R,
    pub(super) progress: Progress<'p>,
}

impl<R: Read> Read for Metered<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        (self.progress)(count as u64)?;

        Ok(count)
    }
}

impl<R: Seek> Seek for Metered<'_, R> {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.inner.seek(target)
    }
}

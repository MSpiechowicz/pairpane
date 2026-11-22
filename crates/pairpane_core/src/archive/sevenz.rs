//! 7z archives. The listing reads the headers and nothing else: the header of a 7z names every entry with its size,
//! times and attributes, so no data is decoded (an archive whose headers are encrypted needs the password even to be
//! listed). The data of chosen entries is decoded one compression block at a time: a block no wanted entry lies in is
//! never touched, and in a solid block the entries before the last wanted one are decoded and thrown away, as the format
//! allows nothing else.
//!
//! A wrong password cannot always be told from a damaged archive: encrypted headers are told (the archive is read
//! again without a password, and if that says the headers need one, the password was wrong), and so is the content of
//! an encrypted block that fails to decode or to match its checksum. A stored (not compressed) encrypted block with a wrong
//! password only fails at its checksum, at the end of the entry.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};

use sevenz_rust2::{Archive, ArchiveEntry, BlockDecoder, Error};

use super::entries::{Declared, EntrySource, EntryType, Flow, Place, Record, Visit, Walked};
use super::error::{PasswordFault, check, password_error, password_fault, unreadable, unsupported};
use super::file::{FileReader, ReadAt};
use super::index::ArchiveIndex;
use super::password::Password;
use crate::view::refusal;

/// The most memory, in KiB, that the headers of one archive and the dictionary of one coder may take: 512 MiB, far past
/// the 64 MiB of `7z -mx9`; an archive that asks for more is refused.
pub(super) const MEMORY_KB: usize = 512 * 1024;

/// How much of the entry [`check_password`] probes it decodes to see whether the password opens it.
const PROBE: u64 = 64 * 1024;

/// Windows attribute bits of a 7z entry.
pub(super) const ATTRIBUTE_DIRECTORY: u32 = 0x10;
const ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Set when the high 16 bits of the attributes are a Unix `st_mode` (what p7zip and 7-Zip on Unix write).
pub(super) const ATTRIBUTE_UNIX_EXTENSION: u32 = 0x8000;

/// The `st_mode` bits: the file type and the permission bits.
const MODE_TYPE: u32 = 0o170_000;
pub(super) const MODE_DIRECTORY: u32 = 0o040_000;
pub(super) const MODE_REGULAR: u32 = 0o100_000;
const MODE_SYMLINK: u32 = 0o120_000;
const MODE_PERMISSIONS: u32 = 0o7777;

/// 100 ns ticks per millisecond, and the milliseconds from 1601 (the 7z epoch) to 1970.
pub(super) const TICKS_PER_MILLISECOND: u64 = 10_000;
pub(super) const EPOCH_GAP_MS: i64 = 11_644_473_600_000;

/// The archive file, read and sought at the pace of the decoder. Once `cancel` is raised a read fails, with an error of
/// kind `Other` and not `Interrupted`: the decoders drain what they skip with `io::copy`, which tries again at every
/// `Interrupted` and so would never stop. [`failure`] turns it into the `Interrupted` error of [`cancelled`].
pub(super) struct Source<'a> {
    reader: FileReader<'a, dyn ReadAt + 'a>,
    cancel: &'a AtomicBool,
}

impl<'a> Source<'a> {
    pub(super) fn new(file: &'a dyn ReadAt, cancel: &'a AtomicBool) -> Self {
        Self {
            reader: FileReader::new(file),
            cancel,
        }
    }
}

impl Read for Source<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(refusal(io::ErrorKind::Other, "the read was cancelled"));
        }

        self.reader.read(buffer)
    }
}

impl Seek for Source<'_> {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.reader.seek(target)
    }
}

/// An archive whose headers are read, and the password it was read with.
pub(super) struct Opened {
    pub(super) archive: Archive,
    pub(super) secret: sevenz_rust2::Password,
}

/// The password the way the format takes it (UTF-16); a missing one is the empty password, which means none. Called once
/// per [`open`] (the result is kept in [`Opened`] for the decoding and dropped with it), so there is one copy of the
/// password in the library's type at a time. That type is the library's own and cannot be overwritten when it is dropped
/// (it holds a plain byte vector and implements no zeroizing), unlike this crate's own [`Password`].
fn secret(password: Option<&Password>) -> sevenz_rust2::Password {
    password
        .filter(|password| !password.as_str().is_empty())
        .map_or_else(sevenz_rust2::Password::empty, |password| {
            sevenz_rust2::Password::new(password.as_str())
        })
}

/// Reads the headers of the archive in `file`.
pub(super) fn open(file: &dyn ReadAt, cancel: &AtomicBool, password: Option<&Password>) -> io::Result<Opened> {
    let secret = secret(password);
    let mut source = Source::new(file, cancel);

    match Archive::read_with_memory_limit(&mut source, &secret, MEMORY_KB) {
        Ok(archive) => Ok(Opened { archive, secret }),
        Err(error) => {
            let failed = failure(error, cancel, false);

            // A header that cannot be read with a password may need it and not be damaged: it does when it asks for one
            // of an empty password.
            if secret.is_empty() || password_fault(&failed).is_some() || failed.kind() != io::ErrorKind::InvalidData {
                return Err(failed);
            }

            // Not needed any more: the second read is without one.
            drop(secret);

            // A read that fails here says why: it is not taken for headers that need no password.
            Err(if headers_are_encrypted(file, cancel)? {
                password_error(PasswordFault::Wrong)
            } else {
                failed
            })
        }
    }
}

/// Whether the headers of the 7z archive in `file` are encrypted: they ask for a password when they are read without one.
///
/// # Errors
///
/// Any other failure of that read (a disk error, a raised `cancel`, headers that cannot be read at all): the answer is not
/// known then, and a caller must not take it for "no".
pub(super) fn headers_are_encrypted(file: &dyn ReadAt, cancel: &AtomicBool) -> io::Result<bool> {
    let mut source = Source::new(file, cancel);
    let none = sevenz_rust2::Password::empty();

    match Archive::read_with_memory_limit(&mut source, &none, MEMORY_KB) {
        Ok(_) => Ok(false),
        Err(Error::PasswordRequired) => Ok(true),
        Err(error) => Err(failure(error, cancel, false)),
    }
}

/// `error` of the 7z code as an error of ours: a raised `cancel` first, then what it says about the password and the
/// method. `encrypted` is whether the data it failed on is encrypted with a password that was given: it failing then is
/// taken for the password being wrong.
pub(super) fn failure(error: Error, cancel: &AtomicBool, encrypted: bool) -> io::Error {
    if let Err(stopped) = check(cancel) {
        return stopped;
    }

    match error {
        Error::PasswordRequired => password_error(PasswordFault::Needed),
        Error::MaybeBadPassword(_) if encrypted => password_error(PasswordFault::Wrong),
        Error::MaxMemLimited { .. } | Error::UnsupportedCompressionMethod(_) | Error::Unsupported(_) => {
            unsupported("this 7z archive uses a method that is not supported")
        }
        Error::Io(error, _) | Error::MaybeBadPassword(error) if error.raw_os_error().is_some() => error,
        _ => unreadable(),
    }
}

/// The entries that make the index, in order: all but the anti-items (marks that an update removes a file, not entries).
pub(super) fn listed(archive: &Archive) -> Vec<usize> {
    (0..archive.files.len())
        .filter(|file| !archive.files[*file].is_anti_item())
        .collect()
}

/// What the entry `file` is by its attributes: the Unix mode when the archive stores one, else the Windows bits.
fn kind_of(entry: &ArchiveEntry) -> EntryType {
    if entry.is_directory() {
        return EntryType::Dir;
    }

    let attributes = entry.windows_attributes();

    if !entry.has_windows_attributes {
        return EntryType::File;
    }

    if attributes & ATTRIBUTE_UNIX_EXTENSION != 0 {
        return match (attributes >> 16) & MODE_TYPE {
            0 | MODE_REGULAR => EntryType::File,
            MODE_DIRECTORY => EntryType::Dir,
            MODE_SYMLINK => EntryType::Symlink,
            _ => EntryType::Special,
        };
    }

    if attributes & ATTRIBUTE_DIRECTORY != 0 {
        EntryType::Dir
    } else if attributes & ATTRIBUTE_REPARSE_POINT != 0 {
        EntryType::Symlink
    } else {
        EntryType::File
    }
}

/// The last modification of `entry` in milliseconds since the Unix epoch.
fn modified_of(entry: &ArchiveEntry) -> Option<i64> {
    if !entry.has_last_modified_date {
        return None;
    }

    let ticks = u64::from(entry.last_modified_date());

    if ticks == 0 {
        return None;
    }

    i64::try_from(ticks / TICKS_PER_MILLISECOND)
        .ok()
        .and_then(|milliseconds| milliseconds.checked_sub(EPOCH_GAP_MS))
}

/// The record of the entry `file` of `archive`.
fn record_of(archive: &Archive, file: usize) -> Record {
    let entry = &archive.files[file];
    let kind = kind_of(entry);
    let attributes = entry.windows_attributes();
    let unix = entry.has_windows_attributes && attributes & ATTRIBUTE_UNIX_EXTENSION != 0;

    Record {
        name: entry.name().to_owned(),
        kind,
        size: if kind == EntryType::Dir { 0 } else { entry.size() },
        modified: modified_of(entry),
        mode: unix.then_some((attributes >> 16) & MODE_PERMISSIONS),
        link: None,
        encrypted: entry.has_stream() && archive.is_file_encrypted(file),
        place: Place::Walk,
    }
}

/// Calls `visit` with the record of every entry of the 7z archive in `file`, from its headers alone, until the end or
/// until it says [`Flow::Stop`]. An archive with encrypted headers is an error that [`password_fault`] tells ([`PasswordFault::Needed`]
/// without `password`, [`PasswordFault::Wrong`] when it does not open them). Anti-items are left out.
pub fn walk(
    file: &dyn ReadAt,
    cancel: &AtomicBool,
    password: Option<&Password>,
    mut visit: impl FnMut(Record) -> Flow,
) -> io::Result<Walked> {
    let opened = open(file, cancel, password)?;

    for entry in listed(&opened.archive) {
        check(cancel)?;

        if visit(record_of(&opened.archive, entry)) == Flow::Stop {
            return Ok(Walked {
                stopped: true,
                cut: false,
            });
        }
    }

    Ok(Walked::default())
}

/// The encrypted entry that tells most about a password for the least work: of the entries that come first in their block
/// (so decoding stops at them), the smallest, which is read to its end when it is within [`PROBE`] and so ends in what the
/// decoder checks there. `None` when no entry is encrypted.
fn probe_entry(archive: &Archive) -> Option<usize> {
    let map = &archive.stream_map;

    (0..archive.files.len())
        .filter(|file| archive.files[*file].has_stream() && archive.is_file_encrypted(*file))
        .filter(|file| map.file_block_index[*file].is_some_and(|block| map.block_first_file_index[block] == *file))
        .min_by_key(|file| archive.files[*file].size())
}

/// Whether `password` opens the archive in `file`: its headers, and when its content is encrypted an entry of it (see
/// [`probe_entry`]: at most 64 KiB of it are decoded). `Ok(false)` is a password that does not open it, or a missing one
/// that is needed; an archive that cannot be read at all is an error. A stored (not compressed) encrypted entry longer than
/// the probe cannot be told wrong before its end, so it counts as opened.
pub fn check_password(file: &dyn ReadAt, password: &Password) -> io::Result<bool> {
    // Nothing here is long enough to need stopping: the headers and at most 64 KiB of one entry.
    let cancel = &AtomicBool::new(false);
    let opened = match open(file, cancel, Some(password)) {
        Ok(opened) => opened,
        Err(error) if password_fault(&error).is_some() => return Ok(false),
        Err(error) => return Err(error),
    };

    let Some(first) = probe_entry(&opened.archive) else {
        return Ok(true);
    };

    let mut source = Source::new(file, cancel);
    let probed = decode(&opened, &mut source, cancel, &[first], &mut |_, data| {
        // One byte more than the probe, so an entry that is as long as it is read to its end.
        io::copy(&mut data.take(PROBE + 1), &mut io::sink())?;

        Ok(Flow::Continue)
    });

    match probed {
        Ok(()) => Ok(true),
        Err(error) if password_fault(&error) == Some(PasswordFault::Wrong) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Reads an entry's decoded data, saying a failure the way [`EntrySource`] promises: a raised cancel flag is
/// `Interrupted`, a failure of encrypted data a wrong password, any other damage `InvalidData`.
pub(super) struct Entry<'r, 'c> {
    pub(super) inner: &'r mut dyn Read,
    pub(super) cancel: &'c AtomicBool,
    pub(super) encrypted: bool,
}

impl Read for Entry<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer).map_err(|error| {
            if let Err(stopped) = check(self.cancel) {
                stopped
            } else if self.encrypted {
                password_error(PasswordFault::Wrong)
            } else if error.raw_os_error().is_some() {
                error
            } else {
                unreadable()
            }
        })
    }
}

/// Calls `visit` with the data of the entries `files` (indices into the archive's files, ascending) in that order; the
/// first argument is the position in `files`. Only the blocks that hold a wanted entry are decoded, and each only up to
/// its last wanted entry.
fn decode(
    opened: &Opened,
    source: &mut Source<'_>,
    cancel: &AtomicBool,
    files: &[usize],
    visit: &mut dyn FnMut(usize, &mut dyn Read) -> io::Result<Flow>,
) -> io::Result<()> {
    let archive = &opened.archive;
    let mut next = 0;
    let mut stopped = false;

    while next < files.len() && !stopped {
        check(cancel)?;

        let file = files[next];
        let encrypted = archive.is_file_encrypted(file);

        let Some(block) = archive.stream_map.file_block_index[file] else {
            // No data of its own: a folder or an empty file.
            let mut nothing: &[u8] = &[];
            let mut data = Declared::new(
                Entry {
                    inner: &mut nothing,
                    cancel,
                    encrypted: false,
                },
                0,
            );
            stopped = visit(next, &mut data)? == Flow::Stop;
            next += 1;

            continue;
        };

        if encrypted && opened.secret.is_empty() {
            return Err(password_error(PasswordFault::Needed));
        }

        let before = next;
        let first = archive.stream_map.block_first_file_index[block];
        let decoder = BlockDecoder::new_with_memory_limit(1, MEMORY_KB, block, archive, &opened.secret, source);
        let end = first + decoder.entry_count();
        let mut here = first;
        let mut failed = None;

        let decoded = decoder.for_each_entries(&mut |entry: &ArchiveEntry, data: &mut dyn Read| {
            let current = here;
            here += 1;

            if files.get(next) != Some(&current) {
                // Not wanted: the decoder reads past it and drops the bytes.
                return Ok(true);
            }

            let mut data = Declared::new(
                Entry {
                    inner: data,
                    cancel,
                    encrypted,
                },
                entry.size(),
            );

            match visit(next, &mut data) {
                Ok(Flow::Continue) => {}
                Ok(Flow::Stop) => stopped = true,
                Err(error) => {
                    failed = Some(error);
                    stopped = true;
                }
            }

            next += 1;

            // Past the last wanted entry of the block the rest of it is left undecoded.
            Ok(!stopped && files.get(next).is_some_and(|following| *following < end))
        });

        if let Some(error) = failed {
            return Err(error);
        }

        if let Err(error) = decoded {
            return Err(failure(error, cancel, encrypted));
        }

        // A block that did not hold the entry the headers say it holds: the archive is damaged.
        if next == before {
            return Err(unreadable());
        }
    }

    Ok(())
}

/// Extracts from a 7z archive: a block at a time (see the module's docs).
pub struct SevenZSource<'a> {
    index: &'a ArchiveIndex,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    password: Option<&'a Password>,
}

impl<'a> SevenZSource<'a> {
    pub fn new(
        index: &'a ArchiveIndex,
        file: &'a dyn ReadAt,
        cancel: &'a AtomicBool,
        password: Option<&'a Password>,
    ) -> Self {
        Self {
            index,
            file,
            cancel,
            password,
        }
    }
}

impl EntrySource for SevenZSource<'_> {
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()> {
        if wanted.is_empty() {
            return Ok(());
        }

        let opened = open(self.file, self.cancel, self.password)?;
        let numbered = listed(&opened.archive);
        let entries = self.index.entries();

        // The ordinals are those of the index: the archive must still list the same entries.
        let files = wanted
            .iter()
            .map(|ordinal| {
                let entry = entries.get(*ordinal as usize)?;
                let file = *numbered.get(*ordinal as usize)?;

                (entry.kind == EntryType::Dir || opened.archive.files[file].size() == entry.size).then_some(file)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(unreadable)?;

        let mut source = Source::new(self.file, self.cancel);

        decode(&opened, &mut source, self.cancel, &files, &mut |position, data| {
            visit(&entries[wanted[position] as usize], data)
        })
    }
}

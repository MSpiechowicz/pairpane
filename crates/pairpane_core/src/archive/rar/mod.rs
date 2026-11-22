//! RAR archives, read through the `unrar` program of the system. This code lists and unpacks and never writes an
//! archive; it has no decoder of its own (RAR's compression is not free to implement), and a system without `unrar` opens
//! no RAR archive.
//!
//! # API
//!
//! - [`records`]: the entries of an archive, from `unrar vt` ([`parse`]); [`open_source`]: an [`EntrySource`] that unpacks
//!   chosen entries ([`RarSource`], see its module for the staging folder and the disk it uses).
//! - [`find`]: where `unrar` is: a few fixed absolute folders, then the absolute entries of `PATH`. [`missing`] is the
//!   error when it is nowhere, [`is_missing`] tells it from every other: `NotFound` with the fixed text [`MISSING`],
//!   which a page shows as it is.
//! - [`protected`]: the error of a password-protected archive (`Unsupported`, [`PASSWORD_TEXT`]). Passwords are not
//!   supported: an archive whose headers are encrypted is refused when it is opened ([`headers_encrypted`] looks at its
//!   first bytes; `unrar`'s own refusal is the second line of defence), and an entry whose content is encrypted is listed
//!   as encrypted and refused when it is unpacked.
//! - [`run::Unrar`]: the program and the two commands; see its module for exactly what is run and with which switches.
//!
//! # Limits
//!
//! A volume of a set is refused (`Unsupported`). Entries other than files and folders (symbolic links, hard links, file
//! references) are listed and never written: `unrar` is told to skip links, and only regular files whose names the index
//! holds are read back. `unrar` runs with the rights of the app (no sandbox yet); it is killed on cancel or deadline.
//! `unrar` matches names without regard to case, so two entries whose names differ only in case may be unpacked over each
//! other on a volume that does not tell them apart. On Windows `unrar` prints local times, which are read as UTC.

pub mod find;
mod parse;
pub mod run;
pub mod source;
pub mod stage;

#[cfg(test)]
mod find_tests;
#[cfg(test)]
mod parse_tests;
#[cfg(test)]
mod run_tests;
#[cfg(test)]
mod testing;

use std::io;
use std::sync::atomic::AtomicBool;

use crate::view::{is_refusal, refusal};

use super::Input;
use super::entries::{EntrySource, Flow, Record};
use super::error::unsupported;
use super::file::ReadAt;
use super::index::ArchiveIndex;

pub use find::find;
pub use parse::{Listing, parse, parse_within};
pub use run::Unrar;
pub use source::RarSource;

/// What a page is told when `unrar` is not installed.
pub const MISSING: &str = "Install unrar to open RAR archives";

/// What a page is told for a password-protected archive.
pub const PASSWORD_TEXT: &str = "password-protected RAR archives are not supported";

/// The error of a system without `unrar`: `NotFound`, text [`MISSING`].
pub fn missing() -> io::Error {
    refusal(io::ErrorKind::NotFound, MISSING)
}

/// Whether `error` is [`missing`].
pub fn is_missing(error: &io::Error) -> bool {
    is_refusal(error) && error.kind() == io::ErrorKind::NotFound && error.to_string() == MISSING
}

/// The error of a password-protected archive or entry: `Unsupported`, text [`PASSWORD_TEXT`].
pub fn protected() -> io::Error {
    unsupported(PASSWORD_TEXT)
}

/// What the archive needs a file of: the viewer has none, and `unrar` reads from a path.
const NEEDS_FILE: &str = "a RAR archive can only be read from a file on disk";

/// Walks the entries of the RAR archive `input` (see [`Unrar::list`]), calling `visit` with each until the end or until it
/// says [`Flow::Stop`]. Returns whether the archive ended early. The path of `input` must be an absolute path.
pub fn records(input: &Input<'_>, cancel: &AtomicBool, visit: &mut dyn FnMut(Record) -> Flow) -> io::Result<bool> {
    records_with(&Unrar::locate()?, input, cancel, visit)
}

/// [`records`] with the `unrar` given.
pub fn records_with(
    unrar: &Unrar,
    input: &Input<'_>,
    cancel: &AtomicBool,
    visit: &mut dyn FnMut(Record) -> Flow,
) -> io::Result<bool> {
    let path = input.path.ok_or_else(|| unsupported(NEEDS_FILE))?;

    if headers_encrypted(input.file)? {
        return Err(protected());
    }

    let listing = unrar.list(path, cancel)?;

    if listing.volume {
        return Err(unsupported("multi-volume RAR archives are not supported"));
    }

    for record in listing.records {
        if visit(record) == Flow::Stop {
            break;
        }
    }

    Ok(listing.cut)
}

/// A source of the data of the entries of `index`, unpacked from the archive of `input` by the `unrar` of the system.
/// Fails with [`missing`] when there is none, so a caller learns it before it writes anything.
pub fn open_source<'a>(
    index: &'a ArchiveIndex,
    input: &Input<'a>,
    cancel: &'a AtomicBool,
) -> io::Result<Box<dyn EntrySource + 'a>> {
    let path = input.path.ok_or_else(|| unsupported(NEEDS_FILE))?;

    Ok(Box::new(RarSource::new(index, path, Unrar::locate()?, cancel)))
}

/// The signature of a RAR 5 file, and of a RAR 4 one.
const SIGNATURE_5: &[u8] = b"Rar!\x1A\x07\x01\x00";
const SIGNATURE_4: &[u8] = b"Rar!\x1A\x07\x00";

/// Whether the headers of the archive are encrypted, as its first header says: RAR 5 starts with an archive encryption
/// header (type 4), RAR 4 sets the password flag (`0x0080`) of its main header.
pub fn headers_encrypted(file: &dyn ReadAt) -> io::Result<bool> {
    let head = file.read_chunk(0, 32)?;

    if let Some(rest) = head.strip_prefix(SIGNATURE_5) {
        // The checksum (4 bytes), the size of the header, then its type; the last two are variable-length integers.
        let after = rest.get(4..).unwrap_or_default();
        let (_, size_end) = vint(after);
        let (kind, _) = vint(after.get(size_end..).unwrap_or_default());

        return Ok(kind == Some(4));
    }

    if let Some(rest) = head.strip_prefix(SIGNATURE_4) {
        // The checksum (2 bytes), the type (0x73, the main header), then the flags.
        return Ok(matches!(rest, [_, _, 0x73, low, high, ..] if u16::from_le_bytes([*low, *high]) & 0x0080 != 0));
    }

    Ok(false)
}

/// A variable-length integer at the start of `bytes` (seven bits per byte, low first; a high bit says more follow), and
/// where it ends.
fn vint(bytes: &[u8]) -> (Option<u64>, usize) {
    let mut value = 0_u64;

    for (at, byte) in bytes.iter().enumerate().take(9) {
        value |= u64::from(byte & 0x7F) << (7 * at);

        if byte & 0x80 == 0 {
            return (Some(value), at + 1);
        }
    }

    (None, bytes.len())
}

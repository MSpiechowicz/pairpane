//! Checking a password against the content of a 7z: one encrypted entry is decoded, so the checksum that ends it (the only
//! thing that tells a wrong password on a stored entry) is checked.

use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use sevenz_rust2::Archive;

use super::{Source, decode, open};
use crate::archive::entries::Flow;
use crate::archive::error::{PasswordFault, check, password_fault, unsupported};
use crate::archive::file::ReadAt;
use crate::archive::password::Password;
use crate::archive::write::pump;

/// How much of the entry [`check_password`] probes it decodes to see whether the password opens it.
const PROBE: u64 = 64 * 1024;

/// The encrypted entry that tells most about a password for the least work: of the entries that come first in their block
/// (so decoding stops at them), the smallest, which is read to its end when it is within [`PROBE`] (always, for
/// [`confirm_password`]) and so ends in what the decoder checks there. `None` when no entry can be chosen.
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

    match probed(file, cancel, password, Some(PROBE + 1))? {
        Probe::Verdict(opens) => Ok(opens),
        Probe::Plain | Probe::Unreachable => Ok(true),
    }
}

/// [`check_password`] for what is about to seal new content with the password: the entry (see [`probe_entry`]) is read to
/// its end, whatever its length, so the checksum that ends it tells a wrong password a stored entry cannot show before. An
/// entry that fails to decode or to match its checksum counts as a wrong password (damage cannot be told from it, and
/// either way the password is not one to seal new entries with). `Ok(true)` when the archive has no encrypted content.
///
/// Only the one entry is checked: entries may be sealed under other passwords, and they are not verified.
///
/// # Errors
///
/// Reading the archive failing, a raised `cancel` (`Interrupted`) included; and `Unsupported` when the archive has
/// encrypted content but no entry of it can be read to check the password.
pub fn confirm_password(file: &dyn ReadAt, cancel: &AtomicBool, password: &Password) -> io::Result<bool> {
    match probed(file, cancel, password, None)? {
        Probe::Verdict(opens) => Ok(opens),
        Probe::Plain => Ok(true),
        Probe::Unreachable => Err(unsupported(
            "the password can't be checked, so the archive can't be changed",
        )),
    }
}

/// What [`probed`] found.
enum Probe {
    /// Whether the password opened the headers and the entry that was read.
    Verdict(bool),
    /// No entry is encrypted.
    Plain,
    /// An entry is encrypted, but none can be read to check the password.
    Unreachable,
}

/// Opens the archive in `file` with `password` and reads the entry [`probe_entry`] chooses: all of it, or `limit` bytes.
fn probed(file: &dyn ReadAt, cancel: &AtomicBool, password: &Password, limit: Option<u64>) -> io::Result<Probe> {
    let opened = match open(file, cancel, Some(password)) {
        Ok(opened) => opened,
        Err(error) if password_fault(&error).is_some() => return Ok(Probe::Verdict(false)),
        Err(error) => return Err(error),
    };

    let Some(first) = probe_entry(&opened.archive) else {
        let sealed = (0..opened.archive.files.len())
            .any(|file| opened.archive.files[file].has_stream() && opened.archive.is_file_encrypted(file));

        return Ok(if sealed { Probe::Unreachable } else { Probe::Plain });
    };

    let mut source = Source::new(file, cancel);
    // Never `io::copy`: a cancel surfaces as `Interrupted`, which it would retry forever. `pump` stops on it.
    let read = decode(&opened, &mut source, cancel, &[first], &mut |_, data| {
        pump(
            &mut data.take(limit.unwrap_or(u64::MAX)),
            &mut io::sink(),
            cancel,
            |_| {},
        )?;

        Ok(Flow::Continue)
    });

    // A cancel is never told as a wrong password.
    check(cancel)?;

    match read {
        Ok(()) => Ok(Probe::Verdict(true)),
        Err(error) if password_fault(&error) == Some(PasswordFault::Wrong) => Ok(Probe::Verdict(false)),
        Err(error) if limit.is_none() && error.kind() == io::ErrorKind::InvalidData => Ok(Probe::Verdict(false)),
        Err(error) => Err(error),
    }
}

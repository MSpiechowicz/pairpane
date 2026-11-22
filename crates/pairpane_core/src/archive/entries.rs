//! What every format hands over: the [`Record`] of one entry as an archive states it, the [`EntrySource`] that yields the
//! data of the entries a caller wants, and the [`Declared`] reader that keeps an entry's data to the size it declares.

use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use crate::view::refusal;

use super::error::{check, unreadable};
use super::index::IndexEntry;
use super::zip::ZipPlace;

/// What an entry is, as far as extracting it goes. Only [`File`](EntryType::File) and [`Dir`](EntryType::Dir) are ever
/// written; the others are listed and then skipped and counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    File,
    Dir,
    Symlink,
    Hardlink,
    /// A device, a pipe, a socket, a sparse tar entry or a type this code does not know.
    Special,
}

/// Where the data of an entry is, for the readers that need more than its place in the archive's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// Found by walking the archive in its own order up to the entry's ordinal (tar, 7z, RAR).
    Walk,
    /// A zip entry: the central directory's record, with everything a reader of the data needs.
    Zip(Box<ZipPlace>),
    /// The one entry of a gzip file. Its size is only the `ISIZE` hint of the gzip trailer (the length modulo 4 GiB), so
    /// a reader must not insist on it.
    Gz,
}

/// One entry as its archive states it, before any rule is applied to it. The walkers of the formats make these and
/// hand them to a visitor in archive order; the index builder keeps them (see [`super::index::Builder`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The path as written, decoded to text but otherwise untouched: nothing is trusted yet, so it may hold `..`, a
    /// drive, control characters or a trailing `/`. [`super::names::normalize`] decides what becomes of it.
    pub name: String,
    pub kind: EntryType,
    /// The size of the content in bytes, as declared (for a gzip file only a hint, see [`Place::Gz`]).
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch.
    pub modified: Option<i64>,
    /// The Unix permission bits the archive stores (`0o7777` at most, setuid, setgid and sticky among them: the caller
    /// strips those), or `None` when it stores none.
    pub mode: Option<u32>,
    /// Where a symbolic or hard link points, when the archive states it with the entry.
    pub link: Option<String>,
    /// The content is encrypted and needs the password.
    pub encrypted: bool,
    pub place: Place,
}

/// What a visitor says after an entry: go on or stop the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Walked {
    /// The visitor said [`Flow::Stop`].
    pub stopped: bool,
    /// The archive ended early: damaged after some entries, or longer than the scan limit allows. What was visited
    /// before is good.
    pub cut: bool,
}

/// The function an [`EntrySource`] calls with each wanted entry and a reader of its data.
pub type Visit<'v> = dyn FnMut(&IndexEntry, &mut dyn Read) -> io::Result<Flow> + 'v;

/// The data of the entries of one archive, one entry at a time. A source is made by [`super::open_source`] from an index
/// and the opened file, with the password and the cancel flag it will use.
///
/// The reader that `visit` gets holds exactly the entry's content: reading it to its end (a read that returns 0) is how
/// the caller learns the whole content was good, because the checks that need all of it (the size the archive declares,
/// a checksum) are made when the end is reached. A reader that fails says why with an error of [`super::error`]'s kinds:
/// `InvalidData` for broken or inconsistent data, a [`super::password_fault`] error for a wrong password, `Unsupported`
/// for a method that is not handled, `Interrupted` for a raised cancel flag. The caller need not read all of the data;
/// the source steps over what is left.
pub trait EntrySource {
    /// Calls `visit` for the entries of `wanted`, in the order of the list. `wanted` holds ordinals of the index,
    /// ascending and without repeats (what [`super::Selection`] gives). A source that has to walk the archive in its own
    /// order (tar, 7z, RAR) stops after the last wanted entry. Stops early when `visit` says [`Flow::Stop`]. An error of
    /// `visit` ends the call with that error; an error of the archive that no entry reader can report (a break between
    /// entries, a password that does not open the archive) ends it too.
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()>;
}

/// Reads `inner` as the content of an entry that declares `size` bytes. Fewer bytes is an error; so is a single byte
/// more, which is how an archive that holds more than it says (a bomb) is stopped before it writes it. Every
/// [`EntrySource`] wraps the decoder of an entry in one.
pub struct Declared<R> {
    inner: R,
    left: u64,
    exact: bool,
    done: bool,
}

impl<R: Read> Declared<R> {
    pub fn new(inner: R, size: u64) -> Self {
        Self {
            inner,
            left: size,
            exact: true,
            done: false,
        }
    }

    /// Reads `inner` for an entry whose size is only a limit (a gzip file): fewer bytes are fine, more are an error.
    pub fn at_most(inner: R, limit: u64) -> Self {
        Self {
            exact: false,
            ..Self::new(inner, limit)
        }
    }
}

impl<R: Read> Read for Declared<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.done || buffer.is_empty() {
            return Ok(0);
        }

        if self.left == 0 {
            // The declared size is read: one more byte means the archive holds more than it declared.
            let mut probe = [0; 1];

            if self.inner.read(&mut probe)? > 0 {
                return Err(refusal(
                    io::ErrorKind::InvalidData,
                    "an entry holds more data than it declares",
                ));
            }

            self.done = true;

            return Ok(0);
        }

        let count = read_within(&mut self.inner, self.left, buffer)?;

        if count == 0 {
            if self.exact {
                return Err(unreadable());
            }

            self.done = true;
        }

        self.left -= count as u64;

        Ok(count)
    }
}

/// Reads `inner` until `cancel` is raised, then fails every read with `Interrupted`.
pub struct Cancellable<'a, R> {
    pub inner: R,
    pub cancel: &'a AtomicBool,
}

impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check(self.cancel)?;

        self.inner.read(buffer)
    }
}

/// Reads from `inner` into `buffer`, at most `left` bytes.
pub fn read_within<R: Read>(inner: &mut R, left: u64, buffer: &mut [u8]) -> io::Result<usize> {
    let room = usize::try_from(left).map_or(buffer.len(), |left| left.min(buffer.len()));

    inner.read(&mut buffer[..room])
}

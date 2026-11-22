//! The data of the entries of a zip, read from the file at the offsets of its central directory (no walk, so an entry is
//! as cheap to reach as any other). An entry's reader decrypts, decodes, stops at the size the entry declares and checks
//! the CRC-32 at its end.
//!
//! What goes wrong with one entry is reported by that entry's reader (see [`EntrySource`]): a method that is not handled
//! is `Unsupported`, a missing or wrong password is a [`password_fault`](crate::archive::password_fault) error, a local
//! header that is not where the directory says or data that runs past the file is `InvalidData`. The other entries are
//! read as usual. What makes the whole archive untrustworthy (entries that overlap) fails [`ZipSource::new`].
//!
//! A wrong password shows itself at the start of an entry: ZipCrypto has a check byte in the 12-byte header, WinZip AES a
//! two-byte verifier. ZipCrypto's byte lets one wrong password in 256 through, so for those entries a checksum or
//! decoding failure (`InvalidData`) is reported as a wrong password too. For AES the verifier lets one in 65,536 through,
//! and a bad tag after a good verifier is damaged data.

use std::cell::RefCell;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Take};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use crate::archive::Password;
use crate::archive::entries::{Cancellable, Declared, EntrySource, EntryType, Flow, Place, Visit};
use crate::archive::error::{PasswordFault, check, password_error, password_fault, unreadable, unsupported};
use crate::archive::file::{FileReader, ReadAt};
use crate::archive::index::{ArchiveIndex, IndexEntry};
use crate::archive::write::pump;

use super::ZipPlace;
use super::decode::{Checked, Method, buffered, decoder};
use super::local::Layout;
use super::winzip_aes::{self, Opening};
use super::zipcrypto::{self, Decrypting};

/// The method that says the entry is a WinZip AES one (the real method is in the extra field).
const AES_METHOD: u16 = 99;

/// The flag of an entry that is encrypted, and the one of the "strong encryption" this code does not read.
const ENCRYPTED: u16 = 1;
const STRONG: u16 = 1 << 6;
/// The flag that says the CRC and sizes follow the data.
const DESCRIPTOR: u16 = 1 << 3;

/// The stored bytes of one entry.
type Raw<'a> = Take<FileReader<'a, dyn ReadAt + 'a>>;

/// The bytes of an AES entry that are read after the decoder is done: the rest of the ciphertext and the tag.
type Tail<'a> = Rc<RefCell<Opening<Raw<'a>>>>;

/// Lets the decoder own a reader that the entry also needs to finish.
struct Shared<R>(Rc<RefCell<R>>);

impl<R: Read> Read for Shared<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(buffer)
    }
}

/// The stored data of an entry, with its password accepted.
struct Unlocked<'a> {
    reader: Box<dyn BufRead + 'a>,
    tail: Option<Tail<'a>>,
    /// The checksum to expect of the content; `None` for AE-2.
    crc: Option<u32>,
    /// A failure of the content says the password is wrong (ZipCrypto).
    blame_password: bool,
    method: Method,
}

/// The extra field of an AES entry, validated.
fn aes_key(place: &ZipPlace) -> io::Result<(usize, u16, u16)> {
    let aes = place.aes.ok_or_else(unreadable)?;
    let key = winzip_aes::key_length(aes.strength).ok_or_else(unreadable)?;

    if !matches!(aes.version, 1 | 2) {
        return Err(unsupported("this encryption is not supported"));
    }

    Ok((key, aes.version, aes.method))
}

/// Opens the stored data `raw` of an entry: decrypts it when the entry is encrypted, which needs `password` and fails
/// with a password error when it is missing or does not open the entry.
fn unlock<'a>(place: &ZipPlace, mut raw: Raw<'a>, password: Option<&Password>) -> io::Result<Unlocked<'a>> {
    if place.flags & STRONG != 0 {
        return Err(unsupported("this encryption is not supported"));
    }

    let aes = place.method == AES_METHOD;
    let (code, version, key) = if aes {
        let (key, version, code) = aes_key(place)?;
        (code, Some(version), key)
    } else {
        (place.method, None, 0)
    };
    let method = Method::of(code)?;

    if !aes && place.flags & ENCRYPTED == 0 {
        return Ok(Unlocked {
            reader: Box::new(buffered(raw)),
            tail: None,
            crc: Some(place.crc),
            blame_password: false,
            method,
        });
    }

    let password = password.ok_or_else(|| password_error(PasswordFault::Needed))?;
    let wrong = || password_error(PasswordFault::Wrong);

    if !aes {
        let accept = zipcrypto::check_bytes(place.crc, place.dos_time, place.flags & DESCRIPTOR != 0);
        let keys = zipcrypto::open_header(&mut raw, password, &accept)?.ok_or_else(wrong)?;

        return Ok(Unlocked {
            reader: Box::new(buffered(Decrypting { inner: raw, keys })),
            tail: None,
            crc: Some(place.crc),
            blame_password: true,
            method,
        });
    }

    let salt = winzip_aes::salt_length(key);
    let overhead = (salt + winzip_aes::VERIFIER + winzip_aes::TAG) as u64;
    let length = place.compressed.checked_sub(overhead).ok_or_else(unreadable)?;
    let mut head = vec![0; salt + winzip_aes::VERIFIER];
    raw.read_exact(&mut head).map_err(|_| unreadable())?;

    let (salt, verifier) = head.split_at(salt);
    let keys = winzip_aes::Keys::derive(password, salt, key);

    if !keys.accepts([verifier[0], verifier[1]]) {
        return Err(wrong());
    }

    let tail = Rc::new(RefCell::new(Opening::new(raw, keys, length)));

    Ok(Unlocked {
        reader: Box::new(buffered(Shared(Rc::clone(&tail)))),
        tail: Some(tail),
        crc: (version == Some(1)).then_some(place.crc),
        blame_password: false,
        method,
    })
}

/// The reader of one entry's content.
struct EntryReader<'a> {
    body: Cancellable<'a, Declared<Checked<Box<dyn Read + 'a>>>>,
    tail: Option<Tail<'a>>,
    blame_password: bool,
}

impl EntryReader<'_> {
    /// The error of a read: for a ZipCrypto entry, data that does not decode or check is a wrong password.
    fn blamed(&self, error: io::Error) -> io::Error {
        if self.blame_password && error.kind() == io::ErrorKind::InvalidData && password_fault(&error).is_none() {
            return password_error(PasswordFault::Wrong);
        }

        error
    }
}

impl Read for EntryReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.body.read(buffer).map_err(|error| self.blamed(error))?;

        if count == 0
            && !buffer.is_empty()
            && let Some(tail) = &self.tail
        {
            tail.borrow_mut().finish().map_err(|error| self.blamed(error))?;
        }

        Ok(count)
    }
}

/// The reader of an entry that cannot be read: it says why on the first read.
struct Failing(Option<io::Error>);

impl Read for Failing {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(self.0.take().unwrap_or_else(unreadable))
    }
}

/// Extracts from a zip by the offsets of its central directory.
pub struct ZipSource<'a> {
    index: &'a ArchiveIndex,
    file: &'a dyn ReadAt,
    cancel: &'a AtomicBool,
    password: Option<&'a Password>,
    layout: Layout,
}

impl<'a> ZipSource<'a> {
    /// A source for the zip of `index` in `file`. Fails when the entries' ranges overlap (`InvalidData`).
    pub fn new(
        index: &'a ArchiveIndex,
        file: &'a dyn ReadAt,
        cancel: &'a AtomicBool,
        password: Option<&'a Password>,
    ) -> io::Result<Self> {
        Ok(Self {
            index,
            file,
            cancel,
            password,
            layout: Layout::new(index.entries())?,
        })
    }

    /// The raw bytes of the entry `place` stores, positioned at its start.
    fn raw(&self, place: &ZipPlace) -> io::Result<Raw<'a>> {
        let start = self.layout.data(self.file, place)?;
        let mut reader = FileReader::new(self.file);
        reader.seek(SeekFrom::Start(start))?;

        Ok(reader.take(place.compressed))
    }

    fn reader(&self, entry: &IndexEntry, place: &ZipPlace) -> io::Result<EntryReader<'a>> {
        check(self.cancel)?;

        let unlocked = unlock(place, self.raw(place)?, self.password)?;
        let decoded = decoder(unlocked.method, unlocked.reader, entry.size)?;
        let checked = Declared::new(Checked::new(decoded, unlocked.crc), entry.size);

        Ok(EntryReader {
            body: Cancellable {
                inner: checked,
                cancel: self.cancel,
            },
            tail: unlocked.tail,
            blame_password: unlocked.blame_password,
        })
    }
}

impl EntrySource for ZipSource<'_> {
    fn extract(&mut self, wanted: &[u32], visit: &mut Visit<'_>) -> io::Result<()> {
        for ordinal in wanted {
            check(self.cancel)?;

            let entry = self.index.entries().get(*ordinal as usize).ok_or_else(unreadable)?;
            let Place::Zip(place) = &entry.place else {
                return Err(unreadable());
            };

            let flow = if entry.kind == EntryType::Dir {
                visit(entry, &mut io::empty())?
            } else {
                match self.reader(entry, place) {
                    Ok(mut reader) => visit(entry, &mut reader)?,
                    Err(error) => visit(entry, &mut Failing(Some(error)))?,
                }
            };

            if flow == Flow::Stop {
                break;
            }
        }

        Ok(())
    }
}

/// Whether `password` opens the zip of `index`, judged by the first encrypted file in it: the check byte of ZipCrypto or the
/// verifier of WinZip AES. Reads a few bytes (the entry's local header and the first of its data), decodes nothing. A zip
/// with nothing encrypted accepts any password. A ZipCrypto check byte lets one wrong password in 256 through; a failure
/// later is reported again as a wrong password by the entry's reader.
pub fn check_password(index: &ArchiveIndex, file: &dyn ReadAt, password: &Password) -> io::Result<bool> {
    let encrypted = index.entries().iter().find_map(|entry| match &entry.place {
        Place::Zip(place) if entry.encrypted && entry.kind == EntryType::File => Some(place),
        _ => None,
    });
    let Some(place) = encrypted else {
        return Ok(true);
    };

    let idle = AtomicBool::new(false);
    let source = ZipSource::new(index, file, &idle, Some(password))?;

    match unlock(place, source.raw(place)?, Some(password)) {
        Ok(_) => Ok(true),
        Err(error) if password_fault(&error) == Some(PasswordFault::Wrong) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Whether `password` opens the zip of `index`, judged by decrypting a whole entry: the smallest encrypted file (one with
/// content before an empty one, which only has the check byte to tell by), read to its end, so the CRC-32 of a ZipCrypto
/// entry and the HMAC of a WinZip AES one are checked too. [`check_password`] stops at the check byte (one wrong password
/// in 256 passes) or the verifier (one in 65,536), which is enough to ask the user again but not to encrypt new entries
/// with. A zip with nothing encrypted accepts any password.
///
/// An entry that fails to decode or check after its check byte or verifier was accepted (a ZipCrypto entry that does not
/// inflate or whose CRC differs, a WinZip AES entry whose tag does not match) counts as a wrong password: damaged data
/// cannot be told from a check that let a wrong password through, and either way the password is not one to seal new
/// entries with.
///
/// # Errors
///
/// Reading the archive failing, a raised `cancel` (`Interrupted`) included.
pub fn confirm_password(
    index: &ArchiveIndex,
    file: &dyn ReadAt,
    cancel: &AtomicBool,
    password: &Password,
) -> io::Result<bool> {
    let smallest = index
        .entries()
        .iter()
        .filter(|entry| entry.encrypted && entry.kind == EntryType::File)
        .filter_map(|entry| match &entry.place {
            Place::Zip(place) => Some((entry, place)),
            _ => None,
        })
        .min_by_key(|(entry, place)| (entry.size == 0, place.compressed));
    let Some((entry, place)) = smallest else {
        return Ok(true);
    };

    let source = ZipSource::new(index, file, cancel, Some(password))?;
    let read = source
        .reader(entry, place)
        .and_then(|mut reader| pump(&mut reader, &mut io::sink(), cancel, |_| {}));

    match read {
        Ok(_) => Ok(true),
        Err(error) if password_fault(&error) == Some(PasswordFault::Wrong) => Ok(false),
        Err(error) if matches!(error.kind(), io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput) => Ok(false),
        Err(error) => Err(error),
    }
}

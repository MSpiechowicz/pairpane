//! Thumbnails: `thumb_get` makes a small preview of a picture file for the panes' thumbnail layout.
//!
//! The picture is never decoded in the app's process: `pairpane_core::view::thumb` hands it to a sandboxed helper process
//! (the app's own executable, started with its own argument; see the module docs there) and gets back the size and the
//! RGBA pixels. This module is what stands around that: what the page sent is checked, the disk is touched only on
//! blocking threads that hold a place of a [`ThreadCap`] until they end, and what was made is remembered.
//!
//! - Validation is the viewer's and the editor's: the folder passes `resolve`, the name is one plain name (`check_name`),
//!   and a request that fails is refused before a thread is taken. The file must be a regular file; a link is refused.
//! - A request holds a place of the lookup cap while it opens the file and looks in the cache. A hit never starts a
//!   helper, so a folder that was visited before shows at once. A miss reads the file under a place of the small reader
//!   cap (a file on a mount that does not answer holds that and nothing else), and only then takes one of the
//!   [`HELPER_THREADS`] places of a helper, and one more of the single [`HEIC_THREADS`] place when the picture is HEIC
//!   (its decoded picture can be a quarter of a gigabyte), for as long as the helper runs. A request that finds a cap
//!   full is refused as `busy` (the room is looked at before the file is read, so that costs no read), and the page asks
//!   again.
//! - The cache (`cache.rs`) is bound by [`CACHE_BYTES`] and keyed by the file's path and the size bucket. It is stamped
//!   with the file's length, modification time and identity (device, file number and time of change), so a changed or
//!   replaced file is made again. It also remembers the failures that are the file's own (not a picture, too big, cannot
//!   be decoded) for a small fixed cost each, with the code and text the page was told, so a file that is bad or slow to
//!   decode is not tried again until it changes; a failure of the system (a read that failed, a helper that did not
//!   start, a cancelled request) is never remembered.
//! - [`Thumbs::shut_down`] raises the flag every helper of this module watches, as the app exits, so none outlives it.
//! - What the page is told is fixed text with no path: `unsupported` for a file that is not a picture of the supported
//!   formats (and for HEIC without a decoder on the system, and on Windows, which makes no HEIC thumbnails), `invalidInput`
//!   for a picture that cannot be decoded or is too big, `busy` as above.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::listing::platform;
use pairpane_core::model::IpcErrorCode;
use pairpane_core::ops::name_fits;
use pairpane_core::view as files;
use pairpane_core::view::thumb::{self, Source};
use tauri::State;
use tauri::ipc::Response;

use self::cache::{Cache, Cached, Failed, Key, Stamp};
use super::items::{begin, blocking};
use super::limit::ThreadCap;
use super::ops::{check_name, names_too_large};
use super::panes::resolve;
use super::view::read::helper_program;
use super::view::view_error;
use crate::error::{CommandError, CommandResult};

mod cache;

#[cfg(test)]
mod tests;

/// Requests that open a file and look in the cache at once. A stuck one (a file on a hung mount) is stepped around by
/// the others; the page asks for many thumbnails at a time, and all but the ones that need a helper are quick.
const LOOKUP_THREADS: usize = 8;

/// Files read at once for a thumbnail (up to 64 MiB each, held until the helper has them or is refused). Reading is what
/// can hang, on a file of a mount that does not answer, and it must not hold a place of a helper.
const READER_THREADS: usize = 4;

/// Helper processes at once: each holds the file and a decoded picture, so a few, and a request beyond them is `busy`.
const HELPER_THREADS: usize = 3;

/// HEIC thumbnails at once, among the helpers: each decodes a whole picture of up to 256 MiB in the app.
const HEIC_THREADS: usize = 1;

/// How many bytes of thumbnails are kept: a 128-pixel one is 64 KiB, so about a thousand of those.
const CACHE_BYTES: usize = 64 * 1024 * 1024;

/// The cache, the caps on the threads and the flag that stops the helpers when the app exits.
pub struct Thumbs {
    lookups: ThreadCap,
    readers: ThreadCap,
    helpers: ThreadCap,
    heic: ThreadCap,
    cache: Arc<Mutex<Cache>>,
    cancel: Arc<AtomicBool>,
}

impl Default for Thumbs {
    fn default() -> Self {
        Self::with_budget(CACHE_BYTES)
    }
}

impl Thumbs {
    fn with_budget(bytes: usize) -> Self {
        Self {
            lookups: ThreadCap::default(),
            readers: ThreadCap::default(),
            helpers: ThreadCap::default(),
            heic: ThreadCap::default(),
            cache: Arc::new(Mutex::new(Cache::new(bytes))),
            cancel: Arc::default(),
        }
    }

    /// Stops the helpers that run and refuses any request that comes after: the app is exiting.
    pub fn shut_down(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The thumbnail of the file `name` of the folder `path`, for a square of `size`, made by the helper `program` (the
    /// app itself) unless the cache has it: the header and the RGBA pixels.
    async fn get(&self, path: String, name: String, size: u32, program: PathBuf) -> CommandResult<Arc<[u8]>> {
        let folder = resolve(Some(path), None)?;
        if !name_fits(&name) {
            return Err(names_too_large());
        }
        // Also refuses a name that is not a single plain name.
        check_name(&folder, &name)?;
        let permit = begin(&self.lookups, LOOKUP_THREADS)?;

        let size = thumb::bucket(size);
        let work = Work {
            readers: self.readers.clone(),
            helpers: self.helpers.clone(),
            heic: self.heic.clone(),
            cache: Arc::clone(&self.cache),
            cancel: Arc::clone(&self.cancel),
        };

        blocking(permit, move || work.run(&folder, &name, size, &program)).await?
    }
}

/// What the blocking thread of one request needs of [`Thumbs`].
struct Work {
    readers: ThreadCap,
    helpers: ThreadCap,
    heic: ThreadCap,
    cache: Arc<Mutex<Cache>>,
    cancel: Arc<AtomicBool>,
}

/// Why a thumbnail was not made: no place to make it in, or an error of the work.
enum Stop {
    Busy,
    Error(io::Error),
}

impl Work {
    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&self, folder: &Path, name: &str, size: u32, program: &Path) -> CommandResult<Arc<[u8]>> {
        // The folder is resolved first: a link on its path may lead to a network path, which Windows would connect to.
        let folder = platform::local_path(folder).map_err(|error| thumb_error(&error))?;
        let source = thumb::open(&folder, name).map_err(|error| thumb_error(&error))?;

        // The path as the page named it and nothing resolved after the open: the open did not follow a link at the end
        // of it, and a key that did could be the path of another file. What the file is now is in the stamp.
        let key = Key {
            path: folder.join(name),
            size,
        };
        let stamp = Stamp {
            len: source.len,
            modified: source.modified,
            identity: source.identity,
        };
        match self.cache().get(&key, &stamp) {
            Some(Cached::Picture(picture)) => return Ok(picture),
            Some(Cached::Failed(failed)) => return Err(CommandError::new(failed.code, failed.message)),
            None => {}
        }

        match self.make(&source, size, program) {
            Ok(picture) => {
                self.cache().insert(key, stamp, Cached::Picture(Arc::clone(&picture)));

                Ok(picture)
            }
            Err(Stop::Busy) => Err(busy()),
            Err(Stop::Error(error)) => {
                // What the file itself decided is remembered, so it is not decoded again until it changes.
                if let Some(failed) = failure_of(&error) {
                    self.cache().insert(key, stamp, Cached::Failed(failed));
                }

                Err(thumb_error(&error))
            }
        }
    }

    /// Reads the file under a place of the readers and runs the helper under a place of the helpers, which is not taken
    /// until the bytes are in hand.
    fn make(&self, source: &Source, size: u32, program: &Path) -> Result<Arc<[u8]>, Stop> {
        // Looked at before the file is read, so that a request that would be refused is refused without the read.
        let heic = source.is_heic();
        if !self.helpers.has_room(HELPER_THREADS) || heic && !self.heic.has_room(HEIC_THREADS) {
            return Err(Stop::Busy);
        }

        let reading = self.readers.acquire(READER_THREADS).ok_or(Stop::Busy)?;
        let bytes = thumb::read(source, &self.cancel).map_err(Stop::Error)?;
        drop(reading);

        // The places are held until the thumbnail is made.
        let _helper = self.helpers.acquire(HELPER_THREADS).ok_or(Stop::Busy)?;
        let _decoder = if heic {
            Some(self.heic.acquire(HEIC_THREADS).ok_or(Stop::Busy)?)
        } else {
            None
        };

        let picture = thumb::make(source, bytes, size, &self.cancel, program).map_err(Stop::Error)?;

        Ok(picture.into())
    }
}

/// What the page is told, and what is remembered, when a thumbnail cannot be made by a reason of the file's own: the
/// viewer's fixed texts, except that a picture that cannot be decoded is `invalidInput`, as one that is too big is (it
/// is the file that is wrong, not the disk, and asking again does not help). `None` for anything that is not one of our
/// refusals of those three kinds: an error of the system, a library or a cancelled request is not the file's.
fn failure_of(error: &io::Error) -> Option<Failed> {
    if !files::is_refusal(error) {
        return None;
    }

    let code = match error.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => IpcErrorCode::InvalidInput,
        io::ErrorKind::Unsupported => IpcErrorCode::Unsupported,
        _ => return None,
    };

    Some(Failed {
        code,
        message: error.to_string(),
    })
}

/// What the page is told when a thumbnail cannot be made (see [`failure_of`]); anything else is the viewer's text
/// (`view_error`).
fn thumb_error(error: &io::Error) -> CommandError {
    match failure_of(error) {
        Some(failed) => CommandError::new(failed.code, failed.message),
        None => view_error(error),
    }
}

fn busy() -> CommandError {
    CommandError::new(IpcErrorCode::Busy, "thumbnails are being made; try again shortly")
}

/// The thumbnail of the picture file `name` of the folder `path`, made in a helper process and kept in memory: 8 bytes
/// (width and height as `u32`, little endian), then `width * height * 4` bytes of RGBA, not premultiplied, rows tightly
/// packed. It fits a square of `size` (32 to 512, brought up to the next of a few sizes) with its aspect ratio kept and
/// is never bigger than the picture. Fails as `unsupported` for a file that is not a PNG, JPEG, GIF, WebP, BMP or HEIC
/// picture (by its first bytes, not its name) and for HEIC where the system has no decoder, `invalidInput` for a picture
/// that cannot be decoded or is too big, and `busy` when the helpers are all at work.
#[tauri::command]
pub async fn thumb_get(thumbs: State<'_, Thumbs>, path: String, name: String, size: u32) -> CommandResult<Response> {
    // The helper is the app itself. Not being able to tell where it is is not a missing file.
    let program = helper_program().map_err(|error| view_error(&io::Error::other(error)))?;

    let picture = thumbs.get(path, name, size, program).await?;

    Ok(Response::new(picture.to_vec()))
}

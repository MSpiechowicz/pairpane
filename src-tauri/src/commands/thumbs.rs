//! Thumbnails: `thumb_get` makes a small preview of a picture file for the panes' thumbnail layout.
//!
//! The picture is never decoded in the app's process: `pairpane_core::view::thumb` hands it to a sandboxed helper process
//! (the app's own executable, started with its own argument; see the module docs there) and gets back the size and the
//! RGBA pixels. This module is what stands around that: what the page sent is checked, the disk is touched only on
//! blocking threads that hold a place of a [`ThreadCap`] until they end, and what was made is remembered.
//!
//! - Validation is the viewer's and the editor's: the folder passes `resolve`, the name is one plain name (`check_name`),
//!   and a request that fails is refused before a thread is taken. The file must be a regular file; a link is refused.
//! - A request holds a place of the lookup cap while it opens the file, looks in the cache and, on a miss, makes the
//!   thumbnail. A hit never starts a helper, so a folder that was visited before shows at once. Only a miss takes one of
//!   the [`HELPER_THREADS`] places of a helper, and one more of the single [`HEIC_THREADS`] place when the picture is
//!   HEIC (its decoded picture can be a quarter of a gigabyte); a request that finds its cap full is refused as `busy`,
//!   and the page asks again.
//! - The cache (`cache.rs`) is bound by [`CACHE_BYTES`], keyed by the canonical path and the size bucket, and stamped
//!   with the file's length and modification time, so a changed file is made again.
//! - [`Thumbs::shut_down`] raises the flag every helper of this module watches, as the app exits, so none outlives it.
//! - What the page is told is fixed text with no path: `unsupported` for a file that is not a picture of the supported
//!   formats (and for HEIC without a decoder on the system), `invalidInput` for a picture that cannot be decoded or is
//!   too big, `busy` as above.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::listing::platform;
use pairpane_core::model::IpcErrorCode;
use pairpane_core::ops::name_fits;
use pairpane_core::view::{self as files, thumb};
use tauri::State;
use tauri::ipc::Response;

use self::cache::{Cache, Key, Stamp};
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

/// Helper processes at once: each holds the file and a decoded picture, so a few, and a request beyond them is `busy`.
const HELPER_THREADS: usize = 3;

/// HEIC thumbnails at once, among the helpers: each decodes a whole picture of up to 256 MiB in the app.
const HEIC_THREADS: usize = 1;

/// How many bytes of thumbnails are kept: a 128-pixel one is 64 KiB, so about a thousand of those.
const CACHE_BYTES: usize = 64 * 1024 * 1024;

/// The cache, the caps on the threads and the flag that stops the helpers when the app exits.
pub struct Thumbs {
    lookups: ThreadCap,
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
    helpers: ThreadCap,
    heic: ThreadCap,
    cache: Arc<Mutex<Cache>>,
    cancel: Arc<AtomicBool>,
}

impl Work {
    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&self, folder: &Path, name: &str, size: u32, program: &Path) -> CommandResult<Arc<[u8]>> {
        // The folder is resolved first: a link on its path may lead to a network path, which Windows would connect to.
        let folder = platform::local_path(folder).map_err(|error| thumb_error(&error))?;
        let source = thumb::open(&folder, name).map_err(|error| thumb_error(&error))?;

        let path = std::fs::canonicalize(folder.join(name)).map_err(|error| thumb_error(&error))?;
        let key = Key { path, size };
        let stamp = Stamp {
            len: source.len,
            modified: source.modified,
        };
        if let Some(picture) = self.cache().get(&key, &stamp) {
            return Ok(picture);
        }

        // The places are held until the thumbnail is made.
        let _helper = self.helpers.acquire(HELPER_THREADS).ok_or_else(busy)?;
        let _decoder = if source.is_heic() {
            Some(self.heic.acquire(HEIC_THREADS).ok_or_else(busy)?)
        } else {
            None
        };

        let picture: Arc<[u8]> = thumb::make(&source, size, &self.cancel, program)
            .map_err(|error| thumb_error(&error))?
            .into();
        self.cache().insert(key, stamp, Arc::clone(&picture));

        Ok(picture)
    }
}

/// What the page is told when a thumbnail cannot be made: the viewer's texts (`view_error`), except that a picture that
/// cannot be decoded is `invalidInput`, as one that is too big is: it is the file that is wrong, not the disk, and asking
/// again does not help.
fn thumb_error(error: &io::Error) -> CommandError {
    if error.kind() == io::ErrorKind::InvalidData && files::is_refusal(error) {
        return CommandError::new(IpcErrorCode::InvalidInput, error.to_string());
    }

    view_error(error)
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

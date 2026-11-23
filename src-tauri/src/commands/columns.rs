//! The extra columns of the detailed view: `columns_dimensions` (the size of the pictures in view) and `columns_checksum`
//! (the SHA-256 of one file). The reading is `pairpane_core::view::columns` (see its module docs for why a size is read
//! in the app's own process and what bounds it); this module is what stands around that.
//!
//! - What the page sent is checked before a thread is taken: the folder passes `resolve`, every name is one plain name
//!   (`check_name`) that fits, and a call has at most [`MAX_NAMES`] names.
//! - The disk is touched on blocking threads that hold a place of a [`ThreadCap`] until they end (a file on a mount that
//!   does not answer holds its place for good and must not use up the pool): up to [`SIZE_THREADS`] batches of sizes, and
//!   [`HASH_THREADS`] checksums, which read up to 64 MiB each. A call that finds its cap full is refused as `busy`, and
//!   the page asks again.
//! - A batch of sizes is answered in the order of its names. A name that is not a regular file (a link is not followed, a
//!   folder, a missing file, anything), that cannot be read or is no picture of a format that is read has `None`; one
//!   file never fails the batch.
//! - A checksum is refused as `invalidInput` with a fixed text when the file is over the cap, and as `notFound` or `io`
//!   by the viewer's texts (`view_error`) for the other failures. No text sent to the page holds a path.
//! - [`Columns::shut_down`] raises the flag both watch, as the app exits: a checksum stops between its chunks and a batch
//!   between its files, and none starts after.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::listing::platform;
use pairpane_core::model::{Dimensions, IpcErrorCode};
use pairpane_core::ops::name_fits;
use pairpane_core::view::columns as read;
use tauri::State;

use super::items::{begin, blocking};
use super::limit::ThreadCap;
use super::ops::{check_name, names_too_large};
use super::panes::resolve;
use super::view::view_error;
use crate::error::{CommandError, CommandResult};

#[cfg(test)]
mod tests;

/// The most names one call of `columns_dimensions` takes.
const MAX_NAMES: usize = 512;

/// Batches of sizes read at once: each reads a small piece of up to [`MAX_NAMES`] files, one after the other.
const SIZE_THREADS: usize = 4;

/// Checksums computed at once: each reads a file of up to 64 MiB.
const HASH_THREADS: usize = 2;

/// The caps on the blocking threads and the flag that stops them when the app exits.
#[derive(Default)]
pub struct Columns {
    sizes: ThreadCap,
    hashes: ThreadCap,
    cancel: Arc<AtomicBool>,
}

impl Columns {
    /// Stops the checksums and batches that run and refuses any that comes after: the app is exiting.
    pub fn shut_down(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The sizes of the pictures `names` of the folder `path`, in the order of the names.
    async fn dimensions(&self, path: String, names: Vec<String>) -> CommandResult<Vec<Option<Dimensions>>> {
        if names.len() > MAX_NAMES {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                format!("ask for at most {MAX_NAMES} names at a time"),
            ));
        }

        let folder = resolve(Some(path), None)?;
        for name in &names {
            checked(&folder, name)?;
        }
        let permit = begin(&self.sizes, SIZE_THREADS)?;
        let cancel = Arc::clone(&self.cancel);

        let sizes = blocking(permit, move || {
            let folder = platform::local_path(&folder)?;

            names
                .iter()
                .map(|name| {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(io::Error::from(io::ErrorKind::Interrupted));
                    }

                    Ok(read::dimensions(&folder, name))
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .await?;

        sizes.map_err(|error| view_error(&error))
    }

    /// The SHA-256 of the file `name` of the folder `path`, as lowercase hex.
    async fn checksum(&self, path: String, name: String) -> CommandResult<String> {
        let folder = resolve(Some(path), None)?;
        checked(&folder, &name)?;
        let permit = self.hashes.acquire(HASH_THREADS).ok_or_else(busy)?;
        let cancel = Arc::clone(&self.cancel);

        let hash = blocking(permit, move || {
            let folder = platform::local_path(&folder)?;

            read::checksum(&folder, &name, &cancel)
        })
        .await?;

        hash.map_err(|error| view_error(&error))
    }
}

/// A name is one plain name that fits, inside `folder`.
fn checked(folder: &std::path::Path, name: &str) -> CommandResult<()> {
    if !name_fits(name) {
        return Err(names_too_large());
    }

    check_name(folder, name)
}

fn busy() -> CommandError {
    CommandError::new(IpcErrorCode::Busy, "checksums are being computed; try again shortly")
}

/// The sizes in pixels of the picture files `names` of the folder `path`, read from their headers: one entry for each name,
/// in order, `null` for a name that is not a regular file (a link is not followed), cannot be read or is not a PNG, JPEG,
/// GIF, BMP, WebP, HEIC or AVIF picture whose header says its size. At most 512 names (`invalidInput` beyond that, or for a
/// name that is not a plain name), and `busy` when too many batches are being read.
#[tauri::command]
pub async fn columns_dimensions(
    columns: State<'_, Columns>,
    path: String,
    names: Vec<String>,
) -> CommandResult<Vec<Option<Dimensions>>> {
    columns.dimensions(path, names).await
}

/// The SHA-256 of the file `name` of the folder `path` as lowercase hex. `invalidInput` ("the file is too big to checksum
/// here") for a file over 64 MiB or one that is not a regular file, `busy` when two checksums are already being computed.
#[tauri::command]
pub async fn columns_checksum(columns: State<'_, Columns>, path: String, name: String) -> CommandResult<String> {
    columns.checksum(path, name).await
}

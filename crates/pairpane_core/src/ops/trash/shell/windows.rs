//! The Recycle Bin on Windows: the `trash` crate lists it and restores from it.

use std::fs;
use std::io;

use ::trash::{Error, TrashItem, os_limited};

use super::{Trashed, UntrashFailure, in_the_way};

pub(super) fn list(_record: &Trashed) -> Result<Vec<TrashItem>, UntrashFailure> {
    os_limited::list().map_err(|error| {
        tracing::warn!(?error, "cannot list the Recycle Bin");

        UntrashFailure::Other
    })
}

/// Puts `items` back and returns how many.
///
/// The crate looks whether the place is taken with `exists`, which follows a link: a link that leads nowhere reads as
/// free, and the shell, which runs without a prompt, would replace it. So every place is looked at here first, without
/// following links, and anything at one of them blocks the restore before an item moves. An item that is made at a
/// place after this look, and before the shell moves its own, can still be replaced; no check can close that gap.
pub(super) fn restore(items: Vec<TrashItem>) -> Result<usize, UntrashFailure> {
    let count = items.len();

    for item in &items {
        let place = item.original_path();

        match fs::symlink_metadata(&place) {
            Ok(_) => return Err(in_the_way(&place)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(?error, "cannot look at the place of an item in the Recycle Bin");

                return Err(UntrashFailure::Other);
            }
        }
    }

    match os_limited::restore_all(items) {
        Ok(()) => Ok(count),
        Err(Error::RestoreCollision { path, .. }) => Err(in_the_way(&path)),
        Err(error) => {
            tracing::warn!(?error, "cannot restore from the Recycle Bin");

            Err(UntrashFailure::Other)
        }
    }
}

//! Systems with no way to list the Trash or restore from it (macOS): the crate only moves.

use ::trash::TrashItem;

use super::{Trashed, UntrashFailure};

pub(super) fn list(_record: &Trashed) -> Result<Vec<TrashItem>, UntrashFailure> {
    Err(UntrashFailure::Unsupported)
}

pub(super) fn restore(_items: Vec<TrashItem>) -> Result<usize, UntrashFailure> {
    Err(UntrashFailure::Unsupported)
}

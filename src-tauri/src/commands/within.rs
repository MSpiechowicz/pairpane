//! The guard of a change to the items of a search result's folder.
//!
//! A search lists items by their paths below the folder searched, and the page then acts on one folder of them at a time
//! (a copy, the Trash, a delete) by the folder's path. Between the search and the action a folder on the way may have
//! been swapped for a link that leads somewhere else, and the path would then reach what the search never saw. A command
//! the page sends with a [`Within`] asks the backend to check first: the folder `dir` below `root` is resolved one name at
//! a time from `root`, never through a link or a file, and has to be the same folder (by identity, not by spelling) as
//! the folder the command acts on. Anything else is refused with the fixed [`CHANGED`] text.
//!
//! What the check does not do: it keeps no identity from the time of the search, so a real folder that was moved into
//! the place of a result's folder after the search is not noticed (the results then name what is there now). Nor does it
//! close the gap between the check and the action that follows, which opens its folder by path: a folder swapped inside
//! that gap is the race every pane has, and the check makes it a matter of microseconds and not of the time the results
//! were on the screen.

use std::io;
use std::path::Path;

use pairpane_core::listing::platform;
use pairpane_core::model::IpcErrorCode;
use pairpane_core::ops::{check_same_folder_below, name_fits};
use serde::Deserialize;

use super::items::{begin, blocking};
use super::limit::ThreadCap;
use super::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// What the page is told when the folder is not the one that was searched.
pub const CHANGED: &str = "A folder on the way was changed; search again";

/// The most folders a `dir` may name: the depth a search goes to.
const MAX_FOLDERS: usize = 1_024;

/// Where the folder of search results is: `dir` is its path below the folder `root` that was searched, names joined with
/// `/` (`""`: `root` itself).
#[derive(Debug, Clone, Deserialize)]
pub struct Within {
    root: String,
    dir: String,
}

/// Checks, when `within` is given, that the folder `path` is the folder `within` names, on a blocking thread that holds a
/// place of `cap` (a stuck folder must not use up the thread pool). Without `within` there is nothing to check.
///
/// The syntax is checked first and on the command's thread; a bad one is `invalidInput`, and a folder that is not the one
/// searched is `invalidInput` with [`CHANGED`] (not `forbidden`: the page shows no text for that).
pub(super) async fn confirm(cap: &ThreadCap, max: usize, within: Option<Within>, path: &str) -> CommandResult<()> {
    let Some(within) = within else {
        return Ok(());
    };

    let root = resolve(Some(within.root), None)?;
    let folder = resolve(Some(path.to_owned()), None)?;
    let dir = checked_dir(within.dir)?;
    let permit = begin(cap, max)?;

    let same = blocking(permit, move || is_same(&root, &dir, &folder)).await?;

    same.map_err(|_| CommandError::new(IpcErrorCode::InvalidInput, CHANGED))
}

/// `dir` when it is `""` or plain names joined with `/`, few and short enough.
fn checked_dir(dir: String) -> CommandResult<String> {
    let plain = dir.is_empty()
        || (dir.split('/').count() <= MAX_FOLDERS
            && dir
                .split('/')
                .all(|name| platform::is_plain_name(name) && name_fits(name)));

    if plain {
        Ok(dir)
    } else {
        Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "the folder of a result must be plain names below the folder searched",
        ))
    }
}

/// Whether `dir` below `root` is `folder`. Any failure to tell is a no.
fn is_same(root: &Path, dir: &str, folder: &Path) -> io::Result<()> {
    let root = platform::local_path(root)?;
    let folder = platform::local_path(folder)?;

    check_same_folder_below(&root, dir, &folder)
}

#[cfg(test)]
pub(super) mod tests;

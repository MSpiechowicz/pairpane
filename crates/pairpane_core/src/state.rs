//! What the app remembers between runs: the tabs of both panes, the active pane and the hotlist, in one small JSON
//! file (`{ "version": 1, ...SavedState }`).
//!
//! The file and everything the webview sends to be saved is untrusted, so both directions go through [`clean`]: paths
//! are checked by their syntax only ([`platform::check_folder`]; nothing on disk is touched, so a path on a dead
//! mount costs nothing), and the number of tabs, favourites and saved searches is capped (a saved search must also be
//! able to run, see [`searches`]). A file that is too big, not JSON, of
//! another version or in any way unexpected is not an error: the app starts with its defaults.
//!
//! A save that leaves out the searches (see [`SavedUpdate`]) keeps the stored ones; one that sends an empty list clears
//! them.
//!
//! Writes replace the file atomically (a temporary file, then a rename), so a crash never leaves half a file.
//! Log lines never contain a path.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::listing::platform::check_folder;
use crate::model::{SavedPane, SavedState, SavedTab};

mod searches;

pub use searches::{MAX_SEARCH_NAME_CHARS, MAX_SEARCHES, SavedUpdate};

/// The layout of the file. A file of another version is ignored (and replaced by the next save).
pub const VERSION: u32 = 1;

/// The biggest state file read or written, in bytes.
pub const MAX_FILE_BYTES: u64 = 1_048_576;

/// Tabs kept per pane.
pub const MAX_TABS: usize = 32;

/// Favourite folders kept.
pub const MAX_HOTLIST: usize = 100;

/// The file as written: the version beside the state's own fields.
#[derive(Serialize)]
struct Versioned<'a> {
    version: u32,
    #[serde(flatten)]
    state: &'a SavedState,
}

/// Just the version of a file, read first so that a file of another layout is refused whatever else is in it.
#[derive(Deserialize)]
struct Header {
    version: u32,
}

/// `state` with everything unusable removed: tabs and favourites whose path is not a plain local absolute folder
/// path, tabs beyond [`MAX_TABS`], favourites beyond [`MAX_HOTLIST`] and repeated favourites. The active tab of a pane
/// stays active when it survives, else the next surviving tab (the last one when none follows) is. Saved searches are
/// cleaned by [`searches::clean_searches`].
pub fn clean(state: SavedState) -> SavedState {
    let mut seen = HashSet::new();
    let hotlist = state
        .hotlist
        .into_iter()
        .filter(|path| is_usable(path) && seen.insert(path.clone()))
        .take(MAX_HOTLIST)
        .collect();

    SavedState {
        active: state.active,
        left: clean_pane(state.left),
        right: clean_pane(state.right),
        hotlist,
        searches: searches::clean_searches(state.searches),
    }
}

fn clean_pane(pane: SavedPane) -> SavedPane {
    let active = usize::try_from(pane.active).unwrap_or(usize::MAX);

    // The position the active tab has among the survivors: the number of survivors before it.
    let mut position = 0;
    let mut tabs: Vec<SavedTab> = Vec::new();
    for (index, tab) in pane.tabs.into_iter().enumerate() {
        if !is_usable(&tab.path) {
            continue;
        }
        if index < active {
            position += 1;
        }
        tabs.push(tab);
    }
    tabs.truncate(MAX_TABS);

    let active = position.min(tabs.len().saturating_sub(1));

    SavedPane {
        tabs,
        active: u32::try_from(active).unwrap_or(0),
    }
}

/// Whether a saved path may be opened: it passes the same syntax check as a path from the pane.
fn is_usable(path: &str) -> bool {
    check_folder(Path::new(path)).is_ok()
}

/// Reads and cleans the state saved in `file`. `None` (the app starts with its defaults) when there is no file or it
/// cannot be used; why is logged, without any path.
///
/// Only `file` is touched. At most [`MAX_FILE_BYTES`] are read, and anything that is not a regular file is refused
/// before it is opened (opening a named pipe would wait for a writer).
pub fn load(file: &Path) -> Option<SavedState> {
    let regular = match fs::metadata(file) {
        Ok(metadata) => metadata.is_file(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(kind = ?error.kind(), "cannot read the saved state");
            return None;
        }
    };
    if !regular {
        tracing::warn!("the saved state is not a regular file");
        return None;
    }

    let mut bytes = Vec::new();
    let read = File::open(file).and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes));
    if let Err(error) = read {
        tracing::warn!(kind = ?error.kind(), "cannot read the saved state");
        return None;
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        tracing::warn!("the saved state is too big; starting with defaults");
        return None;
    }

    parse(&bytes)
}

/// The state in the bytes of a state file, cleaned; `None` for a wrong version or anything that is not a state.
fn parse(bytes: &[u8]) -> Option<SavedState> {
    let Ok(header) = serde_json::from_slice::<Header>(bytes) else {
        tracing::warn!("the saved state is not valid; starting with defaults");
        return None;
    };
    if header.version != VERSION {
        tracing::warn!(
            version = header.version,
            "the saved state has another version; starting with defaults"
        );
        return None;
    }

    match serde_json::from_slice::<SavedState>(bytes) {
        Ok(state) => Some(clean(state)),
        Err(_) => {
            tracing::warn!("the saved state is not valid; starting with defaults");
            None
        }
    }
}

/// Cleans `state` and writes it to `file`, replacing the file as a whole (the folder is created when missing). A
/// state that would not fit in [`MAX_FILE_BYTES`], and so could not be loaded, is refused as invalid input.
pub fn save(file: &Path, state: SavedState) -> io::Result<()> {
    save_changed(file, state, &mut None).map(drop)
}

/// Like [`save`] for a state the webview sent: when it has no `searches` the ones stored in `file` are kept (none when
/// the file cannot be used), when it has them, even none, they replace the stored ones. Otherwise as [`save_changed`].
pub fn save_update(file: &Path, update: SavedUpdate, last: &mut Option<Vec<u8>>) -> io::Result<bool> {
    let state = update.resolve(|| load(file).map(|stored| stored.searches).unwrap_or_default());

    save_changed(file, state, last)
}

/// Like [`save`], but writes nothing when the cleaned state is the very bytes `last` holds, those of the last write
/// that succeeded (a page that saves the same state over and over costs no disk writes). Returns whether the file was
/// written; `last` then holds the new bytes. It is left alone when the write fails.
pub fn save_changed(file: &Path, state: SavedState, last: &mut Option<Vec<u8>>) -> io::Result<bool> {
    let state = clean(state);
    let bytes = serde_json::to_vec_pretty(&Versioned {
        version: VERSION,
        state: &state,
    })
    .map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the state is too big to save",
        ));
    }
    if last.as_deref() == Some(bytes.as_slice()) {
        return Ok(false);
    }

    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }

    // Written and flushed completely before it takes the place of the old file.
    let temporary = file.with_added_extension("tmp");
    let written = write_synced(&temporary, &bytes).and_then(|()| fs::rename(&temporary, file));
    if let Err(error) = written {
        fs::remove_file(&temporary).ok();

        return Err(error);
    }
    *last = Some(bytes);

    Ok(true)
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;

    file.sync_all()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod searches_tests;

//! Saved state: `state_load` and `state_save` read and write the file the app keeps between runs (see
//! `pairpane_core::state`).
//!
//! The file lives in the app's config folder, which may be on a share that does not respond, so both run on a
//! blocking thread, and only one at a time: while a request has its thread (a load or a save stuck on such a share)
//! another is refused as busy, so a page cannot use up threads, and no two writes ever overlap. A refused save is the
//! page's to repeat with its latest state. A save of the very bytes last written (what a page that saves on every
//! change sends again and again) is answered as done without touching the disk. What the page sends to save is
//! cleaned before it is written (paths checked by syntax, sizes capped, saved searches that could not run dropped), and
//! nothing on disk but the file is touched. A state sent without its `searches` leaves the saved searches as they are.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use pairpane_core::model::{IpcErrorCode, SavedState};
use pairpane_core::state::{self, SavedUpdate};
use tauri::State;
use tauri::async_runtime;

use super::limit::{Permit, ThreadCap};
use crate::error::{CommandError, CommandResult};

/// The name of the file in the config folder.
pub const STATE_FILE: &str = "state.json";

/// Where the state is kept and the one thread that may touch it.
pub struct StateStore {
    /// `None` when the system has no config folder; nothing is then loaded or saved.
    file: Option<PathBuf>,
    threads: ThreadCap,
    /// The bytes of the last successful write, so that saving the same state again writes nothing. Only the thread
    /// that holds the one permit uses it.
    written: Arc<Mutex<Option<Vec<u8>>>>,
}

/// Threads that may read or write the file at once.
const THREADS: usize = 1;

/// A started request: the file to use and the place its thread holds until it ends.
#[derive(Debug)]
struct Job {
    file: PathBuf,
    written: Arc<Mutex<Option<Vec<u8>>>>,
    permit: Permit,
}

impl StateStore {
    /// A store for the state file at `file`.
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            file,
            threads: ThreadCap::default(),
            written: Arc::default(),
        }
    }

    /// Starts a request; refused as busy while an earlier one still has its thread. `None` without a state file.
    fn begin(&self) -> CommandResult<Option<Job>> {
        let Some(file) = self.file.clone() else {
            return Ok(None);
        };
        let permit = self.threads.acquire(THREADS).ok_or_else(|| {
            CommandError::new(
                IpcErrorCode::Busy,
                "the saved settings are not responding; try again shortly",
            )
        })?;

        Ok(Some(Job {
            file,
            written: Arc::clone(&self.written),
            permit,
        }))
    }
}

/// The state saved by the last run, cleaned; `null` when there is none or it cannot be used (the app starts with
/// its defaults).
#[tauri::command]
pub async fn state_load(store: State<'_, StateStore>) -> CommandResult<Option<SavedState>> {
    let Some(job) = store.begin()? else {
        return Ok(None);
    };

    let loaded = async_runtime::spawn_blocking(move || {
        let loaded = state::load(&job.file);
        drop(job.permit);

        loaded
    })
    .await;

    loaded.map_err(|_| internal())
}

/// Saves `state`, cleaned, replacing what was saved, except that a state sent without `searches` keeps the saved
/// searches (an explicit empty list clears them). Refused as busy while an earlier request has not finished; the page
/// saves again with its newest state.
#[tauri::command]
pub async fn state_save(store: State<'_, StateStore>, state: SavedUpdate) -> CommandResult<()> {
    let Some(job) = store.begin()? else {
        return Err(CommandError::new(
            IpcErrorCode::Unsupported,
            "there is no place to keep settings on this system",
        ));
    };

    let saved = async_runtime::spawn_blocking(move || {
        let saved = save(&job, state);
        drop(job.permit);

        saved
    })
    .await;

    saved
        .map_err(|_| internal())?
        .map(drop)
        .map_err(|error| save_error(&error))
}

/// Writes `state` unless it is what the last write left in the file. Returns whether it wrote.
fn save(job: &Job, state: SavedUpdate) -> io::Result<bool> {
    let mut last = job.written.lock().unwrap_or_else(PoisonError::into_inner);

    state::save_update(&job.file, state, &mut last)
}

fn internal() -> CommandError {
    CommandError::new(IpcErrorCode::Internal, "the settings task failed")
}

/// What the page is told when the state cannot be written. Never the error itself: it may name the folder.
fn save_error(error: &io::Error) -> CommandError {
    match error.kind() {
        io::ErrorKind::InvalidInput => CommandError::new(IpcErrorCode::InvalidInput, "the state is too big to save"),
        _ => CommandError::new(IpcErrorCode::Io, "Cannot save the settings"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pairpane_core::model::{FindCriteria, PaneId, SavedPane, SavedSearch};

    use super::*;

    fn empty() -> SavedUpdate {
        let pane = || SavedPane {
            tabs: Vec::new(),
            active: 0,
        };

        SavedUpdate {
            active: PaneId::Left,
            left: pane(),
            right: pane(),
            hotlist: Vec::new(),
            searches: Some(Vec::new()),
            renames: None,
            keys: None,
            preferences: None,
        }
    }

    fn store() -> StateStore {
        StateStore::new(Some(PathBuf::from("state.json")))
    }

    /// A store whose file is in a folder of its own, which lives as long as the first part.
    fn store_in_temp_dir() -> (tempfile::TempDir, PathBuf, StateStore) {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("state.json");
        let store = StateStore::new(Some(file.clone()));

        (dir, file, store)
    }

    #[test]
    fn a_request_is_refused_while_the_earlier_one_has_its_thread() {
        let store = store();
        let running = store.begin().expect("free").expect("a file");

        let error = store.begin().expect_err("busy");
        assert_eq!(error.code(), IpcErrorCode::Busy);

        drop(running.permit);
        assert!(store.begin().expect("free again").is_some());
    }

    #[test]
    fn saving_the_same_state_twice_writes_once() {
        let (_dir, file, store) = store_in_temp_dir();
        let job = || store.begin().expect("free").expect("a file");

        assert!(save(&job(), empty()).expect("first"));
        fs::remove_file(&file).expect("removed, so that a write would bring it back");

        assert!(!save(&job(), empty()).expect("second"), "identical: not written");
        assert!(!file.exists());
    }

    fn search(name: &str) -> SavedSearch {
        SavedSearch {
            name: name.to_owned(),
            criteria: FindCriteria::default(),
        }
    }

    #[test]
    fn a_page_that_sends_no_searches_keeps_the_saved_ones_and_an_empty_list_clears_them() {
        let (_dir, file, store) = store_in_temp_dir();
        let job = || store.begin().expect("free").expect("a file");
        let searches = |file: &PathBuf| state::load(file).expect("state").searches;
        let from_wire = |json: &str| serde_json::from_str::<SavedUpdate>(json).expect("an update");
        let pane = r#"{"tabs":[],"active":0}"#;
        let without = format!(r#"{{"active":"left","left":{pane},"right":{pane},"hotlist":[]}}"#);

        let mut with = empty();
        with.searches = Some(vec![search("big"), search("old")]);
        save(&job(), with).expect("saved with searches");

        save(&job(), from_wire(&without)).expect("saved without searches");
        assert_eq!(searches(&file), vec![search("big"), search("old")], "kept");

        let null = without.replace(r#""hotlist":[]"#, r#""hotlist":[],"searches":null"#);
        save(&job(), from_wire(&null)).expect("saved with null searches");
        assert_eq!(searches(&file).len(), 2, "null is absent too");

        let cleared = without.replace(r#""hotlist":[]"#, r#""hotlist":[],"searches":[]"#);
        save(&job(), from_wire(&cleared)).expect("saved with none");
        assert_eq!(searches(&file), Vec::new(), "an explicit empty list clears them");
    }

    #[test]
    fn without_a_config_folder_there_is_nothing_to_do_and_nothing_is_counted() {
        let store = StateStore::new(None);

        assert!(store.begin().expect("not busy").is_none());
        assert!(store.begin().expect("still not busy").is_none());
    }

    #[test]
    fn write_errors_tell_the_page_nothing_about_the_folder() {
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "/home/me/.config/pairpane: denied");

        let error = save_error(&denied);

        assert_eq!(error.code(), IpcErrorCode::Io);
        let wire = serde_json::to_string(&error).expect("serializable");
        assert!(!wire.contains("/home"), "{wire}");
        assert_eq!(
            save_error(&io::Error::new(io::ErrorKind::InvalidInput, "big")).code(),
            IpcErrorCode::InvalidInput
        );
    }
}

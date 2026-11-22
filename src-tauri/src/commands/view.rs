//! The file viewer: sessions over open files, and the commands that read them one window at a time.
//!
//! `view_open` opens a regular file (never a link) and returns a session number; the open file stays in the session,
//! so a later rename or delete changes nothing, and every other command names the session and never a path. At most
//! [`MAX_SESSIONS`] sessions live at once: opening another closes the oldest one. A session is also closed by
//! `view_close`, which is safe to call twice.
//!
//! What the page sent is checked on the command's thread, and a request that fails the check is refused before a
//! thread is taken (the same rule as `items.rs`). The disk is touched only on blocking threads, each holding a place
//! of a [`ThreadCap`] until it ends: a read on a hung mount never returns and must not use up the thread pool, so
//! reads, searches, media requests and HEIC decodes (each of which runs a helper process and holds a large buffer, so
//! one at a time) have caps of their own and a request that finds its cap full is refused as busy. No text sent to the
//! page holds a path.
//!
//! - `archive_entry.rs`: `archive_view_open`, an entry of an archive extracted to a private temporary file, opened as a
//!   session and deleted at once; the stale ones are swept at start-up and exit.
//! - `read.rs`: raw bytes, text windows, Markdown and archive listings (the last two stop when the session ends) and
//!   HEIC pictures decoded to pixels by the system's own codecs (`view_image_decode`).
//! - `diff.rs`: `diff_next_difference`, where the files of two sessions next differ (the hex diff), on the search cap.
//! - `search.rs`: the search of a session, cancelled by the next search, by `view_search_cancel` and by closing.
//! - `media.rs`: the tokens `view_media` hands out; `serve.rs`: the `pairpane-media` scheme that answers them.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pairpane_core::model::{IpcErrorCode, ViewKind, ViewOpened};
use pairpane_core::ops::name_fits;
use pairpane_core::view::{self as files, Opened, ViewFile};
use tauri::State;

use self::media::Token;
use super::items::{begin, in_folder};
use super::limit::ThreadCap;
use super::ops::{check_name, names_too_large};
use super::panes::resolve;
use crate::error::{CommandError, CommandResult};

pub mod archive_entry;
pub mod diff;
pub mod media;
pub mod read;
pub mod search;
pub mod serve;

#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod heic_tests;
#[cfg(test)]
mod serve_tests;
#[cfg(test)]
mod tests;

/// The sessions that live at once; opening one more closes the oldest.
const MAX_SESSIONS: usize = 8;

/// Threads for opening files and reading windows of them, for searches (which can run long, so they have places of
/// their own) and for the media scheme's reads. A stuck one is stepped around by the others.
const READ_THREADS: usize = 3;
const SEARCH_THREADS: usize = 2;
const MEDIA_THREADS: usize = 2;
/// HEIC decodes: each starts a process and holds the file and a picture of up to 256 MiB, so one at a time, and apart
/// from the reads so that opening and reading are not held up by a decode. A second one is refused as busy.
const DECODE_THREADS: usize = 1;

/// The sessions, the tokens for their media and the caps on the threads that serve them. Clones share all of it (the
/// media scheme hands a clone to its blocking thread).
#[derive(Clone, Default)]
pub struct Viewer {
    state: Arc<Mutex<Sessions>>,
    reads: ThreadCap,
    decodes: ThreadCap,
    searches: ThreadCap,
    media: ThreadCap,
}

#[derive(Default)]
struct Sessions {
    /// The number given to the latest session; the next one is one more.
    last: u32,
    /// Oldest first.
    sessions: Vec<Session>,
    /// Token text to what it serves; at most [`media::MAX_TOKENS`].
    tokens: HashMap<String, Token>,
}

struct Session {
    id: u32,
    file: Arc<ViewFile>,
    kind: ViewKind,
    /// The flag of the search that runs now, which the next search (or closing the session) raises.
    search: Option<Arc<AtomicBool>>,
    /// Raised when the session ends (closed, or the oldest of too many); the reads that run on it stop.
    cancel: Arc<AtomicBool>,
}

/// The files a change to the state let go of. They are dropped by the caller after the lock is released, so closing
/// an open file never happens while the state is locked.
type Released = Vec<Arc<ViewFile>>;

/// What a command needs of a session: its open file, what the file is and the flag that stops its reads.
struct Handle {
    file: Arc<ViewFile>,
    kind: ViewKind,
    cancel: Arc<AtomicBool>,
}

impl Viewer {
    fn lock(&self) -> MutexGuard<'_, Sessions> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the file `name` of the folder `path` on a blocking thread and starts a session on it.
    async fn open(&self, path: String, name: String) -> CommandResult<ViewOpened> {
        let folder = resolve(Some(path), None)?;
        if !name_fits(&name) {
            return Err(names_too_large());
        }
        // Also refuses a name that is not a single plain name.
        check_name(&folder, &name)?;
        let permit = begin(&self.reads, READ_THREADS)?;

        let opened = in_folder(permit, folder, move |actual| files::open(actual, &name))
            .await?
            .map_err(|error| view_error(&error))?;

        Ok(self.start(opened))
    }

    /// Starts a session on the file that was opened and tells what the page needs to show it.
    fn start(&self, opened: Opened) -> ViewOpened {
        let (session, released) = self.lock().add(opened.file, opened.kind.clone());
        drop(released);

        ViewOpened {
            session,
            name: opened.name,
            size: opened.size,
            modified: opened.modified,
            kind: opened.kind,
            encoding: opened.encoding,
        }
    }

    /// Ends the session: its search is cancelled, its media tokens stop working and its file is closed once nothing
    /// reads it. A session that is not there is left alone.
    fn close(&self, session: u32) {
        let released = self.lock().remove(session);
        drop(released);
    }

    /// Ends every session, as the app exits: the flag of each is raised, so a HEIC decode that runs stops its helper
    /// process (which would otherwise be left to run until its own deadline or until the pipe to it breaks).
    pub fn close_all(&self) {
        let released: Released = {
            let mut state = self.lock();
            let ids: Vec<u32> = state.sessions.iter().map(|session| session.id).collect();

            ids.into_iter().flat_map(|id| state.remove(id)).collect()
        };
        drop(released);
    }

    /// The open file and kind of `session`.
    fn handle(&self, session: u32) -> CommandResult<Handle> {
        let state = self.lock();
        let found = state.sessions.iter().find(|found| found.id == session);

        found
            .map(|found| Handle {
                file: Arc::clone(&found.file),
                kind: found.kind.clone(),
                cancel: Arc::clone(&found.cancel),
            })
            .ok_or_else(no_session)
    }
}

impl Sessions {
    /// Starts a session on `file`, closing the oldest ones while there are too many.
    fn add(&mut self, file: ViewFile, kind: ViewKind) -> (u32, Released) {
        let mut released = Released::new();
        while self.sessions.len() >= MAX_SESSIONS {
            let oldest = self.sessions[0].id;
            released.extend(self.remove(oldest));
        }

        let mut id = self.last;
        loop {
            id = id.wrapping_add(1);
            if id != 0 && self.sessions.iter().all(|session| session.id != id) {
                break;
            }
        }
        self.last = id;

        self.sessions.push(Session {
            id,
            file: Arc::new(file),
            kind,
            search: None,
            cancel: Arc::default(),
        });

        (id, released)
    }

    /// Removes the session and its tokens and cancels its search and its reads.
    fn remove(&mut self, id: u32) -> Released {
        let mut released = Released::new();

        if let Some(at) = self.sessions.iter().position(|session| session.id == id) {
            let session = self.sessions.remove(at);
            if let Some(search) = &session.search {
                search.store(true, Ordering::Relaxed);
            }
            session.cancel.store(true, Ordering::Relaxed);
            released.push(session.file);
        }

        released.extend(
            self.tokens
                .extract_if(|_, token| token.session == id)
                .map(|(_, token)| token.file),
        );

        released
    }
}

/// Opens the file `name` of the folder `path` for viewing and returns its session: what the file is (text, Markdown, an
/// image, a PDF, audio, video, an archive or something else), its size and time, and the encoding found for text. A
/// link, a folder or anything that is not a regular file is refused.
#[tauri::command]
pub async fn view_open(viewer: State<'_, Viewer>, path: String, name: String) -> CommandResult<ViewOpened> {
    viewer.open(path, name).await
}

/// Ends the session `session`; one that is not there is not an error, so closing twice is safe.
#[tauri::command]
pub async fn view_close(viewer: State<'_, Viewer>, session: u32) -> CommandResult<()> {
    viewer.close(session);

    Ok(())
}

fn no_session() -> CommandError {
    CommandError::new(IpcErrorCode::NotFound, "there is no such viewer session")
}

/// What the page is told when a read fails: fixed lowercase text per kind of failure (the viewer's own refusals, which
/// hold no path, keep their text), never the system's wording and never a path. Only an error made by
/// `pairpane_core::view::refusal` counts as such: any other error with a text (a system call, a library, a helper
/// process that did not start) gets the generic text of its kind. The details go to the log.
pub(super) fn view_error(error: &io::Error) -> CommandError {
    let own_text = files::is_refusal(error);

    match error.kind() {
        io::ErrorKind::NotFound => CommandError::new(IpcErrorCode::NotFound, "the file no longer exists"),
        io::ErrorKind::Interrupted => CommandError::new(IpcErrorCode::Io, "the read was cancelled"),
        io::ErrorKind::PermissionDenied => {
            CommandError::new(IpcErrorCode::Io, "the file cannot be read: no permission")
        }
        io::ErrorKind::InvalidInput if own_text => CommandError::new(IpcErrorCode::InvalidInput, error.to_string()),
        io::ErrorKind::InvalidData if own_text => CommandError::new(IpcErrorCode::Io, error.to_string()),
        io::ErrorKind::Unsupported if own_text => CommandError::new(IpcErrorCode::Unsupported, error.to_string()),
        _ => {
            tracing::debug!(%error, "viewer read failed");

            CommandError::new(IpcErrorCode::Io, "the file cannot be read")
        }
    }
}

/// A byte count or a name that the viewer will not take.
fn invalid(text: &'static str) -> CommandError {
    CommandError::new(IpcErrorCode::InvalidInput, text)
}

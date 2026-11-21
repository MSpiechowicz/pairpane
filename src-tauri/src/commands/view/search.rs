//! The search of a session. One search runs per session: starting another, cancelling it and closing the session all
//! raise the flag of the one that runs, which stops before its next 1 MiB chunk.
//!
//! A search that was stopped that way ends with `null`, the same answer as for a needle that is not in the file: the
//! page knows which search it stopped (it started the newer one, or asked to cancel) and drops that answer.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::model::{SearchHit, SearchNeedle, SearchQuery};
use pairpane_core::view::search as scan;
use tauri::State;
use tauri::ipc::Channel;

use super::read::known;
use super::{SEARCH_THREADS, Viewer, invalid, view_error};
use crate::commands::items::{begin, blocking};
use crate::error::CommandResult;

/// The most characters of a text needle, and bytes of a hex one.
const MAX_NEEDLE: usize = pairpane_core::view::MAX_NEEDLE;

impl Viewer {
    /// Searches the session's file; `progress` gets the bytes scanned so far, now and then, for a long scan.
    pub(super) async fn search(
        &self,
        session: u32,
        query: SearchQuery,
        progress: impl FnMut(u64) + Send + 'static,
    ) -> CommandResult<Option<SearchHit>> {
        check(&query)?;
        let handle = self.handle(session)?;
        let permit = begin(&self.searches, SEARCH_THREADS)?;
        let cancel = self.start_search(session)?;

        let scanning = Arc::clone(&cancel);
        let found = blocking(permit, move || scan(&handle.file, &query, &scanning, progress)).await;
        self.end_search(session, &cancel);

        match found? {
            Ok(hit) => Ok(hit),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(None),
            Err(error) => Err(view_error(&error)),
        }
    }

    /// Raises the flag of the session's search, if one runs.
    pub(super) fn cancel_search(&self, session: u32) -> CommandResult<()> {
        let state = self.lock();
        let found = state.sessions.iter().find(|found| found.id == session);

        if let Some(search) = found.ok_or_else(super::no_session)?.search.as_ref() {
            search.store(true, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Makes a new flag the session's, after stopping the search that held the old one.
    pub(super) fn start_search(&self, session: u32) -> CommandResult<Arc<AtomicBool>> {
        let mut state = self.lock();
        let found = state.sessions.iter_mut().find(|found| found.id == session);
        let found = found.ok_or_else(super::no_session)?;

        if let Some(earlier) = found.search.take() {
            earlier.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        found.search = Some(Arc::clone(&cancel));

        Ok(cancel)
    }

    /// Forgets the flag, unless a newer search has taken its place.
    pub(super) fn end_search(&self, session: u32, cancel: &Arc<AtomicBool>) {
        let mut state = self.lock();

        if let Some(found) = state.sessions.iter_mut().find(|found| found.id == session)
            && found
                .search
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, cancel))
        {
            found.search = None;
        }
    }
}

/// The needle is not empty and not longer than the viewer looks for, and a text needle comes with a known encoding.
fn check(query: &SearchQuery) -> CommandResult<()> {
    match &query.needle {
        SearchNeedle::Text { text, .. } => {
            known(&query.encoding)?;
            if text.is_empty() {
                return Err(invalid("the search text is empty"));
            }
            if text.chars().count() > MAX_NEEDLE {
                return Err(invalid("the search text is too long"));
            }
        }
        SearchNeedle::Hex { bytes } => {
            if bytes.is_empty() {
                return Err(invalid("the search text is empty"));
            }
            if bytes.len() > MAX_NEEDLE {
                return Err(invalid("the search text is too long"));
            }
        }
    }

    Ok(())
}

/// Looks for the next (or previous, backward) match of the needle from `query.from`: its offset and length, or `null`
/// when there is none or when the search was stopped by a newer search, `view_search_cancel` or closing the session.
/// `on_progress` gets the bytes scanned so far every 64 MiB.
#[tauri::command]
pub async fn view_search(
    viewer: State<'_, Viewer>,
    session: u32,
    query: SearchQuery,
    on_progress: Channel<u64>,
) -> CommandResult<Option<SearchHit>> {
    viewer
        .search(session, query, move |scanned| {
            // A page that is gone no longer needs the news.
            let _ = on_progress.send(scanned);
        })
        .await
}

/// Stops the session's search, if one runs (it answers `null`). A session that is not there is `notFound`.
#[tauri::command]
pub async fn view_search_cancel(viewer: State<'_, Viewer>, session: u32) -> CommandResult<()> {
    viewer.cancel_search(session)
}

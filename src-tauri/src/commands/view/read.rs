//! Reading an open file: raw bytes, windows of text, a Markdown document and an archive listing. Each runs on a blocking
//! thread of the read cap, after its arguments and the session are checked (see the module above).

use std::io;
use std::sync::atomic::AtomicBool;

use pairpane_core::model::{ArchiveListing, MdDocument, TextQuery, TextWindow, ViewKind};
use pairpane_core::view::{self as files, MAX_READ, ViewFile};
use tauri::State;
use tauri::ipc::Response;

use super::{Handle, READ_THREADS, Viewer, invalid, view_error};
use crate::commands::items::{begin, blocking};
use crate::error::{CommandError, CommandResult};

impl Viewer {
    /// Runs `work` on the file of `handle` on a blocking thread that holds a place of the read cap. `work` also gets the
    /// flag that is raised when the session ends.
    pub(super) async fn read<T: Send + 'static>(
        &self,
        handle: Handle,
        work: impl FnOnce(&ViewFile, &AtomicBool) -> io::Result<T> + Send + 'static,
    ) -> CommandResult<T> {
        let permit = begin(&self.reads, READ_THREADS)?;

        blocking(permit, move || work(&handle.file, &handle.cancel))
            .await?
            .map_err(|error| view_error(&error))
    }

    /// Up to `length` bytes of the session's file at `offset`; fewer at its end.
    pub(super) async fn bytes(&self, session: u32, offset: u64, length: u32) -> CommandResult<Vec<u8>> {
        let length = usize::try_from(length).map_err(|_| too_long())?;
        if length > MAX_READ {
            return Err(too_long());
        }
        let handle = self.handle(session)?;

        self.read(handle, move |file, _| file.read_at(offset, length)).await
    }

    pub(super) async fn text(&self, session: u32, query: TextQuery) -> CommandResult<TextWindow> {
        known(&query.encoding)?;
        let handle = self.handle(session)?;

        self.read(handle, move |file, _| files::text_window(file, &query)).await
    }

    pub(super) async fn markdown(&self, session: u32, encoding: String) -> CommandResult<MdDocument> {
        known(&encoding)?;
        let handle = self.handle(session)?;
        if !matches!(handle.kind, ViewKind::Markdown | ViewKind::Text) {
            return Err(invalid("the file is not text"));
        }

        self.read(handle, move |file, cancel| files::markdown(file, &encoding, cancel))
            .await
    }

    pub(super) async fn archive(&self, session: u32) -> CommandResult<ArchiveListing> {
        let handle = self.handle(session)?;
        let ViewKind::Archive { format } = handle.kind else {
            return Err(invalid("the file is not an archive"));
        };

        self.read(handle, move |file, cancel| files::archive(file, format, cancel))
            .await
    }
}

/// An encoding label the viewer knows (the answer of the page to what `view_open` found, or the page's own pick).
pub(super) fn known(label: &str) -> CommandResult<()> {
    if files::is_known_encoding(label) {
        return Ok(());
    }

    Err(invalid("the encoding is not known"))
}

fn too_long() -> CommandError {
    invalid("the read is too long")
}

/// Up to `length` bytes (at most 4 MiB) of the session's file from `offset`, as raw bytes; fewer at the end of the
/// file, none past it.
#[tauri::command]
pub async fn view_bytes(viewer: State<'_, Viewer>, session: u32, offset: u64, length: u32) -> CommandResult<Response> {
    viewer.bytes(session, offset, length).await.map(Response::new)
}

/// A window of lines of the session's file, from or up to `query.offset`, decoded as `query.encoding`.
#[tauri::command]
pub async fn view_text(viewer: State<'_, Viewer>, session: u32, query: TextQuery) -> CommandResult<TextWindow> {
    viewer.text(session, query).await
}

/// The session's Markdown (or text) file as a tree to draw. Nothing in it is HTML or a script.
#[tauri::command]
pub async fn view_markdown(viewer: State<'_, Viewer>, session: u32, encoding: String) -> CommandResult<MdDocument> {
    viewer.markdown(session, encoding).await
}

/// The entries of the session's archive (zip, tar or tar.gz), without extracting any of it.
#[tauri::command]
pub async fn view_archive(viewer: State<'_, Viewer>, session: u32) -> CommandResult<ArchiveListing> {
    viewer.archive(session).await
}

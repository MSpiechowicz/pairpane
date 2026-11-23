//! Reading an open file: raw bytes, windows of text, a Markdown document, an archive listing and a decoded HEIC
//! picture. Each runs on a blocking thread of its cap (the read cap, the decode cap for the picture), after its
//! arguments and the session are checked (see the module above).

use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use pairpane_core::model::{ArchiveListing, MdDocument, TextQuery, TextWindow, ViewKind};
use pairpane_core::view::{self as files, MAX_READ, ViewFile};
use tauri::State;
use tauri::ipc::Response;

use super::{DECODE_THREADS, Handle, READ_THREADS, Viewer, invalid, view_error};
use crate::commands::items::{begin, blocking};
use crate::commands::limit::ThreadCap;
use crate::error::{CommandError, CommandResult};

impl Viewer {
    /// Runs `work` on the file of `handle` on a blocking thread that holds a place of the read cap. `work` also gets the
    /// flag that is raised when the session ends.
    pub(super) async fn read<T: Send + 'static>(
        &self,
        handle: Handle,
        work: impl FnOnce(&ViewFile, &AtomicBool) -> io::Result<T> + Send + 'static,
    ) -> CommandResult<T> {
        run(&self.reads, READ_THREADS, handle, work).await
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

    /// The session's HEIC picture as the 8-byte header (width and height, `u32` little endian) and then the RGBA
    /// pixels, decoded by the system's codecs in a child process: the executable `program` (the app itself), started
    /// as the decoding helper. What the session holds is checked before a thread is taken, and the decode has a
    /// place of its own, so a decode that runs does not take a read's place and a second one is refused as busy.
    pub(super) async fn image_decode(&self, session: u32, program: PathBuf) -> CommandResult<Vec<u8>> {
        let handle = self.handle(session)?;
        let ViewKind::Image { mime } = &handle.kind else {
            return Err(not_heic());
        };
        if mime != "image/heic" {
            return Err(not_heic());
        }

        run(&self.decodes, DECODE_THREADS, handle, move |file, cancel| {
            files::decode_heic(file, cancel, &program)
        })
        .await
    }
}

/// Runs `work` on the file of `handle` on a blocking thread that holds a place of `cap` (of at most `max`).
pub(super) async fn run<T: Send + 'static>(
    cap: &ThreadCap,
    max: usize,
    handle: Handle,
    work: impl FnOnce(&ViewFile, &AtomicBool) -> io::Result<T> + Send + 'static,
) -> CommandResult<T> {
    let permit = begin(cap, max)?;

    blocking(permit, move || work(&handle.file, &handle.cancel))
        .await?
        .map_err(|error| view_error(&error))
}

fn not_heic() -> CommandError {
    invalid("the file is not a heic image")
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

/// The executable that is started as the HEIC helper and as the thumbnail helper: the app itself. On Linux that is `/proc/self/exe`, which names
/// the running program itself and not the path it was started from, so a package upgrade that replaces the file at that
/// path (or a swap of it) cannot make the helper another program; elsewhere the path the system reports.
pub(in crate::commands) fn helper_program() -> io::Result<PathBuf> {
    if cfg!(target_os = "linux") {
        return Ok(PathBuf::from("/proc/self/exe"));
    }

    std::env::current_exe()
}

/// The session's HEIC picture decoded by the system's codecs, in a helper process: 8 bytes (width and height as `u32`, little endian), then
/// `width * height * 4` bytes of RGBA, not premultiplied, rows tightly packed. Fails as `unsupported` when the system
/// has no HEIC decoder and as `invalid_input` when the session's file is not a HEIC image or the picture is too big.
#[tauri::command]
pub async fn view_image_decode(viewer: State<'_, Viewer>, session: u32) -> CommandResult<Response> {
    // The helper is the app itself. Not being able to tell where it is is not a missing file.
    let program = helper_program().map_err(|error| view_error(&io::Error::other(error)))?;

    viewer.image_decode(session, program).await.map(Response::new)
}

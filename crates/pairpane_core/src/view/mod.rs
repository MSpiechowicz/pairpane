//! The file viewer: opening a regular file read-only and answering questions about it one window at a time, so a
//! file of any size opens at once and nothing is ever read whole (except a Markdown file, up to [`MAX_MARKDOWN`]).
//!
//! # API
//!
//! - [`open`] opens `name` inside a folder without following a link and says what the file is ([`Opened`]: the
//!   [`ViewFile`] to keep, its size, time, [`ViewKind`] and detected text encoding). The shell wraps that in a
//!   session and shares the [`ViewFile`] (it is `Send + Sync`) between the threads that read it.
//! - [`ViewFile::read_at`] and [`ViewFile::size`]: raw bytes at an offset (the hex view, images, PDF, media).
//! - [`text_window`]: lines of a text file from or up to a byte offset, in any encoding.
//! - [`search`]: the next or previous match of a text or byte needle, scanned in chunks, cancellable.
//! - [`markdown`]: a Markdown file as a tree of plain data; [`archive`]: the entries of a zip, tar (plain or compressed)
//!   or 7z, as [`crate::archive`] reads them. Both take
//!   a cancel flag (raised when the session ends) and then fail with `Interrupted`.
//! - [`decode_heic`]: a HEIC picture as RGBA pixels, decoded by the codecs the system has (libheif on Linux, the Windows
//!   Imaging Component on Windows) and never by a codec of the app; it fails with `Unsupported` when there is none. The
//!   decoding runs in a short-lived child process (the app itself, started with [`HEIC_CHILD_ARG`] and handed to
//!   [`heic_child_main`]), so a crash of a system codec on a hostile file cannot take the app down.
//! - [`range`]: the pure parsing of a `Range: bytes=` header for the media scheme.
//!
//! # Rules
//!
//! The file is read through its handle at offsets (no memory map, no seeking shared state), so the one open file can
//! serve many readers at once and a rename or delete after the open changes nothing. Every size is capped by a
//! constant below, whatever the file or the query holds. Nothing here panics on bad input and nothing extracts or
//! executes anything. Error texts are fixed lowercase English and never hold a path; the errors that carry a text of
//! ours are marked ([`refusal`], [`is_refusal`]), so a caller can show those and nothing else.

mod archive;
mod encoding;
mod file;
mod heic;
mod layout;
mod markdown;
mod md_build;
mod md_frame;
mod open;
pub mod range;
mod search;
mod sniff;
mod text;

#[cfg(test)]
mod archive_limit_tests;
#[cfg(test)]
mod archive_tests;
#[cfg(test)]
mod encoding_tests;
#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod heic_tests;
#[cfg(test)]
mod markdown_tests;
#[cfg(test)]
mod open_tests;
#[cfg(test)]
mod range_tests;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod sniff_tests;
#[cfg(test)]
mod tar_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod text_tests;
#[cfg(test)]
mod zip_tests;

use std::io;

pub use archive::archive;
pub use encoding::is_known_encoding;
pub(crate) use encoding::{bom_len, by_name, detect, is_utf16};
pub use file::ViewFile;
pub use heic::{
    CHILD_ARG as HEIC_CHILD_ARG, child_main as heic_child_main, decode as decode_heic,
    helper_command as heic_helper_command,
};
pub use markdown::markdown;
pub use open::{Opened, open};
pub(crate) use open::{Refusals, open_regular};
pub use search::search;
pub use text::text_window;

/// The most bytes one [`ViewFile::read_at`] returns.
pub const MAX_READ: usize = 4 * 1024 * 1024;

/// The most lines one [`text_window`] returns (a piece of a long line counts as a line).
pub const MAX_TEXT_LINES: usize = 1000;

/// The most bytes of one line that come back as one entry; a longer line comes back in pieces.
pub const MAX_SEGMENT: usize = 16 * 1024;

/// How far [`text_window`] reads for one window, at most.
pub const MAX_TEXT_SCAN: usize = 1024 * 1024;

/// How far back [`text_window`] looks for the start of the line an offset falls in.
pub const MAX_SNAP: usize = 64 * 1024;

/// The most bytes of a Markdown file that are parsed; the rest is dropped and the document says it is truncated.
pub const MAX_MARKDOWN: usize = 8 * 1024 * 1024;

/// The most bytes of a HEIC file [`decode_heic`] reads (the same cap as the page's image view).
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// The most pixels (width times height) of a picture [`decode_heic`] decodes: 256 MiB of RGBA at most.
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;

/// The longest side of a picture [`decode_heic`] decodes.
pub const MAX_IMAGE_SIDE: u32 = 32_767;

/// The most entries [`archive`] lists.
pub const MAX_ARCHIVE_ENTRIES: usize = 20_000;

/// The most bytes of a `.tar.gz` that are inflated to list it.
pub const MAX_GZ_SCAN: u64 = 2 * 1024 * 1024 * 1024;

/// The bytes one step of a search reads (and scans) before it looks at the cancel flag.
pub const SEARCH_CHUNK: usize = 1024 * 1024;

/// The longest needle of a search: bytes of a hex needle, characters of a text needle.
pub const MAX_NEEDLE: usize = 256;

/// How much of the start of a file decides its kind and its encoding.
pub const SNIFF_BYTES: usize = 64 * 1024;

/// The error of a read that was stopped because its session was closed. It is `Interrupted` (so a caller can tell it
/// from the others) and is never retried.
fn cancelled() -> io::Error {
    refusal(io::ErrorKind::Interrupted, "the read was cancelled")
}

/// The payload of an error with a text of our own (see [`refusal`]); what tells it from an error of the system or of a
/// library, whose text must never reach the page.
#[derive(Debug)]
struct Refusal(&'static str);

impl std::fmt::Display for Refusal {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(self.0)
    }
}

impl std::error::Error for Refusal {}

/// An error with a text of its own, for a rule that no system call reports (the text never holds a path). Its
/// `to_string` is `text`, and [`is_refusal`] knows it.
pub fn refusal(kind: io::ErrorKind, text: &'static str) -> io::Error {
    io::Error::new(kind, Refusal(text))
}

/// Whether `error` was made by [`refusal`], so its text is ours and holds no path; any other error (a system call, a
/// library, an error that someone made with a text of their own) is not.
pub fn is_refusal(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Refusal>())
}

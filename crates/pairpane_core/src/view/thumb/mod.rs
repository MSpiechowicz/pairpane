//! Thumbnails: a small preview of a picture file, made without ever decoding an untrusted picture in the app's process.
//!
//! # Trust model
//!
//! Every byte of a picture is untrusted and every format is parsed by a decoder that has had bugs, so no decoder runs in
//! the app. [`read`] reads the file and [`make`] hands the bytes to a short-lived child process, the app's own executable started
//! with [`CHILD_ARG`] and the side of the thumbnail, whose first act is to call [`child_main`]. The child is the HEIC
//! helper's kind of process and shares its machinery (`heic/process.rs`: the framing of the answer, the deadline, the
//! cancel flag, the reaping; `heic/sandbox.rs`, `macos.rs` and `heic/job.rs`: before it reads a byte it closes what it
//! inherited, limits itself, gives up the file system and the network and, on Linux, may make only the dozen system calls
//! it needs; on macOS it enters a sandbox that denies everything; on Windows the parent starts it in an AppContainer, in a Job Object). A crash, a hang or a runaway answer costs that one process
//! and is reported as a picture that cannot be decoded. The parent never trusts the answer: it is read as it is framed,
//! and a picture with a side over the one asked for is refused.
//!
//! - PNG, JPEG, GIF (the first frame), WebP (the first frame) and BMP are decoded by the pure Rust decoders of the
//!   `image` crate in that child (`child.rs`), under limits on the sides and the allocation, so a picture that claims
//!   100000 by 100000 pixels is `too big` and nothing is allocated for it.
//! - HEIC goes through the HEIC helper (`heic::decode`, the system's codec in a confined process of its own). The
//!   parent shrinks the RGBA pixels it gets back, in place, which is a resize of raw pixels and parses nothing. Without a
//!   decoder on the system the thumbnail is `Unsupported`, with the HEIC helper's text. On Windows there is no HEIC
//!   thumbnail at all (`Unsupported`): its codec runs with the user's whole token there, and a thumbnail is made
//!   without a click for every picture of a folder; the viewer decodes the HEIC picture the user opens.
//! - The work is in two steps so that a caller can bound them apart: [`read`] reads the file (which can hang on a mount
//!   that does not answer) and [`make`] runs the helper on the bytes.
//!
//! The format is chosen from the first bytes of the file, never from its extension ([`Format::detect`]), and a file that
//! is none of these is `Unsupported` at once, without a helper being started. A file over `MAX_IMAGE_BYTES` is `too big`
//! and is not read.
//!
//! The answer is the one buffer of the HEIC helper: an 8-byte header (width and height as `u32`, little endian), then
//! `width * height * 4` bytes of RGBA, not premultiplied, rows tightly packed; the picture fits a square of the side
//! asked for with its aspect ratio kept, and is never made bigger than it is. The errors have the fixed texts of the
//! viewer's (`refusal`, `is_refusal`), with no path in them.

#[cfg(windows)]
mod appcontainer;
mod child;
mod format;
mod identity;
#[cfg(target_os = "macos")]
mod macos;

use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

pub use child::child_main;
pub use format::{Format, MAX_SIZE, MIN_SIZE, bucket, clamp_size, fit};
pub use identity::Identity;

use super::file::ViewFile;
use super::heic::{self, process};
use super::{Refusals, open_regular, refusal};

/// The first argument (a second one holds the side of the thumbnail) with which the app's executable is started as the
/// thumbnail helper. The shell checks for it before anything else starts and passes the process to [`child_main`].
pub const CHILD_ARG: &str = "--pairpane-thumbnail";

/// How long the child may take, from its start to its end. The biggest picture shrinks in a few seconds; a child that
/// is still at it after this is hung, and is killed.
const DEADLINE: Duration = Duration::from_secs(20);

/// The committed memory the thumbnail helper may have on Windows (the Job Object's limit), in bytes: the input (64 MiB),
/// the decoded picture (at most 256 MiB) and what the decoder keeps beside it (see `child.rs`); 1.5 GiB is more than
/// that and half of the HEIC helper's. The same room as the address space it is given on Linux.
#[cfg(windows)]
const MEMORY: usize = 1536 * 1024 * 1024;

/// What is said when a name is not a regular file.
const REFUSALS: Refusals = Refusals {
    link: "a thumbnail is made of a regular file, not a link",
    folder: "a thumbnail is made of a file, not a folder",
    other: "a thumbnail is made of a regular file only",
};

/// A picture file opened to make a thumbnail of it: the open file (no link was followed to reach it), what it is and
/// what a cache needs to tell it from another or from itself after a change.
#[derive(Debug)]
pub struct Source {
    /// The size of the file when it was opened, in bytes.
    pub len: u64,
    /// Its last modification, when the system tells.
    pub modified: Option<SystemTime>,
    /// Which file it is (see [`Identity`]), when the system tells: a file that was replaced by another of the same
    /// length and time is another picture.
    pub identity: Option<Identity>,
    file: ViewFile,
    format: Format,
}

impl Source {
    /// Whether the picture is HEIC, which is decoded by a codec of the system and may need a lot of memory.
    pub fn is_heic(&self) -> bool {
        self.format == Format::Heic
    }
}

/// Opens the regular file `name` of `folder` and says what it is by its first bytes. A link, a folder and anything that
/// is not a regular file are refused (`InvalidInput`), and a file that is not a picture of the supported formats is
/// `Unsupported`, both with fixed texts; no helper is started.
pub fn open(folder: &Path, name: &str) -> io::Result<Source> {
    let (file, meta) = open_regular(folder, name, &REFUSALS)?;
    let identity = Identity::of(&file, &meta);
    let file = ViewFile::new(file);

    let len = meta.len();
    let head = file.read_at(
        0,
        usize::try_from(len).map_or(format::HEAD, |len| len.min(format::HEAD)),
    )?;
    let format = Format::detect(&head)
        .filter(|&format| has_thumbnail(format, cfg!(windows)))
        .ok_or_else(not_a_picture)?;

    Ok(Source {
        len,
        modified: meta.modified().ok(),
        identity,
        file,
        format,
    })
}

/// Whether a file of `format` gets a thumbnail on the system (`windows` or not). HEIC does not on Windows: its codec
/// there is the Imaging Component, which runs with the user's whole token, and a thumbnail is made without a click for
/// every picture of a folder that is shown; the viewer decodes a HEIC picture the user opens, and that is all.
fn has_thumbnail(format: Format, windows: bool) -> bool {
    !(windows && format == Format::Heic)
}

/// The bytes of the picture `source`, read whole under the caps, for [`make`]. This is the half of the work that touches
/// the file, and so the half that can hang on a file that is on a mount that does not answer; a caller that limits how
/// many helpers run takes its place for [`make`] after this, not before. A file over `MAX_IMAGE_BYTES` is refused
/// (`InvalidInput`) without being read; `cancel` is looked at between the pieces (`Interrupted`). For HEIC the bytes are
/// also checked to be HEVC-coded HEIF, and on a system without a decoder this is `Unsupported` at once.
pub fn read(source: &Source, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
    if source.format == Format::Heic {
        return heic::read_checked(&source.file, cancel);
    }

    heic::read_whole(&source.file, cancel)
}

/// The thumbnail of the picture `source`, whose bytes are `bytes` (from [`read`]), for a square of `size` (brought into
/// [`MIN_SIZE`]..=[`MAX_SIZE`]) as the header and the RGBA pixels (see the module), made by the helper `program` (the
/// app itself).
///
/// Fails with `Unsupported` when the file is not a picture of the formats (and for HEIC, when the system has no
/// decoder), `InvalidInput` when the picture is over the caps, `InvalidData` when it cannot be decoded (the helper
/// crashing or answering wrongly included) and `Interrupted` when `cancel` is raised (the helper is then stopped). An
/// error of the system (the helper could not be started) is not one of ours and holds no text for a page.
pub fn make(source: &Source, bytes: Vec<u8>, size: u32, cancel: &AtomicBool, program: &Path) -> io::Result<Vec<u8>> {
    let size = clamp_size(size);

    if source.format == Format::Heic {
        let picture = heic::run_checked(bytes, cancel, program, &[heic::CHILD_ARG])?;

        return format::shrink(picture, size);
    }

    let side = size.to_string();

    run_helper(program, &[CHILD_ARG, &side], bytes, cancel, size).map_err(|error| {
        // The helper's "no decoder" is the HEIC helper's text, which is not what is wrong with this file.
        if error.kind() == io::ErrorKind::Unsupported {
            return not_a_picture();
        }

        error
    })
}

/// The helper `program` with `args` on `bytes`: a process of its own with the sandbox of the system (see the module).
#[cfg(not(windows))]
fn run_helper(program: &Path, args: &[&str], bytes: Vec<u8>, cancel: &AtomicBool, size: u32) -> io::Result<Vec<u8>> {
    process::run_within(program, args, bytes, cancel, DEADLINE, size)
}

/// The helper `program` with `args` on `bytes`: a process in an AppContainer and a job (see the module).
#[cfg(windows)]
fn run_helper(program: &Path, args: &[&str], bytes: Vec<u8>, cancel: &AtomicBool, size: u32) -> io::Result<Vec<u8>> {
    appcontainer::run(program, args, bytes, cancel, DEADLINE, size, MEMORY)
}

/// The command that starts the thumbnail helper `program` with `args` (the pipes, the cleared environment): for the tests
/// of the app's executable as the helper, which start it themselves to look at it while it waits.
#[doc(hidden)]
#[must_use]
pub fn helper_command(program: &Path, args: &[&str]) -> std::process::Command {
    heic::helper_command(program, args)
}

/// The error for a file that is not a picture a thumbnail can be made of.
fn not_a_picture() -> io::Error {
    refusal(
        io::ErrorKind::Unsupported,
        "the file is not a picture that has a thumbnail",
    )
}

#[cfg(all(test, windows))]
mod appcontainer_tests;
#[cfg(test)]
mod child_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod format_tests;
#[cfg(all(test, target_os = "macos"))]
mod macos_tests;
#[cfg(test)]
mod make_tests;

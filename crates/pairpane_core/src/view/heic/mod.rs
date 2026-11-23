//! HEIC pictures decoded by the codecs the system has, never by a codec the app ships (HEVC is patent-encumbered, and
//! the system's decoder is the one the user already has the right to use).
//!
//! - Linux and the BSDs: the system's libheif, opened at run time (`libheif.rs`). Without it, or without an HEVC
//!   plugin, [`decode`] fails with `Unsupported`.
//! - Windows: the Windows Imaging Component with the user's HEIF and HEVC Image Extensions (`wic.rs`).
//! - Elsewhere (macOS): `Unsupported` at once, nothing is read or started; the webview shows a HEIC itself there.
//!
//! The system's codecs parse files from anywhere, so they never run in the app's process: [`decode`] reads the file,
//! checks that it is HEVC-coded HEIF (the `ftyp` brands, and every item the file lists, `items.rs`) and hands the bytes
//! to a short-lived child process, the app's own executable started with [`CHILD_ARG`], whose first act is to call
//! [`child_main`] (`process.rs`). A crash, a hang or a runaway answer of the codec costs that one process (a hang at
//! most [`process::DEADLINE`]) and is reported as a file that cannot be decoded. The child is also confined: on Linux
//! it closes what it inherited, limits itself, gives up the file system and the network once the codecs are loaded and
//! refuses the calls that make a socket or start a program (`sandbox.rs`), on Windows
//! the parent puts it in a Job Object that bounds its memory and its processes (`job.rs`). The helper is started with a
//! cleared environment but for what the loader needs.
//!
//! The answer is one buffer: an 8-byte header (width and height as `u32`, little endian) and then `width * height * 4`
//! bytes of RGBA, not premultiplied, rows tightly packed. This module holds what does not touch a system library and
//! so is safe and tested on every system: reading the file whole under a cap, checking the brands and a size, copying
//! rows out of a padded plane and the fixed error texts. The two decoders are the only `unsafe` code.
#![cfg_attr(
    not(any(
        windows,
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    )),
    allow(
        dead_code,
        reason = "the helpers serve the decoders, which exist on Linux, the BSDs and Windows only"
    )
)]

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub(super) mod libheif;
#[cfg(windows)]
pub(super) mod wic;

mod items;
// The thumbnail helper (`view::thumb`) is another child of the same kind and shares the process and its confinement.
pub(super) mod process;

#[cfg(windows)]
mod job;
#[cfg(target_os = "linux")]
mod kernel;
#[cfg(target_os = "linux")]
pub(super) mod sandbox;

pub use process::child_main;

/// The command [`decode`] starts its helper with (the pipes, the cleared environment) for `program` and `args`: for the
/// tests of the app's executable as the helper, which start it themselves to look at it while it waits.
#[doc(hidden)]
#[must_use]
pub fn helper_command(program: &Path, args: &[&str]) -> std::process::Command {
    process::command(program, args)
}

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, MAX_READ, ViewFile, cancelled, refusal};

/// The first argument with which the app's executable is started as the decoding helper. The shell checks for it before
/// anything else starts and passes the process to [`child_main`].
pub const CHILD_ARG: &str = "--pairpane-decode-heic";

/// The bytes of the header: width and height, `u32` little endian each.
pub(super) const HEADER: usize = 8;

/// What the page is told when there is no decoder; the way to get one depends on the system.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
const NO_DECODER: &str = "no heic decoder on this system: install libheif with an hevc plugin";
#[cfg(windows)]
const NO_DECODER: &str = "no heic decoder on this system: install the heif and hevc image extensions";
#[cfg(not(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
const NO_DECODER: &str = "no heic decoder on this system";

/// The picture of the HEIC file as the header and the RGBA pixels (see the module), decoded in a child process: the
/// executable `program` (the app itself) started with [`CHILD_ARG`].
///
/// Fails with `Unsupported` when the system has no decoder (at once, on a system that never has one), `InvalidInput`
/// when the file is not HEVC-coded HEIF (a file that lists an item of any other codec is not) or the file or the picture is over the caps ([`MAX_IMAGE_BYTES`],
/// [`MAX_IMAGE_PIXELS`], [`MAX_IMAGE_SIDE`]), `InvalidData` when the file cannot be decoded (the helper crashed or gave
/// no valid answer too) and `Interrupted` when `cancel` is raised (the helper is then stopped). An error of the system
/// (the helper could not be started) is not one of ours and holds no text for the page.
pub fn decode(file: &ViewFile, cancel: &AtomicBool, program: &Path) -> io::Result<Vec<u8>> {
    decode_isolated(file, cancel, program, &[CHILD_ARG])
}

/// [`decode`] with the helper and its arguments given: a helper that is not the app is how the tests stand in for one.
pub(super) fn decode_isolated(
    file: &ViewFile,
    cancel: &AtomicBool,
    program: &Path,
    args: &[&str],
) -> io::Result<Vec<u8>> {
    let bytes = read_checked(file, cancel)?;

    run_checked(bytes, cancel, program, args)
}

/// The first half of [`decode`]: the whole file, read under the caps, checked to be HEVC-coded HEIF (the brands and every
/// item it lists). Nothing is started and nothing of the file is parsed by a codec. A caller that has to take a place
/// for the helper (the thumbnails) reads first and takes it for [`run_checked`] only. On a system without a decoder this
/// fails with `Unsupported` at once.
pub(super) fn read_checked(file: &ViewFile, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
    #[cfg(any(
        windows,
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    ))]
    {
        let bytes = read_whole(file, cancel)?;
        if !is_hevc_heif(&bytes) || !items::only_hevc_items(&bytes) {
            return Err(not_heic());
        }

        Ok(bytes)
    }

    #[cfg(not(any(
        windows,
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    )))]
    {
        let _ = (file, cancel);

        Err(no_decoder())
    }
}

/// The second half of [`decode`]: the file `bytes` that [`read_checked`] gave, decoded by the helper `program` started
/// with `args`.
pub(super) fn run_checked(bytes: Vec<u8>, cancel: &AtomicBool, program: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    #[cfg(any(
        windows,
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    ))]
    {
        process::run(program, args, bytes, cancel, process::DEADLINE)
    }

    #[cfg(not(any(
        windows,
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    )))]
    {
        let _ = (bytes, cancel, program, args);

        Err(no_decoder())
    }
}

/// Whether `bytes` start with an `ftyp` box that names an HEVC brand (`heic`, `heix`, `heim`, `heis`, `hevc` or `hevx`)
/// as the major brand or among the compatible ones. HEIF holds other codecs too (AVIF, JPEG, uncompressed) that the
/// system's decoder would also take; only HEVC is what this is for. A box that is short, truncated, has a size of 0 or
/// 1 (to the end of the file, a 64-bit size) or is not `ftyp` is refused; at most the first 1 KiB of the box is read.
pub(super) fn is_hevc_heif(bytes: &[u8]) -> bool {
    const HEVC: [[u8; 4]; 6] = [*b"heic", *b"heix", *b"heim", *b"heis", *b"hevc", *b"hevx"];
    const MAX_BOX: usize = 1024;

    let Some(head) = bytes.get(..16) else {
        return false;
    };
    if &head[4..8] != b"ftyp" {
        return false;
    }

    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if size < 16 || size > bytes.len() {
        return false;
    }

    // Major brand, then the compatible brands after the minor version (the 4 bytes between).
    let major = [head[8], head[9], head[10], head[11]];
    let (compatible, _) = bytes[16..size.min(MAX_BOX)].as_chunks::<4>();

    HEVC.contains(&major) || compatible.iter().any(|brand| HEVC.contains(brand))
}

/// The error for a file that is not HEVC-coded HEIF.
fn not_heic() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the file is not a heic image")
}

/// The error for a system without a decoder (the text is fixed and tells how to get one).
pub(super) fn no_decoder() -> io::Error {
    refusal(io::ErrorKind::Unsupported, NO_DECODER)
}

/// The error for a file the decoder cannot read.
pub(super) fn undecodable() -> io::Error {
    refusal(io::ErrorKind::InvalidData, "the image cannot be decoded")
}

/// The error for a file or a picture over a cap.
pub(super) fn too_big() -> io::Error {
    refusal(io::ErrorKind::InvalidInput, "the image is too big to show here")
}

/// The whole file, read in pieces of at most [`MAX_READ`], with the cancel flag looked at before each. A file over
/// [`MAX_IMAGE_BYTES`] is refused before anything is read. A file that shrinks while it is read gives what is left.
pub(super) fn read_whole(file: &ViewFile, cancel: &AtomicBool) -> io::Result<Vec<u8>> {
    let size = file.size()?;
    if size > MAX_IMAGE_BYTES {
        return Err(too_big());
    }

    let size = usize::try_from(size).map_err(|_| too_big())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| too_big())?;

    while bytes.len() < size {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        let want = (size - bytes.len()).min(MAX_READ);
        let piece = file.read_at(bytes.len() as u64, want)?;
        if piece.is_empty() {
            break;
        }

        bytes.extend_from_slice(&piece);
    }

    Ok(bytes)
}

/// The size of a picture as `u32`s, or an error when a side is not positive, is over [`MAX_IMAGE_SIDE`] or the pixels
/// are over [`MAX_IMAGE_PIXELS`]. Takes what the decoders report (signed, as the system gives it) and never overflows.
pub(super) fn check_size(width: i64, height: i64) -> io::Result<(u32, u32)> {
    if width <= 0 || height <= 0 {
        return Err(undecodable());
    }

    let side = i64::from(MAX_IMAGE_SIDE);
    if width > side || height > side {
        return Err(too_big());
    }

    // Both are in 1..=32767 now, so the product fits `i64` and the casts below cannot lose anything.
    if (width * height) as u64 > MAX_IMAGE_PIXELS {
        return Err(too_big());
    }

    Ok((width as u32, height as u32))
}

/// The output buffer of a picture of this size (checked by [`check_size`]): the header and zeroed pixels.
pub(super) fn output(width: u32, height: u32) -> io::Result<Vec<u8>> {
    let pixels = usize::try_from(u64::from(width) * u64::from(height) * 4).map_err(|_| too_big())?;
    let total = HEADER.checked_add(pixels).ok_or_else(too_big)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| too_big())?;

    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.resize(total, 0);

    Ok(out)
}

/// Copies `rows` rows of `row` bytes each from `src`, whose rows start `stride` bytes apart (the last row needs only
/// `row` bytes), into `out`, which holds exactly `rows * row` bytes. Never panics: a `stride` under `row`, a `src` that
/// is too short or an `out` of another length is an error.
#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))),
    test
))]
pub(super) fn copy_rows(out: &mut [u8], src: &[u8], row: usize, rows: usize, stride: usize) -> io::Result<()> {
    if rows == 0 || row == 0 {
        return if out.is_empty() { Ok(()) } else { Err(undecodable()) };
    }

    let wanted = row.checked_mul(rows).ok_or_else(undecodable)?;
    if out.len() != wanted || stride < row {
        return Err(undecodable());
    }

    let needed = (rows - 1)
        .checked_mul(stride)
        .and_then(|before| before.checked_add(row))
        .ok_or_else(undecodable)?;
    if src.len() < needed {
        return Err(undecodable());
    }

    for (at, line) in out.chunks_exact_mut(row).enumerate() {
        let start = at * stride;
        line.copy_from_slice(&src[start..start + row]);
    }

    Ok(())
}

/// The error of a libheif result: `no_decoder` for "unsupported codec", "plugin loading error" and "no matching decoder
/// installed" (libheif is there but cannot decode HEVC), `undecodable` for anything else. A "decoder plugin error" is
/// the decoder failing on the data, so it is a bad file.
#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))),
    test
))]
pub(super) fn map_heif(code: i32, subcode: i32) -> io::Error {
    const UNSUPPORTED_FEATURE: i32 = 4;
    const PLUGIN_LOADING_ERROR: i32 = 11;
    const UNSUPPORTED_CODEC: i32 = 3000;
    const NO_MATCHING_DECODER: i32 = 6003;

    match (code, subcode) {
        (UNSUPPORTED_FEATURE, UNSUPPORTED_CODEC) | (PLUGIN_LOADING_ERROR, _) | (_, NO_MATCHING_DECODER) => no_decoder(),
        _ => undecodable(),
    }
}

/// The error of a Windows Imaging Component result (an `HRESULT`): `no_decoder` when the component or the codec of the
/// format is not installed, `undecodable` for anything else.
///
/// The two codes are those the documentation gives for a missing HEIF codec and a missing HEVC decoder. Whether they
/// are the complete set (and what the component reports with the HEIF extension installed but no HEVC extension) has
/// still to be confirmed on a Windows machine with HEIF and without HEVC; a code outside the set is `undecodable`.
#[cfg(any(windows, test))]
pub(super) fn map_hresult(hresult: i32) -> io::Error {
    /// `WINCODEC_ERR_COMPONENTNOTFOUND`: no codec for the format (the HEIF extension is not installed).
    const COMPONENT_NOT_FOUND: i32 = 0x8898_2F50_u32 as i32;
    /// `MF_E_TOPO_CODEC_NOT_FOUND`: the HEIF codec is there but there is no HEVC decoder.
    const TOPO_CODEC_NOT_FOUND: i32 = 0xC00D_5212_u32 as i32;

    match hresult {
        COMPONENT_NOT_FOUND | TOPO_CODEC_NOT_FOUND => no_decoder(),
        _ => undecodable(),
    }
}

#[cfg(test)]
mod items_tests;
#[cfg(test)]
mod process_tests;
#[cfg(all(test, target_os = "linux"))]
mod sandbox_tests;

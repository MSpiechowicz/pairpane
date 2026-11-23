//! The thumbnail helper's own end: the child process that decodes an untrusted picture.
//!
//! The child is the app's own executable started with [`CHILD_ARG`](super::CHILD_ARG) and the side of the thumbnail
//! (`--pairpane-thumbnail 128`); the shell hands it to [`child_main`] before anything else starts. It confines itself
//! before it reads a byte (there is no codec to load first):
//!
//! - Linux (`heic/sandbox.rs`): it closes the descriptors it inherited, limits itself, gives up the file system and the
//!   network (Landlock) and takes an allow-list seccomp filter, which lets through the dozen calls it still makes and
//!   refuses the rest;
//! - macOS (`macos.rs`): the same descriptors and limits, and the Seatbelt sandbox with every operation denied;
//! - Windows: the parent puts it in a Job Object that bounds its memory and its processes (`heic/job.rs`).
//!
//! It reads the file from its standard input (at most `MAX_IMAGE_BYTES`), decodes it with the pure Rust decoders of the
//! `image` crate, shrinks it and writes the answer in the framing of the HEIC helper (`heic/process.rs`): a status byte,
//! then the 8-byte header and the RGBA pixels of a picture that fits the side.
//!
//! A picture that claims more than it may is refused before a byte of it is decoded: the decoder reports its size from
//! the header, and a side over `MAX_IMAGE_SIDE`, more pixels than `MAX_IMAGE_PIXELS` or a decoded picture of more than
//! [`MAX_DECODED_BYTES`] is `too big`. What the decoders do beside that is bound by what each one honours of
//! `image::Limits` (see [`limits`]): every one is held to the sides; PNG also to the allocation limit; JPEG, WebP and
//! BMP to the sides only (their buffers are the decoded picture, which the check above bounds, and what the JPEG decoder
//! keeps beside it is a few planes of it); the `gif` crate has a limit of its own (50 MB for a frame's buffers). The
//! address space of the process is limited on Linux (and the memory of the job on Windows), so a decoder that wants more
//! than it should fails or ends the process, which the parent counts as a picture that cannot be decoded; it never takes
//! the app's memory. A GIF and a WebP give their first frame only, and the EXIF orientation of a picture is applied.

use std::io::{self, Cursor, Read, Write};

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageError, ImageReader, Limits};

use crate::view::MAX_IMAGE_SIDE;
use crate::view::heic::process::serve;
use crate::view::heic::{check_size, too_big, undecodable};

use super::format::{Format, clamp_size, fit, frame};
use super::not_a_picture;

/// The most bytes the decoded picture may have before it is shrunk: 64 megapixels of RGBA, or half that with 16 bits a
/// channel. What the decoder allocates beside it is bound by [`limits`] and by the address space of the process.
pub(super) const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

/// The most the decoders may allocate at any one time, output included. Only PNG keeps to it (see the module); the
/// address space of the process is the bound for the rest.
const MAX_ALLOC: u64 = 512 * 1024 * 1024;

/// The entry of the child: reads the side of the thumbnail from its arguments, confines itself (on Linux and macOS) and
/// then serves one file from the standard input to the standard output. Returns the exit code of the process (0 when an
/// answer was written, which a failed decoding is too).
pub fn child_main() -> i32 {
    // The side comes from the app that started this process; a missing or garbled one is no answer to give.
    let size = std::env::args_os()
        .nth(2)
        .and_then(|arg| arg.to_str()?.parse::<u32>().ok())
        .map(clamp_size);
    let Some(size) = size else {
        return 1;
    };

    // The streams are taken before the confinement, which only closes what is not these.
    let (input, output) = (io::stdin().lock(), io::stdout().lock());

    #[cfg(target_os = "linux")]
    if let Err(error) = confine() {
        tracing::debug!(%error, "the thumbnail helper cannot confine itself");

        return 1;
    }

    #[cfg(target_os = "macos")]
    if let Err(error) = super::macos::confine() {
        tracing::debug!(%error, "the thumbnail helper cannot confine itself");

        return 1;
    }

    run_child(input, output, size)
}

/// The three steps of `heic/sandbox.rs`, in order, before the first byte is read.
#[cfg(target_os = "linux")]
fn confine() -> io::Result<()> {
    use crate::view::heic::sandbox::{self, Filter};

    sandbox::close_inherited_fds();
    sandbox::limit()?;
    sandbox::single_threaded()?;

    sandbox::confine(sandbox::THUMBNAIL_ADDRESS_SPACE, Filter::AllowList)
}

/// [`child_main`] over any input and output.
pub(super) fn run_child(input: impl Read, output: impl Write, size: u32) -> i32 {
    serve(input, output, |bytes| thumbnail(bytes, size))
}

/// The limits the decoders run under: the sides (every decoder honours these) and the allocation ([`MAX_ALLOC`]).
fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);

    limits
}

/// The answer for the picture in `bytes`, shrunk to fit a square of `size`: the header and the RGBA pixels. `Unsupported`
/// when it is not a picture of the formats this helper decodes, `InvalidInput` when it is too big, `InvalidData` when it
/// cannot be decoded.
pub(super) fn thumbnail(bytes: &[u8], size: u32) -> io::Result<Vec<u8>> {
    let codec = Format::detect(bytes)
        .and_then(Format::codec)
        .ok_or_else(not_a_picture)?;

    let mut reader = ImageReader::with_format(Cursor::new(bytes), codec);
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(decoding)?;

    // The size is known from the header: nothing is allocated for a picture over a cap.
    let (width, height) = decoder.dimensions();
    check_size(i64::from(width), i64::from(height))?;
    if decoder.total_bytes() > MAX_DECODED_BYTES {
        return Err(too_big());
    }

    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let image = DynamicImage::from_decoder(decoder).map_err(decoding)?;

    // Shrunk before it is turned, and before it is made RGBA, so the big picture is never copied.
    let (new_width, new_height) = fit(width, height, size);
    let mut small = if (new_width, new_height) == (width, height) {
        image
    } else {
        image.thumbnail_exact(new_width, new_height)
    };
    small.apply_orientation(orientation);

    let small = small.into_rgba8();

    Ok(frame(small.width(), small.height(), small.as_raw()))
}

/// What a failure of the decoders is: a limit that was hit is `too big`, anything else a picture that cannot be decoded.
/// The BMP decoder refuses a size it cannot address with an error of its own, not a limit, and says so in its text:
/// that is a picture that is too big, too.
fn decoding(error: ImageError) -> io::Error {
    match &error {
        ImageError::Limits(_) => too_big(),
        ImageError::Decoding(_) if error.to_string().contains("too large") => too_big(),
        _ => undecodable(),
    }
}

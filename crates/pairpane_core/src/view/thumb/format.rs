//! Which picture a file is, from its first bytes, and the arithmetic of a thumbnail: the sizes it may have and the
//! shrinking of RGBA pixels. Nothing here parses a picture; it is safe to run in the app's own process.

use image::imageops;
use image::{ImageFormat, RgbaImage};

use crate::model::ViewKind;

use crate::view::heic::{self, HEADER, undecodable};
use crate::view::sniff;

/// The smallest and the biggest side of a thumbnail the page can ask for (anything else is brought into the range).
pub const MIN_SIZE: u32 = 32;
pub const MAX_SIZE: u32 = 512;

/// The sizes a thumbnail is made at: the size asked for is brought up to the next one, so a page that asks for 100
/// and one that asks for 120 share what is cached.
const BUCKETS: [u32; 8] = [32, 64, 96, 128, 192, 256, 384, 512];

/// The pictures a thumbnail is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    /// HEVC-coded HEIF: decoded by the system's codec in the HEIC helper, never by the thumbnail helper.
    Heic,
}

impl Format {
    /// The picture the file is when its first bytes (`head`, the start of the file) say so, and `None` for anything else
    /// (text, an archive, a movie, a picture of a format that is not listed here). The extension plays no part. The
    /// magic numbers are the viewer's (`sniff.rs`), so what the page opens as a picture of these formats is what gets a
    /// thumbnail; a HEIC file counts only when its brands name HEVC, as the decoder requires.
    pub fn detect(head: &[u8]) -> Option<Self> {
        let head = &head[..head.len().min(HEAD)];

        let ViewKind::Image { mime } = sniff::sniff(head, "") else {
            return None;
        };

        match mime.as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::Webp),
            "image/bmp" => Some(Self::Bmp),
            "image/heic" if heic::is_hevc_heif(head) => Some(Self::Heic),
            _ => None,
        }
    }

    /// The decoder of the `image` crate for this format, and `None` for HEIC, which the crate does not decode.
    pub(super) fn codec(self) -> Option<ImageFormat> {
        match self {
            Self::Png => Some(ImageFormat::Png),
            Self::Jpeg => Some(ImageFormat::Jpeg),
            Self::Gif => Some(ImageFormat::Gif),
            Self::Webp => Some(ImageFormat::WebP),
            Self::Bmp => Some(ImageFormat::Bmp),
            Self::Heic => None,
        }
    }
}

/// How much of the start of a file [`Format::detect`] looks at (enough for the `ftyp` box of a HEIC file).
pub const HEAD: usize = 4096;

/// `size` brought into [`MIN_SIZE`]..=[`MAX_SIZE`].
pub fn clamp_size(size: u32) -> u32 {
    size.clamp(MIN_SIZE, MAX_SIZE)
}

/// The size a thumbnail is made at when `size` is asked for: the clamped size brought up to the next of the sizes the
/// thumbnails come in.
pub fn bucket(size: u32) -> u32 {
    let size = clamp_size(size);

    BUCKETS
        .iter()
        .copied()
        .find(|&bucket| bucket >= size)
        .unwrap_or(MAX_SIZE)
}

/// The size of a picture of `width` by `height` shrunk to fit a square of `size`, with its aspect ratio kept (each side
/// is rounded to the nearest pixel and is never 0). A picture that fits keeps its size: a thumbnail is never bigger than
/// the picture.
pub fn fit(width: u32, height: u32, size: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= size {
        return (width, height);
    }

    let scaled = |side: u32| {
        let scaled = (u64::from(side) * u64::from(size) + u64::from(longest) / 2) / u64::from(longest);

        u32::try_from(scaled).unwrap_or(size).max(1)
    };

    (scaled(width), scaled(height))
}

/// The answer of a picture of `width` by `height` with the pixels `rgba` (tightly packed): the header, then the pixels.
pub fn frame(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + rgba.len());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(rgba);

    out
}

/// A picture in the answer's form (the header and RGBA) shrunk to fit `size`; the same buffer when it fits already.
/// Only resizes pixels: there is nothing to parse, so the app's process may do it with the helper's output. A buffer
/// whose length does not match its header is undecodable. The parent never holds more than the picture it was given
/// and the smaller one.
pub fn shrink(mut picture: Vec<u8>, size: u32) -> std::io::Result<Vec<u8>> {
    let header: [u8; HEADER] = picture
        .get(..HEADER)
        .and_then(|header| header.try_into().ok())
        .ok_or_else(undecodable)?;
    let width = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let height = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);

    let pixels = u64::from(width) * u64::from(height) * 4;
    if width == 0 || height == 0 || pixels != (picture.len() - HEADER) as u64 {
        return Err(undecodable());
    }

    let (new_width, new_height) = fit(width, height, size);
    if (new_width, new_height) == (width, height) {
        return Ok(picture);
    }

    // The header is taken off in place: the picture can be a quarter of a gigabyte, and a copy of it would double that.
    picture.drain(..HEADER);
    let image = RgbaImage::from_raw(width, height, picture).ok_or_else(undecodable)?;
    let small = imageops::thumbnail(&image, new_width, new_height);

    Ok(frame(new_width, new_height, small.as_raw()))
}

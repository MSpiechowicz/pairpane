//! The size a picture's header gives: which format it is (the viewer's magic numbers, `sniff`) and where that format
//! keeps its width and height. PNG, GIF and BMP are a few bytes at fixed places and live here; JPEG, WebP and HEIF have
//! files of their own.

use crate::model::{Dimensions, ViewKind};
use crate::view::sniff::sniff;

use super::{HEAD_BYTES, heif, jpeg, webp};

/// The size of the picture whose first bytes are `head` (at most [`HEAD_BYTES`] of them are looked at), or `None` when it
/// is a picture of no format that is read here, or its header is short, wrong or says a side is zero. The extension plays
/// no part.
pub(super) fn from_head(head: &[u8]) -> Option<Dimensions> {
    let head = &head[..head.len().min(HEAD_BYTES)];

    let ViewKind::Image { mime } = sniff(head, "") else {
        return None;
    };

    match mime.as_str() {
        "image/png" => png(head),
        "image/jpeg" => jpeg::dimensions(head),
        "image/gif" => gif(head),
        "image/bmp" => bmp(head),
        "image/webp" => webp::dimensions(head),
        "image/heic" | "image/avif" => heif::dimensions(head),
        _ => None,
    }
}

/// The size as a [`Dimensions`], when neither side is zero.
pub(super) fn sized(width: u32, height: u32) -> Option<Dimensions> {
    (width != 0 && height != 0).then_some(Dimensions { width, height })
}

/// The `N` bytes at `at`, or `None` when the slice ends first.
fn array<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

pub(super) fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    array(bytes, at).map(u16::from_be_bytes)
}

pub(super) fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    array(bytes, at).map(u32::from_be_bytes)
}

pub(super) fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    array(bytes, at).map(u16::from_le_bytes)
}

pub(super) fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    array(bytes, at).map(u32::from_le_bytes)
}

fn le_i32(bytes: &[u8], at: usize) -> Option<i32> {
    array(bytes, at).map(i32::from_le_bytes)
}

/// PNG: the signature, then the `IHDR` chunk (length 13) whose first two fields are the width and the height.
fn png(head: &[u8]) -> Option<Dimensions> {
    if be32(head, 8)? != 13 || head.get(12..16)? != b"IHDR" {
        return None;
    }

    sized(be32(head, 16)?, be32(head, 20)?)
}

/// GIF: the logical screen descriptor right after the six bytes of the signature, two 16-bit sides, little endian.
fn gif(head: &[u8]) -> Option<Dimensions> {
    sized(u32::from(le16(head, 6)?), u32::from(le16(head, 8)?))
}

/// BMP: the size of the info header at byte 14 says which one follows. The core header (12 bytes) has two 16-bit sides;
/// every later one starts with a 32-bit width and a signed 32-bit height, which is negative for rows stored top down.
fn bmp(head: &[u8]) -> Option<Dimensions> {
    if le32(head, 14)? == 12 {
        return sized(u32::from(le16(head, 18)?), u32::from(le16(head, 20)?));
    }

    let width = u32::try_from(le_i32(head, 18)?).ok()?;
    let height = le_i32(head, 22)?.unsigned_abs();

    sized(width, height)
}

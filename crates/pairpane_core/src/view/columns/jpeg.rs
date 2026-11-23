//! JPEG: the markers are walked from the start to the first start-of-frame segment, which holds the sides.
//!
//! Every segment is `FF`, a marker byte and (for most) a 16-bit length that counts itself. The walk takes each segment by
//! its length, so an APPn or comment segment of any size is skipped in one step, and it stops at the start of the scan
//! (`SOS`, after which comes entropy-coded data and no header), at the end of the image, at anything that is not a marker
//! and at the end of the bytes it was given. Each step moves forward by at least two bytes, so the loop is bounded by
//! the length of the bytes (at most [`HEAD_BYTES`](super::HEAD_BYTES)) and nothing is allocated.
//!
//! The EXIF orientation is applied when it is cheap: the `APP1` segment is in the bytes, so the walk needs no extra read,
//! and finding the one tag walks at most [`MAX_TAGS`] entries of the first directory. A picture that is stored
//! sideways (orientation 5 to 8) then has its sides swapped, as a viewer shows it. An orientation that cannot be read
//! leaves the sides as the frame header gave them.

use crate::model::Dimensions;

use super::header::{be16, be32, le16, le32, sized};

/// The most entries of the first EXIF directory that are looked at for the orientation tag.
const MAX_TAGS: usize = 64;

/// The orientation tag of an EXIF directory.
const ORIENTATION: u16 = 0x0112;

/// The size of the JPEG whose first bytes are `head`: `None` when no frame header is reached.
pub(super) fn dimensions(head: &[u8]) -> Option<Dimensions> {
    let mut at = 2;
    let mut sideways = false;

    // A segment is at least two bytes, so this is more than enough steps and cannot run past the end.
    for _ in 0..head.len() / 2 {
        // Fill bytes (`FF FF`) may come before a marker; anything else that is not `FF` is not a marker.
        while *head.get(at)? == 0xFF && *head.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *head.get(at)? != 0xFF {
            return None;
        }

        let marker = *head.get(at + 1)?;
        at += 2;

        match marker {
            // The markers that stand alone: no length follows. `00` is a stuffed byte, only inside scan data.
            0x01 | 0xD0..=0xD8 => continue,
            // The end of the image, and the start of the scan: no header after either.
            0xD9 | 0xDA | 0x00 => return None,
            _ => {}
        }

        let length = usize::from(be16(head, at)?);
        if length < 2 {
            return None;
        }

        if is_frame(marker) {
            let sizes = (be16(head, at + 5)?, be16(head, at + 3)?);

            return frame(length, sizes, sideways);
        }

        if marker == 0xE1 {
            let segment = head.get(at + 2..at + length);
            sideways |= segment.is_some_and(is_sideways);
        }

        at += length;
    }

    None
}

/// Whether `marker` starts a frame header: `SOF0` to `SOF15`, except the table markers among them (`DHT`, `JPG`, `DAC`).
fn is_frame(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// The frame header with this `length` (precision, height, width, components: at least 8 bytes) and `(width, height)`.
fn frame(length: usize, (width, height): (u16, u16), sideways: bool) -> Option<Dimensions> {
    if length < 8 {
        return None;
    }

    let (width, height) = if sideways { (height, width) } else { (width, height) };

    sized(u32::from(width), u32::from(height))
}

/// Whether the data of an `APP1` segment is EXIF whose orientation says the picture is turned a quarter (5 to 8).
fn is_sideways(segment: &[u8]) -> bool {
    let Some(tiff) = segment.strip_prefix(b"Exif\0\0") else {
        return false;
    };

    matches!(orientation(tiff), Some(5..=8))
}

/// The orientation of the TIFF structure `tiff` (a byte order mark, 42, the offset of the first directory), read from
/// the first directory only.
fn orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let short = |at: usize| if little { le16(tiff, at) } else { be16(tiff, at) };
    let long = |at: usize| if little { le32(tiff, at) } else { be32(tiff, at) };

    if short(2)? != 42 {
        return None;
    }

    let directory = usize::try_from(long(4)?).ok()?;
    let count = usize::from(short(directory)?).min(MAX_TAGS);

    for index in 0..count {
        let entry = directory.checked_add(2 + index * 12)?;

        // The type is 3 (a 16-bit number) and the value sits in the first two bytes of the value field.
        if short(entry)? == ORIENTATION && short(entry.checked_add(2)?)? == 3 {
            return short(entry.checked_add(8)?);
        }
    }

    None
}

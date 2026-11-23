//! WebP: a RIFF file whose first chunk says which of the three encodings it is, and each keeps its sides in its own way.

use crate::model::Dimensions;

use super::header::{le16, le32, sized};

/// The size of the WebP picture whose first bytes are `head`: `RIFF`, a size, `WEBP`, then the first chunk (its type,
/// its size and its data, from byte 20).
pub(super) fn dimensions(head: &[u8]) -> Option<Dimensions> {
    match head.get(12..16)? {
        b"VP8 " => lossy(head),
        b"VP8L" => lossless(head),
        b"VP8X" => extended(head),
        _ => None,
    }
}

/// `VP8 `: a key frame starts with a 3-byte tag (bit 0 clear is a key frame), the start code `9D 01 2A`, then two 16-bit
/// sides whose low 14 bits are the size (the top two are a scale, which is not part of the size).
fn lossy(head: &[u8]) -> Option<Dimensions> {
    let tag = *head.get(20)?;

    if tag & 1 != 0 || head.get(23..26)? != [0x9D, 0x01, 0x2A] {
        return None;
    }

    sized(u32::from(le16(head, 26)? & 0x3FFF), u32::from(le16(head, 28)? & 0x3FFF))
}

/// `VP8L`: the signature byte `2F`, then 14 bits of width minus one and 14 bits of height minus one.
fn lossless(head: &[u8]) -> Option<Dimensions> {
    if *head.get(20)? != 0x2F {
        return None;
    }

    let bits = le32(head, 21)?;

    sized((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1)
}

/// `VP8X`: flags and three reserved bytes, then the canvas as 24-bit width minus one and height minus one.
fn extended(head: &[u8]) -> Option<Dimensions> {
    sized(le24(head, 24)? + 1, le24(head, 27)? + 1)
}

/// The 24-bit little-endian number at `at`.
fn le24(head: &[u8], at: usize) -> Option<u32> {
    let &[low, middle, high] = head.get(at..at + 3)? else {
        return None;
    };

    Some(u32::from(low) | u32::from(middle) << 8 | u32::from(high) << 16)
}

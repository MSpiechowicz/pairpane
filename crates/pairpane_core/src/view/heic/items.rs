//! Which codecs a HEIF file asks for, read before the file reaches a codec.
//!
//! The `ftyp` brands name what a file claims to be, not what it holds: a file of the brands `mif1` and `heic` can carry
//! an AV1, a JPEG 2000 or an uncompressed image that libheif and its other plugins would decode as well, and each of
//! them is more code that parses a hostile file. [`only_hevc_items`] walks the boxes that list the file's items
//! (`meta`, its `iinf`, and the `infe` of each item) and accepts the file only when every item is one that needs no
//! codec but HEVC: the HEVC coded images (`hvc1`, `hvt1`), the items that only arrange them (`grid`, `iden`, `iovl`,
//! `tmap`) and the ones that are no picture (`Exif`, `mime`, `uri `).
//!
//! The walk is safe Rust over the bytes, never trusts a size (a box that does not fit its parent is a malformed file),
//! takes the same boxes whatever libheif's version, looks at no more than [`MAX_BOXES`] boxes in a list and refuses what it does not
//! understand: a missing `meta` or `iinf`, an `infe` before version 2 (which has no item type), an item type that is
//! not listed, a second `meta` or `iinf`.

use crate::view::MAX_IMAGE_BYTES;

/// The item types that need no codec but HEVC (see the module).
const ALLOWED: [[u8; 4]; 9] = [
    *b"hvc1", *b"hvt1", *b"grid", *b"iden", *b"iovl", *b"tmap", *b"Exif", *b"mime", *b"uri ",
];

/// The most boxes looked at in one list (the top level, the children of `meta`, the items) before a file is refused.
const MAX_BOXES: usize = 4096;

/// One box cut off the front of its bytes: its type, its content and what follows it.
struct Split<'a> {
    kind: [u8; 4],
    content: &'a [u8],
    rest: &'a [u8],
}

/// The first box of `data`: a size of 1 is a 64-bit size after the type, a size of 0 is to the end of `data`. `None`
/// when the head is cut short or the box is smaller than its head or does not fit in `data`.
fn split_box(data: &[u8]) -> Option<Split<'_>> {
    let size = u32::from_be_bytes(data.get(..4)?.try_into().ok()?);
    let kind: [u8; 4] = data.get(4..8)?.try_into().ok()?;

    let (head, total) = match size {
        0 => (8, data.len() as u64),
        1 => (16, u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)),
        size => (8, u64::from(size)),
    };

    let total = usize::try_from(total).ok()?;
    if total < head || total > data.len() {
        return None;
    }

    Some(Split {
        kind,
        content: &data[head..total],
        rest: &data[total..],
    })
}

/// The content of a full box (a version byte and three bytes of flags before it), and its version.
fn full_box(content: &[u8]) -> Option<(u8, &[u8])> {
    Some((*content.first()?, content.get(4..)?))
}

/// Whether the HEIF file `bytes` lists only items of types that [`ALLOWED`] holds (see the module).
pub(super) fn only_hevc_items(bytes: &[u8]) -> bool {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return false;
    }

    let Some(meta) = top_level_meta(bytes) else {
        return false;
    };

    let Some(iinf) = single_child(meta, *b"iinf") else {
        return false;
    };

    items_allowed(iinf).is_some()
}

/// The content of the one `meta` box of the file, after its version and flags. The boxes before it must be whole; after
/// it, a second `meta` makes the file ambiguous (refused), and a box that is malformed ends the walk, because no reader
/// gets past it to a later box either.
fn top_level_meta(bytes: &[u8]) -> Option<&[u8]> {
    let mut found = None;
    let mut rest = bytes;

    for _ in 0..MAX_BOXES {
        if rest.is_empty() {
            return found;
        }

        let Some(split) = split_box(rest) else {
            // Past the `meta` box, a box that is cut short or too big stops the walk; before it, it refuses the file.
            return found;
        };

        if split.kind == *b"meta" {
            if found.is_some() {
                return None;
            }

            found = Some(full_box(split.content)?.1);
        }
        rest = split.rest;
    }

    None
}

/// The content of the one box of type `kind` among the boxes of `children`. `None` when there is none, there are two, or
/// a box is malformed.
fn single_child(children: &[u8], kind: [u8; 4]) -> Option<&[u8]> {
    let mut found = None;
    let mut rest = children;

    for _ in 0..MAX_BOXES {
        if rest.is_empty() {
            return found;
        }

        let split = split_box(rest)?;
        if split.kind == kind {
            if found.is_some() {
                return None;
            }

            found = Some(split.content);
        }
        rest = split.rest;
    }

    None
}

/// `Some(())` when every `infe` of the `iinf` content `content` (with its version and flags) is of an allowed type.
fn items_allowed(content: &[u8]) -> Option<()> {
    let (version, body) = full_box(content)?;

    // The count of entries: 16 bits in version 0, 32 after. It is not trusted; every box that follows is looked at.
    let mut rest = if version == 0 { body.get(2..)? } else { body.get(4..)? };

    for _ in 0..MAX_BOXES {
        if rest.is_empty() {
            return Some(());
        }

        let split = split_box(rest)?;
        if split.kind != *b"infe" || !item_allowed(split.content) {
            return None;
        }
        rest = split.rest;
    }

    // More than MAX_BOXES entries.
    None
}

/// Whether the `infe` content `content` is a version 2 or 3 entry of an allowed item type. Version 2 has a 16-bit item
/// ID, version 3 a 32-bit one; then a 16-bit protection index and the item type. Earlier versions have no type.
fn item_allowed(content: &[u8]) -> bool {
    let Some((version, body)) = full_box(content) else {
        return false;
    };

    let id = match version {
        2 => 2,
        3 => 4,
        _ => return false,
    };
    let Some(kind) = body.get(id + 2..id + 6) else {
        return false;
    };

    ALLOWED.iter().any(|allowed| allowed == kind)
}

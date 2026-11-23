//! HEIC and AVIF: the size of the primary picture, from the `ispe` property the container gives it.
//!
//! The container is boxes inside boxes, and the box walk is the one the HEIC helper's pre-check uses (`heic/items.rs`):
//! a box that does not fit its parent ends the walk, and no list is looked at for more than `MAX_BOXES` boxes. The file
//! itself is never decoded. The path is `meta`, then `pitm` (which item is the primary one), `iprp` with `ipma` (which
//! properties that item has, by their number in `ipco`) and `ipco` (the properties). The size is the `ispe` among
//! them, then what the item's `clap` (a crop: encoders pad a picture to a multiple of the block size and crop it back) and
//! `irot` (a quarter turn swaps the sides) do to it, in the order the item lists its properties in `ipma` (as the
//! standard says), each found by its number among the boxes of `ipco`; a crop that cannot be computed is ignored. A file whose `meta` box does not lie within the bytes
//! (one that comes after the picture data, which is the case for some encoders) has no dimensions here.

use crate::model::Dimensions;
use crate::view::heic::items::{MAX_BOXES, Split, full_box, single_child, split_box, top_level_meta};

use super::header::{be16, be32, sized};

/// The size of the primary picture of the HEIF file whose first bytes are `head`.
pub(super) fn dimensions(head: &[u8]) -> Option<Dimensions> {
    let meta = top_level_meta(head)?;
    let primary = primary_item(single_child(meta, *b"pitm")?)?;
    let properties = single_child(meta, *b"iprp")?;

    let wanted = Wanted::of(single_child(properties, *b"ipma")?, primary)?;
    let ipco = single_child(properties, *b"ipco")?;

    let mut size = None;

    // The item's own list of properties, in its order: that is the order the transformations apply in.
    for index in 0..wanted.len() {
        let Some(split) = wanted.get(index).and_then(|number| property(ipco, number)) else {
            continue;
        };

        match &split.kind {
            b"ispe" => size = Some(ispe(split.content)?),
            b"clap" => size = size.map(|(width, height)| cropped(split.content).unwrap_or((width, height))),
            b"irot" if split.content.first()? & 1 == 1 => size = size.map(|(width, height)| (height, width)),
            _ => {}
        }
    }

    let (width, height) = size?;

    sized(width, height)
}

/// The property with this number (from 1) among the boxes of `ipco`. `None` for 0 (no property), a number past the
/// boxes or past [`MAX_BOXES`], and when a box does not fit. The walk takes at most `number` steps.
fn property(ipco: &[u8], number: usize) -> Option<Split<'_>> {
    if number == 0 || number > MAX_BOXES {
        return None;
    }

    let mut rest = ipco;

    for _ in 1..number {
        rest = split_box(rest)?.rest;
    }

    split_box(rest)
}

/// The ID of the primary item: 16 bits in version 0, 32 after.
fn primary_item(content: &[u8]) -> Option<u32> {
    let (version, body) = full_box(content)?;

    if version == 0 {
        be16(body, 0).map(u32::from)
    } else {
        be32(body, 0)
    }
}

/// The size a `clap` (clean aperture: the width and the height as fractions, then the offset of the centre) crops to:
/// each fraction rounded down. `None` when a denominator is zero or a side comes to zero, and the crop is then ignored.
fn cropped(content: &[u8]) -> Option<(u32, u32)> {
    let side = |at: usize| be32(content, at)?.checked_div(be32(content, at + 4)?);

    let (width, height) = (side(0)?, side(8)?);

    (width != 0 && height != 0).then_some((width, height))
}

/// The width and the height of an `ispe` (a full box of two 32-bit numbers).
fn ispe(content: &[u8]) -> Option<(u32, u32)> {
    let (_, body) = full_box(content)?;

    Some((be32(body, 0)?, be32(body, 4)?))
}

/// The associations of one item in `ipma`: a flag for "essential" and a property number each, in one byte (7 bits of
/// number) or, when the flags of the box say so, two (15 bits). They stay as the bytes of the file and are read in place,
/// so nothing is allocated.
struct Wanted<'a> {
    list: &'a [u8],
    wide: bool,
}

impl<'a> Wanted<'a> {
    /// The associations `ipma` (the box `content`) gives to `item`. `None` when the box is malformed or lists no such item.
    fn of(content: &'a [u8], item: u32) -> Option<Self> {
        let (version, body) = full_box(content)?;
        let wide = *content.get(3)? & 1 == 1;
        let id_bytes = if version == 0 { 2 } else { 4 };
        let each = if wide { 2 } else { 1 };

        // The count is not trusted: it only bounds the walk, which also ends where the bytes do.
        let entries = be32(body, 0)?;
        let mut rest = body.get(4..)?;

        for _ in 0..entries.min(u32::try_from(MAX_BOXES).ok()?) {
            let id = if version == 0 {
                u32::from(be16(rest, 0)?)
            } else {
                be32(rest, 0)?
            };
            let count = usize::from(*rest.get(id_bytes)?);
            let end = id_bytes + 1 + count * each;
            let list = rest.get(id_bytes + 1..end)?;
            rest = rest.get(end..)?;

            if id == item {
                return Some(Self { list, wide });
            }
        }

        None
    }

    /// How many properties the item lists.
    fn len(&self) -> usize {
        self.list.len() / if self.wide { 2 } else { 1 }
    }

    /// The number of the property listed `index`th (from 0), without the flag for "essential".
    fn get(&self, index: usize) -> Option<usize> {
        if self.wide {
            return be16(self.list, index.checked_mul(2)?).map(|number| usize::from(number & 0x7FFF));
        }

        self.list.get(index).map(|byte| usize::from(byte & 0x7F))
    }
}

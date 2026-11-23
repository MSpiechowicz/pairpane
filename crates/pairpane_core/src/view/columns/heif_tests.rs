//! The size of a HEIC or AVIF picture: the primary item's `ispe`, found through `pitm` and `ipma`, and the files that
//! must have none.

use pretty_assertions::assert_eq;

use crate::model::Dimensions;

use crate::view::heic::items_tests::boxed;

use super::fixtures::{avif, clap, heif, heif_crowded, heif_listed, irot};
use super::header::from_head;

/// A 16 by 8 picture of one colour made by `heif-enc` (one `hvc1` item): a real file, not a built one.
const TINY: &[u8] = include_bytes!("../heic_testdata/tiny.heic");

fn size(width: u32, height: u32) -> Option<Dimensions> {
    Some(Dimensions { width, height })
}

#[test]
fn a_real_heic_file_gives_its_size() {
    assert_eq!(from_head(TINY), size(16, 8));
}

#[test]
fn the_ispe_of_the_primary_item_is_taken_and_not_the_first_one_in_the_file() {
    assert_eq!(from_head(&heif(640, 480, &[], false)), size(640, 480));
    assert_eq!(
        from_head(&heif(640, 480, &[], true)),
        size(640, 480),
        "32-bit ids, 16-bit numbers"
    );
}

#[test]
fn a_quarter_turn_swaps_the_sides_and_a_half_turn_does_not() {
    assert_eq!(from_head(&heif(640, 480, &[irot(0)], false)), size(640, 480));
    assert_eq!(from_head(&heif(640, 480, &[irot(1)], false)), size(480, 640));
    assert_eq!(from_head(&heif(640, 480, &[irot(2)], false)), size(640, 480));
    assert_eq!(from_head(&heif(640, 480, &[irot(3)], true)), size(480, 640));
}

#[test]
fn a_crop_is_applied_before_a_turn_and_a_crop_that_cannot_be_computed_is_ignored() {
    assert_eq!(from_head(&heif(64, 64, &[clap(16, 8)], false)), size(16, 8));
    assert_eq!(from_head(&heif(64, 64, &[clap(16, 8), irot(1)], false)), size(8, 16));

    let mut no_denominator = heif(64, 64, &[clap(16, 8)], false);
    let at = no_denominator
        .windows(4)
        .position(|window| window == b"clap")
        .expect("clap");
    no_denominator[at + 8..at + 12].copy_from_slice(&0_u32.to_be_bytes());
    assert_eq!(from_head(&no_denominator), size(64, 64));

    assert_eq!(
        from_head(&heif(64, 64, &[clap(0, 8)], false)),
        size(64, 64),
        "a crop to nothing"
    );
}

#[test]
fn the_properties_apply_in_the_order_the_item_lists_them_and_not_the_order_of_the_boxes() {
    // `ipco` is: the tile's size (1), the size (2), a crop to 300 by 200 (3), a quarter turn (4).
    let boxes = [clap(300, 200), irot(1)];

    // Cropped first, then turned.
    assert_eq!(
        from_head(&heif_listed(640, 480, &boxes, &[2, 3, 4], false)),
        size(200, 300)
    );
    // Turned first (640 by 480 becomes 480 by 640), then cropped to what the crop says.
    assert_eq!(
        from_head(&heif_listed(640, 480, &boxes, &[2, 4, 3], false)),
        size(300, 200)
    );
    assert_eq!(
        from_head(&heif_listed(640, 480, &boxes, &[2, 4, 3], true)),
        size(300, 200),
        "16-bit numbers"
    );
    // A property the item does not list is not applied, and a number that names nothing is passed over.
    assert_eq!(
        from_head(&heif_listed(640, 480, &boxes, &[2, 4, 0, 9], false)),
        size(480, 640)
    );
    assert_eq!(from_head(&heif_listed(640, 480, &boxes, &[2], false)), size(640, 480));
}

#[test]
fn an_avif_container_is_read_the_same_way() {
    assert_eq!(from_head(&avif(640, 480)), size(640, 480));
}

#[test]
fn a_property_list_that_is_crowded_is_walked_to_the_number_and_no_further_than_the_cap() {
    // The quarter turn is property 4093, within what a list is walked for, and with 4094 other boxes it is past it.
    assert_eq!(from_head(&heif_crowded(4090)), size(480, 640));
    assert_eq!(from_head(&heif_crowded(4094)), size(640, 480));
}

#[test]
fn a_file_whose_meta_box_is_not_in_the_bytes_that_were_read_has_no_dimensions() {
    let whole = heif(640, 480, &[], false);
    let ftyp = 8 + 16;

    // The picture data first, then the box with the sizes, which is past the end of what is read.
    let mut late = whole[..ftyp].to_vec();
    late.extend_from_slice(&boxed(b"mdat", &vec![0; 70_000]));
    late.extend_from_slice(&whole[ftyp..]);
    assert_eq!(from_head(&late), None);

    // Cut inside the meta box.
    assert_eq!(from_head(&whole[..whole.len() - 40]), None);
}

#[test]
fn a_container_that_does_not_add_up_has_no_dimensions() {
    let whole = heif(640, 480, &[], false);

    for (name, change) in [
        ("no primary item", b"pitm" as &[u8; 4]),
        ("no property box", b"iprp"),
        ("no association box", b"ipma"),
        ("no property list", b"ipco"),
        ("no size", b"ispe"),
    ] {
        let mut bytes = whole.clone();
        // Renaming a box hides it from the walk.
        for at in 0..bytes.len() - 4 {
            if &bytes[at..at + 4] == change {
                bytes[at..at + 4].copy_from_slice(b"free");
            }
        }

        assert_eq!(from_head(&bytes), None, "{name}");
    }
}

#[test]
fn an_item_the_associations_do_not_list_has_no_dimensions() {
    let mut bytes = heif(640, 480, &[], false);
    // The primary item is 1; make `pitm` say 7, which `ipma` has no entry for.
    let pitm = bytes.windows(4).position(|window| window == b"pitm").expect("pitm");
    bytes[pitm + 8..pitm + 10].copy_from_slice(&7_u16.to_be_bytes());

    assert_eq!(from_head(&bytes), None);
}

#[test]
fn sides_of_zero_in_the_ispe_are_refused_and_huge_ones_are_returned() {
    assert_eq!(from_head(&heif(0, 480, &[], false)), None);
    assert_eq!(from_head(&heif(640, 0, &[], false)), None);
    assert_eq!(
        from_head(&heif(u32::MAX, u32::MAX, &[], false)),
        size(u32::MAX, u32::MAX)
    );
}

#[test]
fn an_association_count_that_lies_is_stopped_by_the_end_of_the_bytes() {
    let mut bytes = heif(640, 480, &[], false);
    let ipma = bytes.windows(4).position(|window| window == b"ipma").expect("ipma");
    // The count of entries, after the type and the version and flags.
    bytes[ipma + 8..ipma + 12].copy_from_slice(&u32::MAX.to_be_bytes());
    // Make the item that is looked for absent too, so that every entry is walked.
    let pitm = bytes.windows(4).position(|window| window == b"pitm").expect("pitm");
    bytes[pitm + 8..pitm + 10].copy_from_slice(&9_u16.to_be_bytes());

    assert_eq!(from_head(&bytes), None);
}

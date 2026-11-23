//! The JPEG marker walk: where the frame header is found, what is skipped on the way, where the walk stops, and the EXIF
//! orientation.

use pretty_assertions::assert_eq;

use crate::model::Dimensions;

use super::HEAD_BYTES;
use super::fixtures::{exif, frame, jpeg, jpeg_of, segment};
use super::header::from_head;

fn size(width: u32, height: u32) -> Option<Dimensions> {
    Some(Dimensions { width, height })
}

#[test]
fn a_jpeg_gives_the_sides_of_its_first_frame_header() {
    assert_eq!(from_head(&jpeg(640, 480)), size(640, 480));
    assert_eq!(from_head(&jpeg(1, 65535)), size(1, 65535));
}

#[test]
fn segments_before_the_frame_are_skipped_by_their_length_whatever_they_hold() {
    // Bytes in a segment that look like a frame header or a marker mean nothing: the length decides.
    let decoy = segment(0xE2, &[0xFF, 0xC0, 0, 17, 8, 0, 1, 0, 1, 3]);
    let comment = segment(0xFE, &[0xFF; 300]);
    let tables = segment(0xDB, &[0; 65]);
    let huffman = segment(0xC4, &[0; 20]);

    assert_eq!(
        from_head(&jpeg_of(&[decoy, comment, tables, huffman], 800, 600)),
        size(800, 600)
    );
}

#[test]
fn fill_bytes_and_markers_without_a_length_are_stepped_over() {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xFF, 0xFF, 0x01, 0xFF, 0xD0];
    bytes.extend_from_slice(&frame(30, 20));

    assert_eq!(from_head(&bytes), size(30, 20));
}

#[test]
fn the_progressive_and_the_other_frame_markers_count_but_the_table_markers_do_not() {
    for marker in [0xC1, 0xC2, 0xC3, 0xC5, 0xC9, 0xCF] {
        let mut bytes = vec![0xFF, 0xD8];
        let mut header = frame(30, 20);
        header[1] = marker;
        bytes.extend_from_slice(&header);

        assert_eq!(from_head(&bytes), size(30, 20), "{marker:#x}");
    }

    for marker in [0xC4, 0xC8, 0xCC] {
        let mut bytes = vec![0xFF, 0xD8];
        let mut table = frame(30, 20);
        table[1] = marker;
        bytes.extend_from_slice(&table);

        assert_eq!(from_head(&bytes), None, "{marker:#x}");
    }
}

#[test]
fn the_walk_stops_at_the_scan_and_at_the_end_of_the_image() {
    let scan_first = [0xFF, 0xD8, 0xFF, 0xDA, 0, 2, 0xFF, 0xC0, 0, 17, 8, 0, 1, 0, 1, 3];
    let ended = [0xFF, 0xD8, 0xFF, 0xD9];

    assert_eq!(from_head(&scan_first), None);
    assert_eq!(from_head(&ended), None);
}

#[test]
fn a_length_that_is_too_small_or_a_byte_that_is_no_marker_ends_the_walk() {
    assert_eq!(
        from_head(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 1, 0, 0]),
        None,
        "a length under 2"
    );
    assert_eq!(from_head(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0]), None, "a length of 0");
    assert_eq!(from_head(&[0xFF, 0xD8, 0xFF, 0x00, 0, 4, 0, 0]), None, "a stuffed byte");
    assert_eq!(
        from_head(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 7, 7, 7, 7]),
        None,
        "no marker after a segment"
    );

    let mut short_frame = frame(30, 20);
    short_frame[3] = 7;
    let mut bytes = vec![0xFF, 0xD8];
    bytes.extend_from_slice(&short_frame);
    assert_eq!(
        from_head(&bytes),
        None,
        "a frame header too short to hold the sides and the components"
    );
}

#[test]
fn a_frame_header_cut_off_has_no_dimensions() {
    let whole = jpeg(640, 480);
    let frame_at = 2 + segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0").len();
    // The sides end 7 bytes after the marker's two bytes and the length's two (precision, height, width).
    let needed = frame_at + 9;

    assert_eq!(from_head(&whole[..needed]), size(640, 480));
    assert_eq!(from_head(&whole[..needed - 1]), None);
    assert_eq!(from_head(&whole[..frame_at + 2]), None);
}

#[test]
fn many_big_segments_before_the_frame_stop_the_walk_at_the_end_of_the_bytes_that_were_read() {
    // 40 segments of 4 KiB are 160 KiB before the frame header, which is not in the first 64 KiB.
    let padding: Vec<Vec<u8>> = (0..40).map(|_| segment(0xE2, &[0; 4096])).collect();
    let far = jpeg_of(&padding, 640, 480);
    assert!(far.len() > 2 * HEAD_BYTES);

    assert_eq!(from_head(&far), None);
    assert_eq!(from_head(&far[..HEAD_BYTES]), None);

    // The same with the frame header inside the window is found.
    let near: Vec<Vec<u8>> = (0..10).map(|_| segment(0xE2, &[0; 4096])).collect();
    assert_eq!(from_head(&jpeg_of(&near, 640, 480)), size(640, 480));
}

#[test]
fn a_walk_over_nothing_but_fill_bytes_and_empty_markers_ends() {
    let mut fill = vec![0xFF, 0xD8];
    fill.extend_from_slice(&[0xFF; HEAD_BYTES]);
    let mut empty = vec![0xFF, 0xD8, 0xFF];
    for _ in 0..HEAD_BYTES / 2 {
        empty.extend_from_slice(&[0xD0, 0xFF]);
    }

    assert_eq!(from_head(&fill), None);
    assert_eq!(from_head(&empty), None);
}

#[test]
fn a_picture_stored_sideways_has_its_sides_swapped_by_its_exif_orientation() {
    for (orientation, swapped) in [
        (1, false),
        (2, false),
        (3, false),
        (4, false),
        (5, true),
        (6, true),
        (7, true),
        (8, true),
    ] {
        for big_endian in [false, true] {
            let expected = if swapped { size(480, 640) } else { size(640, 480) };

            assert_eq!(
                from_head(&jpeg_of(&[exif(orientation, big_endian)], 640, 480)),
                expected,
                "orientation {orientation}, big endian {big_endian}"
            );
        }
    }
}

#[test]
fn an_exif_that_cannot_be_read_leaves_the_sides_as_the_frame_gave_them() {
    let mut wrong_order = exif(6, false);
    wrong_order[10] = b'X';
    let mut wrong_magic = exif(6, false);
    wrong_magic[12] = 43;
    let mut far_directory = exif(6, false);
    far_directory[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut huge_count = exif(6, false);
    huge_count[20..22].copy_from_slice(&0x0100_u16.to_le_bytes());
    huge_count[18..20].copy_from_slice(&u16::MAX.to_le_bytes());
    let truncated = segment(0xE1, b"Exif\0\0II*\0");
    let not_exif = segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>");

    for (name, app1) in [
        ("wrong byte order", wrong_order),
        ("wrong magic", wrong_magic),
        ("directory far away", far_directory),
        ("a count past the data", huge_count),
        ("truncated", truncated),
        ("not exif", not_exif),
    ] {
        assert_eq!(from_head(&jpeg_of(&[app1], 640, 480)), size(640, 480), "{name}");
    }
}

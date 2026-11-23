//! The size each format's header gives: normal pictures, headers cut short, garbage, and sides that are zero or absurd.

use pretty_assertions::assert_eq;

use crate::model::Dimensions;

use super::fixtures::{bmp_core, bmp_info, gif, jpeg, png, webp_extended, webp_lossless, webp_lossy};
use super::header::from_head;

fn size(width: u32, height: u32) -> Option<Dimensions> {
    Some(Dimensions { width, height })
}

#[test]
fn a_png_gives_the_sides_of_its_ihdr() {
    assert_eq!(from_head(&png(640, 480)), size(640, 480));
    assert_eq!(from_head(&png(1, 1)), size(1, 1));
}

#[test]
fn a_png_whose_first_chunk_is_not_a_header_of_13_bytes_has_no_dimensions() {
    let mut other_chunk = png(10, 10);
    other_chunk[12..16].copy_from_slice(b"IDAT");
    let mut other_length = png(10, 10);
    other_length[8..12].copy_from_slice(&14_u32.to_be_bytes());

    assert_eq!(from_head(&other_chunk), None);
    assert_eq!(from_head(&other_length), None);
}

#[test]
fn a_gif_gives_the_logical_screen() {
    assert_eq!(from_head(&gif(320, 200)), size(320, 200));
    assert_eq!(from_head(&gif(65535, 65535)), size(65535, 65535));

    let mut older = gif(5, 6);
    older[4] = b'7';
    assert_eq!(from_head(&older), size(5, 6));
}

#[test]
fn a_bmp_gives_its_sides_from_either_header_and_a_negative_height_is_a_top_down_picture() {
    assert_eq!(from_head(&bmp_core(100, 50)), size(100, 50));
    assert_eq!(from_head(&bmp_info(100, 50)), size(100, 50));
    assert_eq!(from_head(&bmp_info(100, -50)), size(100, 50));
    assert_eq!(from_head(&bmp_info(1, i32::MIN)), size(1, 2_147_483_648));
}

#[test]
fn a_bmp_with_no_width_or_a_negative_one_has_no_dimensions() {
    assert_eq!(from_head(&bmp_info(0, 50)), None);
    assert_eq!(from_head(&bmp_info(-100, 50)), None);
}

#[test]
fn a_webp_gives_its_sides_in_each_of_its_three_encodings() {
    assert_eq!(from_head(&webp_lossy(640, 480, 0)), size(640, 480));
    assert_eq!(from_head(&webp_lossless(640, 480)), size(640, 480));
    assert_eq!(from_head(&webp_extended(640, 480)), size(640, 480));
}

#[test]
fn a_lossy_webp_ignores_the_scale_bits_and_wants_a_key_frame() {
    assert_eq!(from_head(&webp_lossy(640, 480, 3)), size(640, 480));

    let mut inter_frame = webp_lossy(640, 480, 0);
    inter_frame[20] |= 1;
    assert_eq!(from_head(&inter_frame), None);

    let mut no_start_code = webp_lossy(640, 480, 0);
    no_start_code[23] = 0;
    assert_eq!(from_head(&no_start_code), None);
}

#[test]
fn the_biggest_sides_a_webp_can_say_are_returned_as_they_are() {
    assert_eq!(from_head(&webp_lossless(16384, 16384)), size(16384, 16384));
    assert_eq!(from_head(&webp_extended(1 << 24, 1 << 24)), size(1 << 24, 1 << 24));
}

#[test]
fn sides_that_are_absurd_are_returned_and_sides_that_are_zero_are_refused() {
    assert_eq!(from_head(&png(u32::MAX, u32::MAX)), size(u32::MAX, u32::MAX));
    assert_eq!(from_head(&jpeg(65535, 65535)), size(65535, 65535));

    assert_eq!(from_head(&png(0, 10)), None);
    assert_eq!(from_head(&png(10, 0)), None);
    assert_eq!(from_head(&gif(0, 10)), None);
    assert_eq!(from_head(&gif(10, 0)), None);
    assert_eq!(from_head(&bmp_core(0, 10)), None);
    assert_eq!(from_head(&bmp_info(10, 0)), None);
    assert_eq!(from_head(&jpeg(0, 10)), None);
    assert_eq!(from_head(&jpeg(10, 0)), None);
    assert_eq!(from_head(&webp_lossy(0, 10, 0)), None);
}

#[test]
fn a_header_is_read_as_soon_as_its_last_field_is_there_and_not_before() {
    // The shortest start of each file that holds the sides, and one byte less.
    for (name, bytes, needed) in [
        ("png", png(640, 480), 24),
        ("gif", gif(640, 480), 10),
        ("bmp core", bmp_core(640, 480), 22),
        ("bmp info", bmp_info(640, 480), 26),
        ("webp lossy", webp_lossy(640, 480, 0), 30),
        ("webp lossless", webp_lossless(640, 480), 25),
        ("webp extended", webp_extended(640, 480), 30),
    ] {
        assert_eq!(from_head(&bytes[..needed]), size(640, 480), "{name}");
        assert_eq!(from_head(&bytes[..needed - 1]), None, "{name} one byte short");
    }
}

#[test]
fn what_is_no_picture_of_these_formats_has_no_dimensions() {
    let icon = [0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 0, 0];

    for (name, bytes) in [
        ("empty", &b""[..]),
        ("text", b"hello, this is not a picture at all"),
        ("zeros", &[0; 100]),
        ("a pdf", b"%PDF-1.7\n1 0 obj"),
        ("an icon", &icon),
        ("a movie", b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom"),
        ("a wave", b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0"),
        ("a png signature alone", b"\x89PNG\r\n\x1a\n"),
        (
            "text that starts with BM",
            b"BM and then a sentence that goes on for a while",
        ),
    ] {
        assert_eq!(from_head(bytes), None, "{name}");
    }
}

#[test]
fn only_the_first_64_kib_are_looked_at() {
    let mut late = png(10, 10);
    late.resize(70_000, 0);
    // A WebP whose first chunk is padding is none of the three encodings, so it has none, however long it goes on.
    let mut padded = webp_lossless(10, 10);
    padded[12..16].copy_from_slice(b"JUNK");
    padded.resize(70_000, 0);

    assert_eq!(from_head(&late), size(10, 10), "the header is at the start");
    assert_eq!(from_head(&padded), None);
}

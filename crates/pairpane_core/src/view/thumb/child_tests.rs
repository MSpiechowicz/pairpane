//! What the helper makes of a file and what it writes (in this process: the confinement and the real process are tried by
//! `src-tauri/tests/thumb_helper.rs`).

use std::io::{self, Read};

use image::ImageFormat::{Bmp, Gif, Jpeg, Png, WebP};
use pretty_assertions::assert_eq;

use super::child::{run_child, thumbnail};
use super::fixtures::{animated_gif, encode, is_color, oriented_jpeg, png_claiming, size_of};
use crate::view::heic::HEADER;
use crate::view::heic::process::read_picture;
use crate::view::{MAX_IMAGE_BYTES, is_refusal};

/// The helper's answer for `input` and the exit code.
fn answer(input: impl Read, size: u32) -> (Vec<u8>, i32) {
    let mut output = Vec::new();
    let code = run_child(input, &mut output, size);

    (output, code)
}

fn refused(bytes: &[u8], kind: io::ErrorKind) {
    let error = thumbnail(bytes, 64).expect_err("refused");

    assert_eq!(error.kind(), kind, "{error}");
    assert!(is_refusal(&error), "the text is ours: {error}");
}

#[test]
fn every_format_is_decoded_and_shrunk_to_fit_with_its_proportions() {
    for (format, tolerance) in [(Png, 0), (Gif, 0), (Bmp, 0), (WebP, 0), (Jpeg, 8)] {
        let picture = thumbnail(&encode(format, 200, 100), 64).expect("a thumbnail");

        assert_eq!(size_of(&picture), (64, 32), "{format:?}");
        assert_eq!(picture.len(), HEADER + 64 * 32 * 4, "{format:?}");
        assert!(is_color(&picture[HEADER..], tolerance), "{format:?}");
    }
}

#[test]
fn a_tall_picture_and_a_picture_that_fits_keep_their_shapes() {
    let tall = thumbnail(&encode(Png, 50, 400), 100).expect("a thumbnail");
    // 50 * 100 / 400 is 12.5, which is rounded up.
    assert_eq!(size_of(&tall), (13, 100));

    // 40 by 30 in a square of 64: not made bigger.
    let small = thumbnail(&encode(Png, 40, 30), 64).expect("a thumbnail");
    assert_eq!(size_of(&small), (40, 30));
    assert!(is_color(&small[HEADER..], 0));
}

#[test]
fn a_gif_gives_its_first_frame() {
    let red = [255, 0, 0, 255];
    let gif = animated_gif(40, 40, red, [0, 255, 0, 255]);

    let picture = thumbnail(&gif, 32).expect("a thumbnail");

    assert_eq!(size_of(&picture), (32, 32));
    for pixel in picture[HEADER..].as_chunks::<4>().0 {
        assert_eq!(pixel, &red);
    }
}

#[test]
fn the_exif_orientation_of_a_picture_is_applied() {
    // Stored 200 by 100 and to be shown turned a quarter: it is 100 wide and 200 tall.
    let picture = thumbnail(&oriented_jpeg(200, 100, 6), 64).expect("a thumbnail");
    assert_eq!(size_of(&picture), (32, 64));

    let upright = thumbnail(&oriented_jpeg(200, 100, 1), 64).expect("a thumbnail");
    assert_eq!(size_of(&upright), (64, 32));
}

#[test]
fn a_picture_that_claims_more_than_the_caps_allow_is_too_big_and_nothing_is_allocated_for_it() {
    for (name, bomb) in [
        // A side over the cap, whatever the other is.
        ("100000 by 100000", png_claiming(100_000, 100_000, 8, 6)),
        ("100000 by 1", png_claiming(100_000, 1, 8, 6)),
        // Sides that are allowed and pixels that are not: 900 megapixels.
        ("30000 by 30000", png_claiming(30_000, 30_000, 8, 6)),
        // Pixels that are allowed and bytes that are not: 64 megapixels of 16-bit RGBA are 512 MiB.
        ("8000 by 8000 at 16 bits", png_claiming(8_000, 8_000, 16, 6)),
    ] {
        let started = std::time::Instant::now();

        let error = thumbnail(&bomb, 64).expect_err(name);

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}: {error}");
        assert!(is_refusal(&error), "{name}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{name}: refused at once"
        );
    }
}

#[test]
fn a_bomb_in_every_other_format_is_refused_the_same_way() {
    // A BMP header that claims 100000 by 100000 pixels of 24 bits, with no pixels.
    let mut bmp = encode(Bmp, 2, 2);
    bmp[18..22].copy_from_slice(&100_000_i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&100_000_i32.to_le_bytes());
    refused(&bmp, io::ErrorKind::InvalidInput);

    // A GIF whose logical screen is 65535 by 65535.
    let mut gif = encode(Gif, 2, 2);
    gif[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
    gif[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
    refused(&gif, io::ErrorKind::InvalidInput);
}

#[test]
fn a_truncated_or_garbled_picture_cannot_be_decoded() {
    for format in [Png, Jpeg, Gif, WebP, Bmp] {
        let whole = encode(format, 64, 64);

        // Cut in the middle of the pixels, and cut right after the header.
        refused(&whole[..whole.len() / 2], io::ErrorKind::InvalidData);
        refused(&whole[..32], io::ErrorKind::InvalidData);

        // The magic number and then noise.
        let mut garbled = whole[..24].to_vec();
        garbled.extend((0..4000_u32).map(|at| (at.wrapping_mul(2_654_435_761) >> 24) as u8));
        refused(&garbled, io::ErrorKind::InvalidData);
    }
}

#[test]
fn what_is_not_a_picture_of_these_formats_is_unsupported() {
    for bytes in [
        b"just some text".as_slice(),
        b"",
        // A HEIC file is the HEIC helper's, not this one's.
        b"\0\0\0\x18ftypheic\0\0\0\0mif1heic\0\0\0\0",
    ] {
        refused(bytes, io::ErrorKind::Unsupported);
    }
}

#[test]
fn the_answer_is_a_status_and_the_picture_or_a_status_alone() {
    let png = encode(Png, 200, 100);

    let (written, code) = answer(&png[..], 64);
    assert_eq!(code, 0);
    assert_eq!(written[0], 0);
    let picture = read_picture(&written[..], 64).expect("a picture");
    assert_eq!(size_of(&picture), (64, 32));

    for (input, status) in [
        (b"text".to_vec(), 1),
        (png[..30].to_vec(), 2),
        (png_claiming(100_000, 100_000, 8, 6), 3),
    ] {
        let (written, code) = answer(&input[..], 64);

        assert_eq!(code, 0, "an answer was written");
        assert_eq!(written, [status]);
    }
}

#[test]
fn a_file_over_the_cap_is_too_big_and_is_not_read_to_the_end() {
    let input = io::repeat(0).take(MAX_IMAGE_BYTES * 2);

    let (written, code) = answer(input, 64);

    assert_eq!(code, 0);
    assert_eq!(written, [3]);
}

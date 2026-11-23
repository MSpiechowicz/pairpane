//! What a file is by its first bytes, the sizes a thumbnail has and the shrinking of pixels.

use std::io;

use pretty_assertions::assert_eq;

use super::fixtures::{COLOR, encode, is_color, size_of, solid};
use super::format::{Format, MAX_SIZE, MIN_SIZE, bucket, clamp_size, fit, frame, shrink};
use crate::view::heic::HEADER;
use crate::view::is_refusal;
use crate::view::testing::ftyp;

#[test]
fn each_format_is_known_by_its_magic_number() {
    use image::ImageFormat::{Bmp, Gif, Jpeg, Png, WebP};

    for (bytes, format) in [
        (encode(Png, 4, 4), Format::Png),
        (encode(Jpeg, 4, 4), Format::Jpeg),
        (encode(Gif, 4, 4), Format::Gif),
        (encode(WebP, 4, 4), Format::Webp),
        (encode(Bmp, 4, 4), Format::Bmp),
        (b"GIF87a\x01\0\x01\0".to_vec(), Format::Gif),
        (ftyp(b"heic", &[]), Format::Heic),
        (ftyp(b"mif1", &[b"heic"]), Format::Heic),
    ] {
        assert_eq!(Format::detect(&bytes), Some(format), "{:?}", &bytes[..12]);
    }
}

#[test]
fn a_prefix_of_the_start_is_enough_and_only_the_start_counts() {
    let png = encode(image::ImageFormat::Png, 4, 4);

    assert_eq!(Format::detect(&png[..8]), Some(Format::Png));

    let mut shifted = b"\n".to_vec();
    shifted.extend_from_slice(&png);
    assert_eq!(Format::detect(&shifted), None);
}

#[test]
fn what_is_not_one_of_the_formats_is_not_a_picture() {
    let zip = b"PK\x03\x04\x14\0\0\0\0\0".to_vec();
    let wave = b"RIFF\x24\0\0\0WAVEfmt ".to_vec();
    let icon = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec();

    for bytes in [
        b"just some text".to_vec(),
        b"%PDF-1.7\n".to_vec(),
        zip,
        wave,
        icon,
        svg,
        // A picture of a format the helper does not decode, and a HEIF file of another codec.
        ftyp(b"avif", &[b"mif1"]),
        ftyp(b"mif1", &[b"avif"]),
        ftyp(b"mp42", &[]),
        vec![],
        vec![0xFF],
    ] {
        assert_eq!(Format::detect(&bytes), None, "{:?}", &bytes[..bytes.len().min(12)]);
    }
}

#[test]
fn the_codec_is_the_images_for_every_format_but_heic() {
    for format in [Format::Png, Format::Jpeg, Format::Gif, Format::Webp, Format::Bmp] {
        assert!(format.codec().is_some(), "{format:?}");
    }

    assert!(Format::Heic.codec().is_none());
}

#[test]
fn the_size_asked_for_is_brought_into_the_range() {
    assert_eq!(clamp_size(0), MIN_SIZE);
    assert_eq!(clamp_size(31), 32);
    assert_eq!(clamp_size(32), 32);
    assert_eq!(clamp_size(200), 200);
    assert_eq!(clamp_size(512), 512);
    assert_eq!(clamp_size(513), MAX_SIZE);
    assert_eq!(clamp_size(u32::MAX), MAX_SIZE);
}

#[test]
fn a_size_is_brought_up_to_the_next_bucket_and_never_out_of_the_range() {
    for (asked, made) in [
        (0, 32),
        (32, 32),
        (33, 64),
        (64, 64),
        (100, 128),
        (128, 128),
        (129, 192),
        (200, 256),
        (257, 384),
        (400, 512),
        (512, 512),
        (100_000, 512),
        (u32::MAX, 512),
    ] {
        assert_eq!(bucket(asked), made, "asked {asked}");
    }

    for asked in 0..=600 {
        let made = bucket(asked);

        assert!(made >= clamp_size(asked), "never smaller than asked: {asked}");
        assert!((MIN_SIZE..=MAX_SIZE).contains(&made));
        assert_eq!(bucket(made), made, "a bucket is its own bucket");
    }
}

#[test]
fn a_picture_is_shrunk_to_fit_the_square_and_keeps_its_proportions() {
    for (width, height, size, want) in [
        (200, 100, 64, (64, 32)),
        (100, 200, 64, (32, 64)),
        (500, 500, 128, (128, 128)),
        (4000, 3000, 256, (256, 192)),
        // Rounded to the nearest pixel.
        (300, 200, 100, (100, 67)),
        // A very long picture keeps one pixel of its short side.
        (10_000, 1, 64, (64, 1)),
        (1, 10_000, 64, (1, 64)),
        (32_767, 32_767, 32, (32, 32)),
    ] {
        assert_eq!(fit(width, height, size), want, "{width}x{height} in {size}");
    }
}

#[test]
fn a_picture_that_fits_is_not_made_bigger() {
    assert_eq!(fit(16, 8, 64), (16, 8));
    assert_eq!(fit(64, 64, 64), (64, 64));
    assert_eq!(fit(64, 10, 64), (64, 10));
    assert_eq!(fit(1, 1, 32), (1, 1));
}

#[test]
fn shrinking_pixels_gives_the_header_and_the_smaller_picture() {
    let picture = frame(200, 100, solid(200, 100, COLOR).as_raw());

    let small = shrink(picture, 50).expect("shrunk");

    assert_eq!(size_of(&small), (50, 25));
    assert_eq!(small.len(), HEADER + 50 * 25 * 4);
    assert!(is_color(&small[HEADER..], 1), "the colour is kept");
}

#[test]
fn shrinking_averages_the_pixels_it_joins() {
    // A chessboard of black and white pixels, shrunk to half: every pixel of the thumbnail is the grey between.
    let mut image = solid(64, 64, [0, 0, 0, 255]);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        if (x + y) % 2 == 0 {
            *pixel = image::Rgba([255, 255, 255, 255]);
        }
    }
    let picture = frame(64, 64, image.as_raw());

    let small = shrink(picture, 32).expect("shrunk");

    assert_eq!(size_of(&small), (32, 32));
    for pixel in small[HEADER..].as_chunks::<4>().0 {
        assert!(pixel[..3].iter().all(|&value| value.abs_diff(127) <= 4), "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }
}

#[test]
fn a_picture_that_fits_is_given_back_as_it_is() {
    let picture = frame(16, 8, solid(16, 8, COLOR).as_raw());

    assert_eq!(shrink(picture.clone(), 64).expect("kept"), picture);
}

#[test]
fn pixels_that_do_not_match_their_header_are_undecodable() {
    let good = frame(4, 4, solid(4, 4, COLOR).as_raw());

    let mut short = good.clone();
    short.pop();
    let mut long = good.clone();
    long.push(0);
    let mut empty = frame(0, 4, &[]);
    empty.truncate(HEADER);

    for bad in [short, long, empty, vec![1, 0, 0], vec![]] {
        let error = shrink(bad, 2).expect_err("refused");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(is_refusal(&error));
    }
}

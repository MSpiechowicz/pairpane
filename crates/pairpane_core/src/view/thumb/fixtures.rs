//! Pictures for the tests of the thumbnails, made by the encoders of the `image` crate or by hand.

use std::io::Cursor;

use image::{DynamicImage, Frame, ImageEncoder, ImageFormat, Rgba, RgbaImage};

/// The colour of the pictures: `#3080c0`, opaque.
pub(super) const COLOR: [u8; 4] = [0x30, 0x80, 0xC0, 0xFF];

/// A picture of one colour.
pub(super) fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(width, height, Rgba(color))
}

/// `width` by `height` pixels of [`COLOR`] in `format` (a JPEG has no alpha, so it is made from RGB).
pub(super) fn encode(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::ImageRgba8(solid(width, height, COLOR));
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(image.to_rgb8())
    } else {
        image
    };

    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).expect("encoded");

    out.into_inner()
}

/// A GIF of two frames, the first of `first` and the second of `second`.
pub(super) fn animated_gif(width: u32, height: u32, first: [u8; 4], second: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
        for color in [first, second] {
            encoder
                .encode_frame(Frame::new(solid(width, height, color)))
                .expect("a frame");
        }
    }

    out
}

/// A JPEG of `width` by `height` pixels of [`COLOR`] whose EXIF says it is to be shown with `orientation` (1 to 8).
pub(super) fn oriented_jpeg(width: u32, height: u32, orientation: u16) -> Vec<u8> {
    // A little endian TIFF header, an IFD of one entry (the orientation, a SHORT) and no next IFD.
    let mut exif = b"II*\0".to_vec();
    exif.extend_from_slice(&8_u32.to_le_bytes());
    exif.extend_from_slice(&1_u16.to_le_bytes());
    exif.extend_from_slice(&0x112_u16.to_le_bytes());
    exif.extend_from_slice(&3_u16.to_le_bytes());
    exif.extend_from_slice(&1_u32.to_le_bytes());
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0, 0]);
    exif.extend_from_slice(&0_u32.to_le_bytes());

    let pixels = solid(width, height, COLOR);
    let rgb = DynamicImage::ImageRgba8(pixels).to_rgb8();

    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut out);
    encoder.set_exif_metadata(exif).expect("exif");
    encoder
        .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .expect("encoded");

    out
}

/// A PNG that claims `width` by `height` pixels of `depth` bits and the colour type `color_type` and holds no pixels
/// (its image data is an empty zlib stream that is cut short): what a decompression bomb is made of.
pub(super) fn png_claiming(width: u32, height: u32, depth: u8, color_type: u8) -> Vec<u8> {
    let mut header = b"IHDR".to_vec();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // Depth, colour type, then the compression, the filter and the interlace method, all 0.
    header.extend_from_slice(&[depth, color_type, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(&header);
    png.extend_from_slice(&crc32fast::hash(&header).to_be_bytes());

    // The image data, two bytes (a zlib header), then the end.
    for (name, data) in [(b"IDAT", &[0x78, 0x01][..]), (b"IEND", &[][..])] {
        let mut chunk = name.to_vec();
        chunk.extend_from_slice(data);

        png.extend_from_slice(&u32::try_from(data.len()).expect("small").to_be_bytes());
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32fast::hash(&chunk).to_be_bytes());
    }

    png
}

/// Whether every pixel of `rgba` is [`COLOR`], give or take `tolerance` per channel (a JPEG is not exact).
pub(super) fn is_color(rgba: &[u8], tolerance: u8) -> bool {
    rgba.as_chunks::<4>().0.iter().all(|pixel| {
        pixel
            .iter()
            .zip(COLOR)
            .all(|(got, want)| got.abs_diff(want) <= tolerance)
    })
}

/// The width and the height in the header of an answer.
pub(super) fn size_of(answer: &[u8]) -> (u32, u32) {
    let width = u32::from_le_bytes(answer[0..4].try_into().expect("a width"));
    let height = u32::from_le_bytes(answer[4..8].try_into().expect("a height"));

    (width, height)
}

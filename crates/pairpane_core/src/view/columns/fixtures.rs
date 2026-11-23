//! Tiny pictures built byte by byte for the tests: just the headers, never a decodable picture.

use crate::view::heic::items_tests::boxed;

/// A full box (version and flags before the content).
fn full(kind: &[u8; 4], version: u8, flags: u8, content: &[u8]) -> Vec<u8> {
    let mut body = vec![version, 0, 0, flags];
    body.extend_from_slice(content);

    boxed(kind, &body)
}

pub(super) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    // Depth, colour type, compression, filter, interlace, and the checksum of the chunk.
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);

    bytes
}

pub(super) fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, b';']);

    bytes
}

/// The 14 bytes of the file header of a BMP.
fn bmp_file_header() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend_from_slice(&[0; 12]);

    bytes
}

/// A BMP with the 12-byte core header.
pub(super) fn bmp_core(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = bmp_file_header();
    bytes.extend_from_slice(&12_u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&[1, 0, 24, 0]);

    bytes
}

/// A BMP with the 40-byte info header (`height` is negative for rows stored top down).
pub(super) fn bmp_info(width: i32, height: i32) -> Vec<u8> {
    let mut bytes = bmp_file_header();
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&[0; 28]);

    bytes
}

/// A RIFF file of the form `WEBP` whose first chunk is `kind` with `data`.
fn webp(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&u32::try_from(12 + data.len()).expect("small").to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(&u32::try_from(data.len()).expect("small").to_le_bytes());
    bytes.extend_from_slice(data);

    bytes
}

/// A lossy WebP (`VP8 `): a key frame tag, the start code and the 14-bit sides (`scale` is the two bits above each).
pub(super) fn webp_lossy(width: u16, height: u16, scale: u16) -> Vec<u8> {
    let mut data = vec![0x10, 0x02, 0x00, 0x9D, 0x01, 0x2A];
    data.extend_from_slice(&(width | scale << 14).to_le_bytes());
    data.extend_from_slice(&(height | scale << 14).to_le_bytes());

    webp(b"VP8 ", &data)
}

/// A lossless WebP (`VP8L`): the signature byte and 14 bits each of width and height minus one.
pub(super) fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
    let bits = (width - 1) | (height - 1) << 14;
    let mut data = vec![0x2F];
    data.extend_from_slice(&bits.to_le_bytes());

    webp(b"VP8L", &data)
}

/// An extended WebP (`VP8X`): flags, and the canvas as 24-bit sides minus one.
pub(super) fn webp_extended(width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0x10, 0, 0, 0];
    data.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    data.extend_from_slice(&(height - 1).to_le_bytes()[..3]);

    webp(b"VP8X", &data)
}

/// A JPEG segment: the marker and its data, with the length that counts itself.
pub(super) fn segment(marker: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xFF, marker];
    bytes.extend_from_slice(&u16::try_from(2 + data.len()).expect("small").to_be_bytes());
    bytes.extend_from_slice(data);

    bytes
}

/// The frame header segment (`SOF0`) of a picture of 3 components.
pub(super) fn frame(width: u16, height: u16) -> Vec<u8> {
    let mut data = vec![8];
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);

    segment(0xC0, &data)
}

/// A JPEG of `segments` between the start of the image and the frame header, then the start of the scan.
pub(super) fn jpeg_of(segments: &[Vec<u8>], width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8];
    for segment in segments {
        bytes.extend_from_slice(segment);
    }
    bytes.extend_from_slice(&frame(width, height));
    bytes.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);

    bytes
}

pub(super) fn jpeg(width: u16, height: u16) -> Vec<u8> {
    jpeg_of(&[segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0")], width, height)
}

/// An `APP1` segment of EXIF whose first directory holds one entry, the orientation.
pub(super) fn exif(orientation: u16, big_endian: bool) -> Vec<u8> {
    let short = |value: u16| {
        if big_endian {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        }
    };
    let long = |value: u32| {
        if big_endian {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        }
    };

    let mut data = b"Exif\0\0".to_vec();
    data.extend_from_slice(if big_endian { b"MM" } else { b"II" });
    data.extend_from_slice(&short(42));
    data.extend_from_slice(&long(8));
    // One entry: the tag, the type (a 16-bit number), the count and the value in the first two bytes of the field.
    data.extend_from_slice(&short(1));
    data.extend_from_slice(&short(0x0112));
    data.extend_from_slice(&short(3));
    data.extend_from_slice(&long(1));
    data.extend_from_slice(&short(orientation));
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    segment(0xE1, &data)
}

/// The `irot` property: `quarter_turns` of 90 degrees.
pub(super) fn irot(quarter_turns: u8) -> Vec<u8> {
    boxed(b"irot", &[quarter_turns])
}

/// The `clap` property: a clean aperture of `width` by `height` pixels, centred (the offsets are zero).
pub(super) fn clap(width: u32, height: u32) -> Vec<u8> {
    let mut content = Vec::new();
    for (numerator, denominator) in [(width, 1_u32), (height, 1), (0, 1), (0, 1)] {
        content.extend_from_slice(&numerator.to_be_bytes());
        content.extend_from_slice(&denominator.to_be_bytes());
    }

    boxed(b"clap", &content)
}

/// A HEIC file of the primary picture `width` by `height` with the properties `transforms` after its size (in this
/// order): the `ftyp`, then a `meta` with a decoy item first (a 64 by 64 tile that the primary item does not have),
/// then the picture data. `wide` is the form with 32-bit item IDs and 16-bit property numbers.
pub(super) fn heif(width: u32, height: u32, transforms: &[Vec<u8>], wide: bool) -> Vec<u8> {
    let version = u8::from(wide);

    let ispe = |width: u32, height: u32| {
        let mut content = width.to_be_bytes().to_vec();
        content.extend_from_slice(&height.to_be_bytes());

        full(b"ispe", 0, 0, &content)
    };
    let mut ipco = ispe(64, 64);
    ipco.extend_from_slice(&ispe(width, height));
    for transform in transforms {
        ipco.extend_from_slice(transform);
    }

    // The property numbers: 1 is the tile's, 2 the primary's size (essential bit set), 3 and on its transforms.
    let essential = if wide { 0x8000 } else { 0x80 };
    let number = |number: u16| -> Vec<u8> {
        if wide {
            number.to_be_bytes().to_vec()
        } else {
            vec![u8::try_from(number).expect("a narrow number")]
        }
    };
    let id = |id: u16| -> Vec<u8> {
        if wide {
            u32::from(id).to_be_bytes().to_vec()
        } else {
            id.to_be_bytes().to_vec()
        }
    };

    let mut ipma = 2_u32.to_be_bytes().to_vec();
    ipma.extend_from_slice(&id(2));
    ipma.extend_from_slice(&[1]);
    ipma.extend_from_slice(&number(1));
    ipma.extend_from_slice(&id(1));
    ipma.extend_from_slice(&[u8::try_from(1 + transforms.len()).expect("few")]);
    ipma.extend_from_slice(&number(2 | essential));
    for index in 0..transforms.len() {
        ipma.extend_from_slice(&number(u16::try_from(3 + index).expect("few")));
    }

    let mut iprp = boxed(b"ipco", &ipco);
    iprp.extend_from_slice(&full(b"ipma", version, version, &ipma));

    let mut meta_content = full(b"pitm", version, 0, &id(1));
    meta_content.extend_from_slice(&boxed(b"iprp", &iprp));

    let mut bytes = boxed(b"ftyp", b"heic\0\0\0\0mif1heic");
    bytes.extend_from_slice(&full(b"meta", 0, 0, &meta_content));
    bytes.extend_from_slice(&boxed(b"mdat", &[0; 16]));

    bytes
}

/// Every format once, with its name: the starting points of the fuzz-like tests.
pub(super) fn all() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("png", png(640, 480)),
        ("jpeg", jpeg(640, 480)),
        ("jpeg with exif", jpeg_of(&[exif(6, false)], 640, 480)),
        ("gif", gif(640, 480)),
        ("bmp core", bmp_core(640, 480)),
        ("bmp info", bmp_info(640, -480)),
        ("webp lossy", webp_lossy(640, 480, 0)),
        ("webp lossless", webp_lossless(640, 480)),
        ("webp extended", webp_extended(640, 480)),
        ("heic", heif(640, 480, &[clap(600, 400), irot(1)], false)),
        ("heic wide", heif(640, 480, &[], true)),
    ]
}

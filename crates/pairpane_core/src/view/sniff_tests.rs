//! What a file is, by its first bytes and its name.

use pretty_assertions::assert_eq;

use crate::model::{ArchiveFormat, ViewKind};

use super::sniff::sniff;

fn media(kind: &str, mime: &str) -> ViewKind {
    let mime = mime.to_owned();

    match kind {
        "image" => ViewKind::Image { mime },
        "audio" => ViewKind::Audio { mime },
        _ => ViewKind::Video { mime },
    }
}

fn image(mime: &str) -> ViewKind {
    media("image", mime)
}

fn audio(mime: &str) -> ViewKind {
    media("audio", mime)
}

fn video(mime: &str) -> ViewKind {
    media("video", mime)
}

fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
    [&[0, 0, 0, 0x18], b"ftyp".as_slice(), brand.as_slice(), &[0; 16]].concat()
}

fn riff(form: &[u8; 4]) -> Vec<u8> {
    [b"RIFF".as_slice(), &[0x24, 0, 0, 0], form.as_slice(), &[0; 16]].concat()
}

fn bmp() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 32]);

    bytes
}

fn tar_header() -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[257..262].copy_from_slice(b"ustar");

    bytes
}

#[test]
fn the_magic_number_wins_over_the_extension() {
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice();

    assert_eq!(sniff(png, "photo.txt"), image("image/png"));
    assert_eq!(sniff(png, "noext"), image("image/png"));
    assert_eq!(sniff(b"plain words", "fake.png"), ViewKind::Text);
}

#[test]
fn recognises_the_formats_by_their_magic() {
    let table: Vec<(&str, Vec<u8>, ViewKind)> = vec![
        ("a.jpg", vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10], image("image/jpeg")),
        ("a.gif", b"GIF89a\x01\0\x01\0".to_vec(), image("image/gif")),
        ("a.gif", b"GIF87a\x01\0\x01\0".to_vec(), image("image/gif")),
        ("a.webp", riff(b"WEBP"), image("image/webp")),
        ("a.bmp", bmp(), image("image/bmp")),
        ("a.ico", vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0], image("image/x-icon")),
        ("a.avif", ftyp(b"avif"), image("image/avif")),
        ("a.heic", ftyp(b"heic"), image("image/heic")),
        ("a.heic", ftyp(b"mif1"), image("image/heic")),
        ("a.pdf", b"%PDF-1.7\n".to_vec(), ViewKind::Pdf),
        ("a.mp4", ftyp(b"isom"), video("video/mp4")),
        ("a.mp4", ftyp(b"mp42"), video("video/mp4")),
        ("a.mov", ftyp(b"qt  "), video("video/quicktime")),
        ("a.m4a", ftyp(b"M4A "), audio("audio/mp4")),
        (
            "a.webm",
            [&[0x1A, 0x45, 0xDF, 0xA3][..], &[0x42, 0x82, 0x84], b"webm"].concat(),
            video("video/webm"),
        ),
        (
            "a.mkv",
            [&[0x1A, 0x45, 0xDF, 0xA3][..], &[0x42, 0x82, 0x88], b"matroska"].concat(),
            video("video/x-matroska"),
        ),
        ("a.ogg", b"OggS\0\x02".to_vec(), audio("audio/ogg")),
        ("a.ogv", b"OggS\0\x02".to_vec(), video("video/ogg")),
        ("a.mp3", b"ID3\x04\0\0".to_vec(), audio("audio/mpeg")),
        ("a.mp3", vec![0xFF, 0xFB, 0x90, 0x44], audio("audio/mpeg")),
        ("a.flac", b"fLaC\0\0\0\x22".to_vec(), audio("audio/flac")),
        ("a.wav", riff(b"WAVE"), audio("audio/wav")),
        (
            "a.zip",
            b"PK\x03\x04\x14\0".to_vec(),
            ViewKind::Archive {
                format: ArchiveFormat::Zip,
            },
        ),
        (
            "empty.zip",
            b"PK\x05\x06\0\0".to_vec(),
            ViewKind::Archive {
                format: ArchiveFormat::Zip,
            },
        ),
        (
            "a.tar",
            tar_header(),
            ViewKind::Archive {
                format: ArchiveFormat::Tar,
            },
        ),
        (
            "a.tar.gz",
            vec![0x1F, 0x8B, 8, 0],
            ViewKind::Archive {
                format: ArchiveFormat::TarGz,
            },
        ),
        (
            "A.TGZ",
            vec![0x1F, 0x8B, 8, 0],
            ViewKind::Archive {
                format: ArchiveFormat::TarGz,
            },
        ),
    ];

    for (name, head, expected) in table {
        assert_eq!(sniff(&head, name), expected, "{name}");
    }
}

#[test]
fn a_gzip_file_that_is_not_a_tar_is_bytes() {
    assert_eq!(sniff(&[0x1F, 0x8B, 8, 0], "log.gz"), ViewKind::Binary);
}

#[test]
fn svg_needs_both_its_extension_and_its_element() {
    let svg = b"<?xml version=\"1.0\"?>\n<!-- c -->\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";

    assert_eq!(sniff(svg, "logo.svg"), image("image/svg+xml"));
    assert_eq!(sniff(svg, "LOGO.SVG"), image("image/svg+xml"));
    assert_eq!(sniff(svg, "logo.xml"), ViewKind::Text);
    assert_eq!(sniff(b"<html></html>", "logo.svg"), ViewKind::Text);
}

#[test]
fn markdown_by_extension_when_it_is_text() {
    assert_eq!(sniff(b"# title", "README.md"), ViewKind::Markdown);
    assert_eq!(sniff(b"# title", "notes.markdown"), ViewKind::Markdown);
    assert_eq!(sniff(b"# title", "notes.txt"), ViewKind::Text);
    assert_eq!(sniff(b"# ti\0tle", "notes.md"), ViewKind::Binary);
}

#[test]
fn a_nul_byte_makes_binary_but_utf16_text_is_text() {
    assert_eq!(sniff(&[1, 2, 0, 3], "x.dat"), ViewKind::Binary);
    assert_eq!(sniff(&[], "empty.txt"), ViewKind::Text);

    let units: Vec<u8> = "hello, wörld\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(sniff(&units, "x.txt"), ViewKind::Text);

    // The byte order mark of UTF-16 is not the frame header of an MP3.
    assert_eq!(sniff(&[0xFF, 0xFE, b'a', 0], "x.txt"), ViewKind::Text);
}

#[test]
fn text_that_starts_like_a_magic_number_is_still_text() {
    assert_eq!(sniff(b"BM is a company", "notes.txt"), ViewKind::Text);
    assert_eq!(sniff(b"PKG info", "notes.txt"), ViewKind::Text);
    assert_eq!(sniff(b"ID", "notes.txt"), ViewKind::Text);
}

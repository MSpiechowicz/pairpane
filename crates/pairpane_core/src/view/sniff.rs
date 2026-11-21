//! What a file is, from its first bytes: the magic number wins over the extension, so a `.txt` that holds a PNG is
//! shown as an image. The extension decides only what no magic number can: Markdown and SVG.

use crate::model::{ArchiveFormat, ViewKind};

use super::encoding;

/// How much of the start is looked at for a NUL byte.
const NUL_SCAN: usize = 8 * 1024;

/// The kind of the file called `name` whose first bytes are `head`.
pub(super) fn sniff(head: &[u8], name: &str) -> ViewKind {
    let name = name.to_ascii_lowercase();

    if let Some(kind) = by_magic(head, &name) {
        return kind;
    }

    if name.ends_with(".svg") && looks_like_svg(head) {
        return image("image/svg+xml");
    }

    // UTF-16 text is full of zero bytes; anything else with a NUL byte is not text.
    if encoding::utf16(head).is_none() && head[..head.len().min(NUL_SCAN)].contains(&0) {
        return ViewKind::Binary;
    }

    if name.ends_with(".md") || name.ends_with(".markdown") {
        return ViewKind::Markdown;
    }

    ViewKind::Text
}

fn image(mime: &str) -> ViewKind {
    ViewKind::Image { mime: mime.to_owned() }
}

fn audio(mime: &str) -> ViewKind {
    ViewKind::Audio { mime: mime.to_owned() }
}

fn video(mime: &str) -> ViewKind {
    ViewKind::Video { mime: mime.to_owned() }
}

fn by_magic(head: &[u8], name: &str) -> Option<ViewKind> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(image("image/png"));
    }

    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(image("image/jpeg"));
    }

    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(image("image/gif"));
    }

    if is_bmp(head) {
        return Some(image("image/bmp"));
    }

    if is_ico(head) {
        return Some(image("image/x-icon"));
    }

    if head.starts_with(b"%PDF-") {
        return Some(ViewKind::Pdf);
    }

    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some(ViewKind::Archive {
            format: ArchiveFormat::Zip,
        });
    }

    if head.starts_with(&[0x1F, 0x8B]) {
        return Some(gzip(name));
    }

    if head.len() >= 262 && &head[257..262] == b"ustar" {
        return Some(ViewKind::Archive {
            format: ArchiveFormat::Tar,
        });
    }

    riff(head)
        .or_else(|| iso_media(head))
        .or_else(|| sound_or_video(head, name))
}

/// A gzip file is listed only when its name says it holds a tar; any other one is bytes.
fn gzip(name: &str) -> ViewKind {
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        return ViewKind::Archive {
            format: ArchiveFormat::TarGz,
        };
    }

    ViewKind::Binary
}

/// `BM`, then the file header and the size of the info header that follows it (the sizes the versions of the
/// format use), which a text that starts with `BM` does not have.
fn is_bmp(head: &[u8]) -> bool {
    if head.len() < 18 || !head.starts_with(b"BM") {
        return false;
    }

    let info = u32::from_le_bytes([head[14], head[15], head[16], head[17]]);

    matches!(info, 12 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// An icon directory: reserved zero, type 1, at least one image, and a zero in the reserved byte of the first entry.
fn is_ico(head: &[u8]) -> bool {
    head.len() >= 10 && head.starts_with(&[0, 0, 1, 0]) && (head[4] != 0 || head[5] != 0) && head[9] == 0
}

/// WebP and WAVE: `RIFF`, a size, then the form.
fn riff(head: &[u8]) -> Option<ViewKind> {
    if head.len() < 12 || !head.starts_with(b"RIFF") {
        return None;
    }

    match &head[8..12] {
        b"WEBP" => Some(image("image/webp")),
        b"WAVE" => Some(audio("audio/wav")),
        _ => None,
    }
}

/// The ISO base media files (`ftyp` box): the major brand tells a picture from a movie or a sound.
fn iso_media(head: &[u8]) -> Option<ViewKind> {
    if head.len() < 12 || &head[4..8] != b"ftyp" {
        return None;
    }

    let kind = match &head[8..12] {
        b"avif" | b"avis" => image("image/avif"),
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => image("image/heic"),
        b"qt  " => video("video/quicktime"),
        b"M4A " | b"M4B " => audio("audio/mp4"),
        _ => video("video/mp4"),
    };

    Some(kind)
}

/// Matroska and WebM (EBML), Ogg, MP3, FLAC.
fn sound_or_video(head: &[u8], name: &str) -> Option<ViewKind> {
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        let doctype = &head[..head.len().min(64)];

        if doctype.windows(4).any(|window| window == b"webm") {
            return Some(video("video/webm"));
        }

        return Some(video("video/x-matroska"));
    }

    if head.starts_with(b"OggS") {
        return Some(if name.ends_with(".ogv") {
            video("video/ogg")
        } else {
            audio("audio/ogg")
        });
    }

    if head.starts_with(b"fLaC") {
        return Some(audio("audio/flac"));
    }

    // A frame header of MPEG audio layer 3 (version 1 or 2, with or without CRC), or an ID3 tag. The byte order mark
    // of UTF-16 (`FF FE`) is not one.
    if head.starts_with(b"ID3") || matches!(head, [0xFF, 0xFB | 0xFA | 0xF3 | 0xF2, ..]) {
        return Some(audio("audio/mpeg"));
    }

    None
}

/// An SVG file: an `<svg` element near the start (after an XML declaration, comments and the doctype).
fn looks_like_svg(head: &[u8]) -> bool {
    let start = &head[..head.len().min(4096)];

    start.windows(4).any(|window| window.eq_ignore_ascii_case(b"<svg"))
}

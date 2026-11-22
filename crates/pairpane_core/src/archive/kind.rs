//! Which archive format a file is. A pane goes by the name first and checks the first bytes ([`detect`]); the viewer
//! goes by the bytes first ([`by_magic`]), as it does for every other kind of file.

use crate::model::ArchiveFormat;

use super::tar::walk::header_is_valid;

/// How many bytes of the start of a file [`detect`] needs: a tar header block.
pub const HEAD_BYTES: usize = 512;

/// The compression a name or a file's first bytes say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Magic {
    Zip,
    Gzip,
    Xz,
    Zstd,
    Bzip2,
    SevenZ,
    Rar,
    Tar,
}

fn magic(head: &[u8]) -> Option<Magic> {
    let starts = |prefix: &[u8]| head.starts_with(prefix);

    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") || starts(b"PK\x07\x08") {
        Some(Magic::Zip)
    } else if starts(&[0x1F, 0x8B]) {
        Some(Magic::Gzip)
    } else if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        Some(Magic::Xz)
    } else if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Some(Magic::Zstd)
    } else if starts(b"BZh") && head.get(3).is_some_and(|level| (b'1'..=b'9').contains(level)) {
        Some(Magic::Bzip2)
    } else if starts(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        Some(Magic::SevenZ)
    } else if starts(b"Rar!\x1A\x07\x00") || starts(b"Rar!\x1A\x07\x01\x00") {
        Some(Magic::Rar)
    } else if head.len() >= 262 && &head[257..262] == b"ustar" {
        Some(Magic::Tar)
    } else {
        None
    }
}

/// The format of the file called `name` (any case) from its extension alone: `.zip` and `.jar`, `.tar`, `.tar.gz` and
/// `.tgz`, `.tar.xz` and `.txz`, `.tar.zst` and `.tzst`, `.tar.bz2`, `.tbz2` and `.tbz`, `.7z`, `.rar`, and `.gz` that
/// is not a tar. A single `.xz`, `.zst` or `.bz2` file is not an archive here.
pub fn by_extension(name: &str) -> Option<ArchiveFormat> {
    let name = name.to_ascii_lowercase();
    let ends = |suffixes: &[&str]| suffixes.iter().any(|suffix| name.ends_with(suffix));

    if ends(&[".zip", ".jar"]) {
        Some(ArchiveFormat::Zip)
    } else if ends(&[".tar"]) {
        Some(ArchiveFormat::Tar)
    } else if ends(&[".tar.gz", ".tgz"]) {
        Some(ArchiveFormat::TarGz)
    } else if ends(&[".tar.xz", ".txz"]) {
        Some(ArchiveFormat::TarXz)
    } else if ends(&[".tar.zst", ".tzst"]) {
        Some(ArchiveFormat::TarZst)
    } else if ends(&[".tar.bz2", ".tbz2", ".tbz"]) {
        Some(ArchiveFormat::TarBz2)
    } else if ends(&[".7z"]) {
        Some(ArchiveFormat::SevenZ)
    } else if ends(&[".rar"]) {
        Some(ArchiveFormat::Rar)
    } else if ends(&[".gz"]) {
        Some(ArchiveFormat::Gz)
    } else {
        None
    }
}

/// The format of the file called `name` whose first bytes are `head` (at least [`HEAD_BYTES`] of them, unless the file
/// is shorter), when the extension names a format and the bytes agree: a name is no proof, so a `.zip` that is not a
/// zip is not an archive. A tar is checked by its header block, whether it says `ustar` or is an old one with a
/// right checksum.
pub fn detect(name: &str, head: &[u8]) -> Option<ArchiveFormat> {
    let format = by_extension(name)?;
    let found = magic(head);

    let agrees = match format {
        ArchiveFormat::Zip => found == Some(Magic::Zip),
        ArchiveFormat::Tar => found == Some(Magic::Tar) || header_is_valid(head),
        ArchiveFormat::TarGz | ArchiveFormat::Gz => found == Some(Magic::Gzip),
        ArchiveFormat::TarXz => found == Some(Magic::Xz),
        ArchiveFormat::TarZst => found == Some(Magic::Zstd),
        ArchiveFormat::TarBz2 => found == Some(Magic::Bzip2),
        ArchiveFormat::SevenZ => found == Some(Magic::SevenZ),
        ArchiveFormat::Rar => found == Some(Magic::Rar),
    };

    agrees.then_some(format)
}

/// The format of a file from its first bytes, the name only telling a tar inside a compression from a lone compressed
/// file: a zip, a tar (by `ustar`), a 7z or a RAR by their bytes; a gzip, XZ, Zstandard or bzip2 file only when its name
/// says it holds a tar (a single compressed file is not an archive here, apart from `.gz`, see [`by_extension`]).
pub fn by_magic(head: &[u8], name: &str) -> Option<ArchiveFormat> {
    let compressed_tar = by_extension(name);

    match magic(head)? {
        Magic::Zip => Some(ArchiveFormat::Zip),
        Magic::Tar => Some(ArchiveFormat::Tar),
        Magic::SevenZ => Some(ArchiveFormat::SevenZ),
        Magic::Rar => Some(ArchiveFormat::Rar),
        Magic::Gzip => compressed_tar.filter(|format| matches!(format, ArchiveFormat::TarGz | ArchiveFormat::Gz)),
        Magic::Xz => compressed_tar.filter(|format| *format == ArchiveFormat::TarXz),
        Magic::Zstd => compressed_tar.filter(|format| *format == ArchiveFormat::TarZst),
        Magic::Bzip2 => compressed_tar.filter(|format| *format == ArchiveFormat::TarBz2),
    }
}

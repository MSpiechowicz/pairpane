//! Checksums and the checksum file: the three hashes behind one type, and the format of the file (the one of
//! `sha256sum` and `md5sum`: `<hex>  <path>` per line, a path with a backslash or a line break written escaped).

use md5::Md5;
use sha2::{Digest, Sha256};

use crate::listing::platform;
use crate::model::HashAlgorithm;

/// A hash being computed.
pub(super) enum Digester {
    Sha256(Sha256),
    Blake3(Box<blake3::Hasher>),
    Md5(Md5),
}

impl Digester {
    pub(super) fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Sha256 => Self::Sha256(Sha256::new()),
            HashAlgorithm::Blake3 => Self::Blake3(Box::new(blake3::Hasher::new())),
            HashAlgorithm::Md5 => Self::Md5(Md5::new()),
        }
    }

    pub(super) fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(bytes),
            Self::Blake3(hasher) => {
                hasher.update(bytes);
            }
            Self::Md5(hasher) => hasher.update(bytes),
        }
    }

    /// The hash as lowercase hex.
    pub(super) fn finish(self) -> String {
        match self {
            Self::Sha256(hasher) => hex(&hasher.finalize()),
            Self::Blake3(hasher) => hasher.finalize().to_hex().to_string(),
            Self::Md5(hasher) => hex(&hasher.finalize()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");

            text
        })
}

/// The extension a checksum file of `algorithm` gets (without the dot).
#[cfg(test)]
pub(super) fn extension(algorithm: HashAlgorithm) -> &'static str {
    match algorithm {
        HashAlgorithm::Sha256 => "sha256",
        HashAlgorithm::Blake3 => "blake3",
        HashAlgorithm::Md5 => "md5",
    }
}

/// The length of the hex form of a hash of `algorithm`.
pub(super) fn hex_len(algorithm: HashAlgorithm) -> usize {
    match algorithm {
        HashAlgorithm::Md5 => 32,
        HashAlgorithm::Sha256 | HashAlgorithm::Blake3 => 64,
    }
}

/// The algorithm a checksum file's name says it uses, by its extension (case does not matter).
pub(super) fn algorithm_of_name(name: &str) -> Option<HashAlgorithm> {
    let (_, extension) = name.rsplit_once('.')?;

    match extension.to_ascii_lowercase().as_str() {
        "sha256" | "sha256sum" => Some(HashAlgorithm::Sha256),
        "blake3" | "b3" => Some(HashAlgorithm::Blake3),
        "md5" | "md5sum" => Some(HashAlgorithm::Md5),
        _ => None,
    }
}

/// The algorithm a hash of `len` hex digits belongs to, when the name does not say: 32 is MD5, 64 is SHA-256.
pub(super) fn algorithm_of_length(len: usize) -> Option<HashAlgorithm> {
    match len {
        32 => Some(HashAlgorithm::Md5),
        64 => Some(HashAlgorithm::Sha256),
        _ => None,
    }
}

/// One line of a checksum file: the hash, and the path it is for, relative to the folder of the file.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Listed {
    /// Lowercase hex.
    pub(super) hex: String,
    pub(super) path: String,
}

/// The line for the file at `path` (`/` between the names) with the hash `hex`, ending in a line feed.
pub(super) fn format_line(hex: &str, path: &str) -> String {
    let escaped = path.contains(['\\', '\n', '\r']);

    if !escaped {
        return format!("{hex}  {path}\n");
    }

    let path = path.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r");

    format!("\\{hex}  {path}\n")
}

/// Reads one line of a checksum file (without its line break). `None` when it is not one: no hex, no path, a mark that
/// is neither a space nor `*` (the binary mode), or an escape that is not `\\`, `\n` or `\r`.
pub(super) fn parse_line(line: &str) -> Option<Listed> {
    let (escaped, rest) = match line.strip_prefix('\\') {
        Some(rest) => (true, rest),
        None => (false, line),
    };

    let (hex, rest) = rest.split_once(' ')?;
    if hex.is_empty() || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    let path = rest.strip_prefix([' ', '*'])?;
    if path.is_empty() {
        return None;
    }

    let path = if escaped { unescape(path)? } else { path.to_owned() };

    Some(Listed {
        hex: hex.to_ascii_lowercase(),
        path,
    })
}

fn unescape(text: &str) -> Option<String> {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(char) = chars.next() {
        if char != '\\' {
            plain.push(char);
            continue;
        }

        match chars.next()? {
            '\\' => plain.push('\\'),
            'n' => plain.push('\n'),
            'r' => plain.push('\r'),
            _ => return None,
        }
    }

    Some(plain)
}

/// Whether `path`, as a checksum file lists it, stays inside the folder of the file: names of plain files joined by `/`,
/// nothing absolute, nothing that climbs.
pub(super) fn stays_inside(path: &str) -> bool {
    path.split('/').all(platform::is_plain_name)
}

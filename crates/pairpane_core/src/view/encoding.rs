//! Text encodings: detecting one from the start of a file and finding one by its label.

use std::io;

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

use super::refusal;

/// The fewest pairs of bytes (16 bytes) that make a guess of UTF-16 without a byte order mark.
const MIN_PAIRS: usize = 8;

/// The encoding with this label (`UTF-8`, `windows-1250`, `latin1`). The labels of the replacement encodings
/// (ISO-2022-KR, HZ-GB-2312 and the like, which decode to nothing useful) are refused with the unknown ones.
pub(super) fn by_name(label: &str) -> io::Result<&'static Encoding> {
    Encoding::for_label_no_replacement(label.trim().as_bytes())
        .ok_or_else(|| refusal(io::ErrorKind::InvalidInput, "the encoding is not known"))
}

/// Whether [`by_name`] knows `label`; for a caller that checks an argument before it does the work.
pub fn is_known_encoding(label: &str) -> bool {
    by_name(label).is_ok()
}

/// The length of the byte order mark of `encoding` at the start of `head`: only a mark that belongs to that
/// encoding counts, so a file read as another encoding shows the mark's bytes as they are.
pub(super) fn bom_len(head: &[u8], encoding: &'static Encoding) -> usize {
    let (found, length) = match Encoding::for_bom(head) {
        Some(found) => found,
        None => return 0,
    };

    if found == encoding { length } else { 0 }
}

/// UTF-16 by its byte order mark or, without one, by the zero bytes of mostly Latin text: where one byte of most
/// pairs is zero, and always the same one of the two.
pub(super) fn utf16(sample: &[u8]) -> Option<&'static Encoding> {
    if let Some((encoding, _)) = Encoding::for_bom(sample)
        && encoding != UTF_8
    {
        return Some(encoding);
    }

    let pairs = sample.len() / 2;
    if pairs < MIN_PAIRS {
        return None;
    }

    let side = |parity: usize| sample.iter().skip(parity).step_by(2).copied();
    let zeros = |parity: usize| side(parity).filter(|byte| *byte == 0).count();
    let readable = |parity: usize| {
        side(parity)
            .filter(|byte| matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7E))
            .count()
    };

    // A fifth of the pairs have a zero byte on one side, at most one in fifty on the other, and three in four of the
    // bytes beside the zeros are characters of ASCII: a binary file with zeros in a pattern is not text.
    let heavy = pairs.div_ceil(5);
    let light = pairs / 50;
    let text = pairs - pairs / 4;

    if zeros(1) >= heavy && zeros(0) <= light && readable(0) >= text {
        return Some(UTF_16LE);
    }

    if zeros(0) >= heavy && zeros(1) <= light && readable(1) >= text {
        return Some(UTF_16BE);
    }

    None
}

/// The encoding of a file from its first bytes: a byte order mark, UTF-16 without one, valid UTF-8 (a character cut
/// at the end of the sample is not an error) and otherwise the guess of chardetng. `complete` says the sample is the
/// whole file.
pub(super) fn detect(sample: &[u8], complete: bool) -> &'static Encoding {
    if let Some(found) = utf16(sample) {
        return found;
    }

    let text = &sample[bom_len(sample, UTF_8)..];

    let utf8 = match std::str::from_utf8(text) {
        Ok(_) => true,
        Err(error) => error.error_len().is_none() && !complete,
    };
    if utf8 {
        return UTF_8;
    }

    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(text, complete);

    detector.guess(None, Utf8Detection::Allow)
}

/// Whether `encoding` stores a character in two-byte units.
pub(super) fn is_utf16(encoding: &'static Encoding) -> bool {
    encoding == UTF_16LE || encoding == UTF_16BE
}

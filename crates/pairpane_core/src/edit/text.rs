//! Text of the editor: reading bytes as text without replacing anything, the line breaks of a text, and writing a text
//! back as bytes in the encoding the user chose.

use std::io;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

use crate::model::{EditSaveOptions, LineEnding};
use crate::view::{bom_len, by_name, detect, is_utf16, refusal};

use super::MAX_SAVE_TEXT;

/// What the bytes of a file are.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decoded {
    /// Valid text in `encoding`; `bom` says the bytes start with its byte order mark (which is not in `text`).
    Text {
        text: String,
        encoding: &'static Encoding,
        bom: bool,
    },
    /// A NUL byte in an encoding that never writes one: not text.
    Binary,
    /// Not valid in `encoding`: it is never read with replacement characters.
    Undecodable(&'static Encoding),
}

/// Reads `bytes` as text in the encoding labelled `label`, or in the one detected from all of them. The line breaks
/// are left as they are.
pub(super) fn decode(bytes: &[u8], label: Option<&str>) -> io::Result<Decoded> {
    let encoding = match label {
        Some(label) => by_name(label)?,
        None => detect(bytes, true),
    };

    if !is_utf16(encoding) && bytes.contains(&0) {
        return Ok(Decoded::Binary);
    }

    let mark = bom_len(bytes, encoding);

    match encoding.decode_without_bom_handling_and_without_replacement(&bytes[mark..]) {
        Some(text) => Ok(Decoded::Text {
            text: text.into_owned(),
            encoding,
            bom: mark > 0,
        }),
        None => Ok(Decoded::Undecodable(encoding)),
    }
}

/// The line breaks of a text.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Breaks {
    /// The most common one (`Lf` for a text with none, and when two are equally common, the first of `\n`, `\r\n`, `\r`).
    pub ending: LineEnding,
    /// Whether more than one kind is used.
    pub mixed: bool,
}

/// Counts the line breaks of `text`.
pub(super) fn breaks(text: &str) -> Breaks {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let cr = text.matches('\r').count() - crlf;

    let counts = [(LineEnding::Lf, lf), (LineEnding::Crlf, crlf), (LineEnding::Cr, cr)];
    let mut ending = LineEnding::Lf;
    let mut most = 0;
    for (kind, count) in counts {
        if count > most {
            ending = kind;
            most = count;
        }
    }

    Breaks {
        ending,
        mixed: counts.iter().filter(|(_, count)| *count > 0).count() > 1,
    }
}

/// `text` with every line break as `\n`.
pub(super) fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn separator(ending: LineEnding) -> &'static str {
    match ending {
        LineEnding::Lf => "\n",
        LineEnding::Crlf => "\r\n",
        LineEnding::Cr => "\r",
    }
}

/// Whether `encoding` has a byte order mark.
fn has_mark(encoding: &'static Encoding) -> bool {
    encoding == UTF_8 || is_utf16(encoding)
}

/// The bytes to write for `text` as `options` say: every line break (`\r\n`, `\r` and `\n`) as the chosen one, in the
/// chosen encoding, after its byte order mark when asked for. A text too large, an encoding that is not known, a mark
/// for an encoding that has none and a character the encoding cannot hold are refused with a text that holds none of
/// the file.
pub(super) fn encode(text: &str, options: &EditSaveOptions) -> io::Result<Vec<u8>> {
    if text.len() > MAX_SAVE_TEXT {
        return Err(refusal(io::ErrorKind::InvalidInput, "the text is too large to save"));
    }

    let encoding = by_name(&options.encoding)?;
    if options.bom && !has_mark(encoding) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "a byte order mark belongs to UTF-8 and UTF-16 only",
        ));
    }

    let normal = normalise(text);
    let lines = match options.line_ending {
        LineEnding::Lf => normal,
        other => normal.replace('\n', separator(other)),
    };

    let mut bytes = Vec::with_capacity(lines.len() + 3);
    if options.bom {
        bytes.extend_from_slice(mark(encoding));
    }

    if is_utf16(encoding) {
        // The encoder of the web writes UTF-8 for UTF-16, so the units are written by hand.
        for unit in lines.encode_utf16() {
            let pair = if encoding == UTF_16LE {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            bytes.extend_from_slice(&pair);
        }

        return Ok(bytes);
    }

    let (encoded, _, unmappable) = encoding.encode(&lines);
    if unmappable {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "some characters cannot be saved in this encoding",
        ));
    }

    bytes.extend_from_slice(&encoded);

    Ok(bytes)
}

/// The byte order mark of `encoding`.
fn mark(encoding: &'static Encoding) -> &'static [u8] {
    if encoding == UTF_16LE {
        &[0xFF, 0xFE]
    } else if encoding == UTF_16BE {
        &[0xFE, 0xFF]
    } else {
        &[0xEF, 0xBB, 0xBF]
    }
}

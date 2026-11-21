//! Windows of text: the lines of a file from or up to a byte offset, in any encoding, without reading the rest.

use std::io;

use crate::model::{TextLine, TextQuery, TextWindow};

use super::file::ViewFile;
use super::layout::Layout;
use super::{MAX_SEGMENT, MAX_TEXT_LINES, MAX_TEXT_SCAN, encoding};

/// A line of the buffer, before it is cut into pieces and decoded.
#[derive(Debug, Clone, Copy)]
struct Line {
    from: usize,
    /// The end of the text, before the line ending.
    to: usize,
    /// Where the next line starts.
    next: usize,
    /// A line ending follows (and a carriage return before it is not text).
    ended: bool,
    /// The line goes on past the end of the buffer.
    partial: bool,
}

/// A part of a line of at most [`MAX_SEGMENT`] bytes.
#[derive(Debug, Clone, Copy)]
struct Piece {
    from: usize,
    to: usize,
    continues: bool,
}

/// The window of `query` in `file`, at most [`MAX_TEXT_LINES`] lines, reading at most [`MAX_TEXT_SCAN`] bytes (and
/// [`MAX_SNAP`](super::MAX_SNAP) more to find the start of a line).
///
/// A line longer than [`MAX_SEGMENT`] bytes comes back in pieces, all but the last with `continues` set; a piece
/// counts as a line. A window that would end in the middle of a line because the read ended there ends there, and
/// the next window starts where it ended. Lines end at a line feed (a carriage return before it is dropped) in the
/// encoding the query names; invalid bytes show as `�`.
pub fn text_window(file: &ViewFile, query: &TextQuery) -> io::Result<TextWindow> {
    let layout = Layout::new(file, encoding::by_name(&query.encoding)?)?;
    let size = file.size()?;
    let wanted = (query.lines as usize).clamp(1, MAX_TEXT_LINES);
    let offset = layout.place(file, query.offset, query.snap, size)?;

    let (base, buf, eof) = if query.backward {
        // The scan limit is even, so UTF-16 stays on its units.
        let from = offset.saturating_sub(MAX_TEXT_SCAN as u64).max(layout.floor);
        (from, file.read_at(from, (offset - from) as usize)?, offset >= size)
    } else {
        let len = MAX_TEXT_SCAN.min(usize::try_from(size.saturating_sub(offset)).unwrap_or(usize::MAX));
        let buf = file.read_at(offset, len)?;
        let eof = offset + buf.len() as u64 >= size;

        (offset, buf, eof)
    };

    let (pieces, end) = if query.backward {
        backward(&layout, &buf, base, base == layout.floor, eof, wanted)
    } else {
        forward(&layout, &buf, base, eof, wanted)
    };

    let start = pieces
        .first()
        .map_or(base + end as u64, |piece| base + piece.from as u64);
    let lines = pieces
        .iter()
        .map(|piece| TextLine {
            offset: base + piece.from as u64,
            text: layout
                .encoding
                .decode_without_bom_handling(&buf[piece.from..piece.to])
                .0
                .into_owned(),
            continues: piece.continues,
        })
        .collect();

    Ok(TextWindow {
        start,
        end: base + end as u64,
        lines,
        at_start: start <= layout.floor,
        at_end: base + end as u64 >= size,
    })
}

/// The pieces of the first `wanted` lines of `buf`, and the index where the next window starts.
fn forward(layout: &Layout, buf: &[u8], base: u64, eof: bool, wanted: usize) -> (Vec<Piece>, usize) {
    let mut lines = Vec::new();
    let mut estimate = 0;
    let mut at = 0;
    let mut newlines = layout.newlines(buf, base);

    while at < buf.len() && estimate < wanted {
        let line = match newlines.find(|&found| found >= at) {
            Some(found) => Line {
                from: at,
                to: found,
                next: found + layout.width(),
                ended: true,
                partial: false,
            },
            None if eof => Line {
                from: at,
                to: buf.len(),
                next: buf.len(),
                ended: false,
                partial: false,
            },
            None => {
                let cut = layout.cut(buf, at, buf.len());
                if cut <= at {
                    break;
                }

                Line {
                    from: at,
                    to: cut,
                    next: cut,
                    ended: false,
                    partial: true,
                }
            }
        };

        estimate += pieces_in(line.to - line.from);
        at = line.next;
        lines.push(line);
    }

    let mut pieces = Vec::new();
    for line in &lines {
        split(layout, buf, line, &mut pieces);
    }

    if pieces.len() > wanted {
        let end = pieces[wanted].from;
        pieces.truncate(wanted);

        return (pieces, end);
    }

    (pieces, lines.last().map_or(0, |line| line.next))
}

/// The pieces of the last `wanted` lines before the end of `buf`, and that end. `at_floor`: `buf` starts at the
/// start of the text, so its first line is a whole one; otherwise it is dropped (it may start mid-line) unless it is
/// all there is. `eof`: the end of `buf` is the end of the file, so its last line is whole even without a line
/// ending; otherwise a last line that is not ended goes on in the file.
fn backward(layout: &Layout, buf: &[u8], base: u64, at_floor: bool, eof: bool, wanted: usize) -> (Vec<Piece>, usize) {
    if buf.is_empty() {
        return (Vec::new(), 0);
    }

    let width = layout.width();
    let ended = layout
        .newlines(buf, base)
        .next_back()
        .is_some_and(|last| last + width == buf.len());
    let content = if ended { buf.len() - width } else { buf.len() };
    let partial = !ended && !eof;

    let mut lines = Vec::new();
    let mut estimate = 0;
    let mut line_end = content;

    for found in layout.newlines(&buf[..content], base).rev() {
        if estimate >= wanted {
            break;
        }

        let line = Line {
            from: found + width,
            to: line_end,
            next: line_end,
            ended: line_end < content || ended,
            partial: partial && line_end == content,
        };
        estimate += pieces_in(line.to - line.from);
        line_end = found;
        lines.push(line);
    }

    if estimate < wanted && (at_floor || (lines.is_empty() && line_end > 0)) {
        let line = Line {
            from: 0,
            to: line_end,
            next: line_end,
            ended: line_end < content || ended,
            partial: partial && line_end == content,
        };
        lines.push(line);
    }

    let mut pieces = Vec::new();
    for line in lines.iter().rev() {
        split(layout, buf, line, &mut pieces);
    }

    pieces.drain(..pieces.len().saturating_sub(wanted));

    (pieces, buf.len())
}

/// How many pieces a line of `len` bytes is cut into.
fn pieces_in(len: usize) -> usize {
    1 + len.saturating_sub(1) / MAX_SEGMENT
}

/// Cuts `line` into pieces of at most [`MAX_SEGMENT`] bytes, on characters.
fn split(layout: &Layout, buf: &[u8], line: &Line, pieces: &mut Vec<Piece>) {
    let mut to = line.to;
    if line.ended {
        to -= layout.carriage_return(&buf[line.from..to]);
    }

    let mut at = line.from;
    while to - at > MAX_SEGMENT {
        // A run with no character start in it (invalid UTF-8 bytes) is cut where the piece ends, so the loop always
        // moves on; the bytes show as replacement characters.
        let cut = Some(layout.cut(buf, at, at + MAX_SEGMENT))
            .filter(|cut| *cut > at)
            .unwrap_or(at + MAX_SEGMENT);

        pieces.push(Piece {
            from: at,
            to: cut,
            continues: true,
        });
        at = cut;
    }

    pieces.push(Piece {
        from: at,
        to,
        continues: line.partial,
    });
}

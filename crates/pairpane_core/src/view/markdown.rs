//! Markdown to a tree of plain data. Nothing of the file reaches the webview as markup: raw HTML is text, a link keeps
//! a URL only when it is http, https or mailto, and an image is its alternative text.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use pulldown_cmark::{Options, Parser};

use crate::model::MdDocument;

use super::file::ViewFile;
use super::md_build::Builder;
use super::md_frame::{Frame, Kind};
use super::{MAX_MARKDOWN, MAX_READ, cancelled, encoding};

/// The Markdown file as a tree, read as `encoding` (a label). At most [`MAX_MARKDOWN`] bytes are read, at most
/// 100,000 nodes and 32 levels of nesting are kept; when any limit cuts the file the document says it is truncated.
/// A raised `cancel` stops it, between reads and between events, with an `Interrupted` error.
pub fn markdown(file: &ViewFile, encoding: &str, cancel: &AtomicBool) -> io::Result<MdDocument> {
    let encoding = encoding::by_name(encoding)?;
    let size = file.size()?;
    let wanted = usize::try_from(size).map_or(MAX_MARKDOWN, |size| size.min(MAX_MARKDOWN));

    let mut bytes = Vec::with_capacity(wanted);
    while bytes.len() < wanted {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        let chunk = file.read_at(bytes.len() as u64, MAX_READ.min(wanted - bytes.len()))?;
        if chunk.is_empty() {
            break;
        }

        bytes.extend_from_slice(&chunk);
    }

    let (text, _) = encoding.decode_with_bom_removal(&bytes);
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

    let mut builder = Builder {
        stack: vec![Frame::new(Kind::Root)],
        skipping: 0,
        nodes: 0,
        truncated: size > MAX_MARKDOWN as u64,
    };
    for event in Parser::new_ext(&text, options) {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }

        if !builder.event(event) {
            break;
        }
    }

    // An open container (the node limit stopped the events) is closed where it stands.
    while builder.stack.len() > 1 {
        builder.close();
    }

    let truncated = builder.truncated;
    let blocks = builder.stack.pop().map(Frame::finish).unwrap_or_default();

    Ok(MdDocument { blocks, truncated })
}

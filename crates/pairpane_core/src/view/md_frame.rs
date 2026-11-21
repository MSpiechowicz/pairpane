//! The open containers of a Markdown tree under construction: what each is building and what it holds so far.

use crate::model::{MdBlock, MdInline, MdListItem};

/// What a frame of the stack is building.
pub(super) enum Kind {
    Root,
    Paragraph,
    Heading(u8),
    Quote,
    List {
        ordered: bool,
        start: Option<u64>,
    },
    Item,
    Code(Option<String>),
    Html,
    Emphasis,
    Strong,
    Strike,
    Link(Option<String>),
    Image,
    Table,
    Head,
    Row,
    Cell,
    /// A construct this viewer does not draw: its content goes to the parent as it is.
    Plain,
}

/// A container that is open: what it holds so far.
pub(super) struct Frame {
    pub(super) kind: Kind,
    pub(super) blocks: Vec<MdBlock>,
    pub(super) inlines: Vec<MdInline>,
    pub(super) items: Vec<MdListItem>,
    pub(super) cells: Vec<Vec<MdInline>>,
    pub(super) rows: Vec<Vec<Vec<MdInline>>>,
    pub(super) header: Vec<Vec<MdInline>>,
    /// The text of a code block or an HTML block, or the alternative text of an image.
    pub(super) text: String,
    pub(super) checked: Option<bool>,
}

impl Frame {
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            kind,
            blocks: Vec::new(),
            inlines: Vec::new(),
            items: Vec::new(),
            cells: Vec::new(),
            rows: Vec::new(),
            header: Vec::new(),
            text: String::new(),
            checked: None,
        }
    }

    /// Adds a block, after the loose text this container holds (a tight list item has text without a paragraph).
    pub(super) fn add_block(&mut self, block: MdBlock) {
        self.flush();
        self.blocks.push(block);
    }

    pub(super) fn flush(&mut self) {
        if !self.inlines.is_empty() {
            let inlines = std::mem::take(&mut self.inlines);
            self.blocks.push(MdBlock::Paragraph { inlines });
        }
    }

    pub(super) fn finish(mut self) -> Vec<MdBlock> {
        self.flush();
        self.blocks
    }

    pub(super) fn add_inline(&mut self, inline: MdInline) {
        self.inlines.push(inline);
    }

    /// Adds text to what the frame holds: the text of a code or HTML block, or an inline of text.
    pub(super) fn add_text(&mut self, text: &str) {
        match self.kind {
            Kind::Code(_) | Kind::Html => self.text.push_str(text),
            _ => match self.inlines.last_mut() {
                Some(MdInline::Text { text: last }) => last.push_str(text),
                _ => self.inlines.push(MdInline::Text { text: text.to_owned() }),
            },
        }
    }
}

//! Building the tree of a Markdown document from the events of the parser, within the limits of nesting and size.

use pulldown_cmark::{CodeBlockKind, Event, Tag};

use crate::model::{MdBlock, MdInline, MdListItem};

use super::md_frame::{Frame, Kind};

/// The deepest nesting of blocks and inlines kept; what is nested deeper is dropped and the document is truncated.
const MAX_DEPTH: usize = 32;

/// The most nodes (blocks, inlines, items, cells) kept; the rest is dropped and the document is truncated.
const MAX_NODES: usize = 100_000;

/// The longest link URL kept.
const MAX_HREF: usize = 2048;

/// The longest language name of a fenced block kept.
const MAX_LANGUAGE: usize = 32;

pub(super) struct Builder {
    pub(super) stack: Vec<Frame>,
    /// How many levels of a dropped subtree are open.
    pub(super) skipping: usize,
    pub(super) nodes: usize,
    pub(super) truncated: bool,
}

impl Builder {
    fn top(&mut self) -> &mut Frame {
        // The root is never closed while events come.
        self.stack.last_mut().expect("the root frame")
    }

    fn start(&mut self, tag: Tag<'_>) {
        if self.skipping > 0 {
            self.skipping += 1;
            return;
        }

        if self.stack.len() > MAX_DEPTH || self.nodes >= MAX_NODES {
            self.skipping = 1;
            self.truncated = true;
            return;
        }

        self.nodes += 1;
        self.stack.push(Frame::new(kind_of(tag)));
    }

    fn end(&mut self) {
        if self.skipping > 0 {
            self.skipping -= 1;
        } else if self.stack.len() > 1 {
            self.close();
        }
    }

    /// Closes the open frame into its parent.
    pub(super) fn close(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };

        let parent = self.top();

        match frame.kind {
            Kind::Root => {}
            Kind::Paragraph => parent.add_block(MdBlock::Paragraph { inlines: frame.inlines }),
            Kind::Heading(level) => parent.add_block(MdBlock::Heading {
                level,
                inlines: frame.inlines,
            }),
            Kind::Quote => parent.add_block(MdBlock::Quote { blocks: frame.finish() }),
            Kind::List { ordered, start } => parent.add_block(MdBlock::List {
                ordered,
                start,
                items: frame.items,
            }),
            Kind::Item => {
                let checked = frame.checked;
                parent.items.push(MdListItem {
                    checked,
                    blocks: frame.finish(),
                });
            }
            Kind::Code(language) => parent.add_block(MdBlock::CodeBlock {
                language,
                text: frame.text,
            }),
            Kind::Html => parent.add_block(MdBlock::Paragraph {
                inlines: vec![MdInline::Text { text: frame.text }],
            }),
            Kind::Emphasis => parent.add_inline(MdInline::Emphasis {
                children: frame.inlines,
            }),
            Kind::Strong => parent.add_inline(MdInline::Strong {
                children: frame.inlines,
            }),
            Kind::Strike => parent.add_inline(MdInline::Strike {
                children: frame.inlines,
            }),
            Kind::Link(href) => parent.add_inline(MdInline::Link {
                href,
                children: frame.inlines,
            }),
            Kind::Image => parent.add_inline(MdInline::Image { alt: frame.text }),
            Kind::Table => parent.add_block(MdBlock::Table {
                header: frame.header,
                rows: frame.rows,
            }),
            Kind::Head => parent.header = frame.cells,
            Kind::Row => parent.rows.push(frame.cells),
            Kind::Cell => parent.cells.push(frame.inlines),
            Kind::Plain => {
                parent.inlines.extend(frame.inlines);
                parent.blocks.extend(frame.blocks);
            }
        }
    }

    /// Takes one event; `false` once the node limit is reached.
    pub(super) fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(_) => self.end(),
            _ if self.skipping > 0 => {}
            _ if self.nodes >= MAX_NODES => {
                self.truncated = true;
                return false;
            }
            leaf => {
                self.nodes += 1;
                self.leaf(leaf);
            }
        }

        true
    }

    /// Adds an event that opens nothing. Inside an image, text of any kind (also inside emphasis and the like) is part
    /// of the alternative text.
    fn leaf(&mut self, event: Event<'_>) {
        if let Some(image) = self
            .stack
            .iter_mut()
            .rev()
            .find(|frame| matches!(frame.kind, Kind::Image))
        {
            match event {
                Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) | Event::Html(text) => {
                    image.text.push_str(&text)
                }
                Event::SoftBreak | Event::HardBreak => image.text.push(' '),
                _ => {}
            }

            return;
        }

        let top = self.top();

        match event {
            Event::Text(text)
            | Event::InlineHtml(text)
            | Event::Html(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => top.add_text(&text),
            Event::Code(text) => top.add_inline(MdInline::Code {
                text: text.into_string(),
            }),
            Event::SoftBreak => top.add_text(" "),
            Event::HardBreak => top.add_inline(MdInline::Break),
            Event::Rule => top.add_block(MdBlock::Rule),
            Event::TaskListMarker(checked) => top.checked = Some(checked),
            Event::Start(_) | Event::End(_) => {}
        }
    }
}

fn kind_of(tag: Tag<'_>) -> Kind {
    match tag {
        Tag::Paragraph => Kind::Paragraph,
        Tag::Heading { level, .. } => Kind::Heading(level as u8),
        Tag::BlockQuote(_) => Kind::Quote,
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => Kind::Code(language(&info)),
        Tag::CodeBlock(CodeBlockKind::Indented) => Kind::Code(None),
        Tag::HtmlBlock => Kind::Html,
        Tag::List(start) => Kind::List {
            ordered: start.is_some(),
            start,
        },
        Tag::Item => Kind::Item,
        Tag::Table(_) => Kind::Table,
        Tag::TableHead => Kind::Head,
        Tag::TableRow => Kind::Row,
        Tag::TableCell => Kind::Cell,
        Tag::Emphasis => Kind::Emphasis,
        Tag::Strong => Kind::Strong,
        Tag::Strikethrough => Kind::Strike,
        Tag::Link { dest_url, .. } => Kind::Link(safe_href(&dest_url)),
        Tag::Image { .. } => Kind::Image,
        _ => Kind::Plain,
    }
}

/// The language of a fenced block: the first word of its info string, when it is short.
fn language(info: &str) -> Option<String> {
    let word = info.split_whitespace().next()?;

    (word.chars().count() <= MAX_LANGUAGE).then(|| word.to_owned())
}

/// The URL when a click may open it: http, https or mailto, of a sane length and without control characters.
fn safe_href(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let scheme = ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme));

    (scheme && url.len() <= MAX_HREF && !url.chars().any(char::is_control)).then(|| url.to_owned())
}

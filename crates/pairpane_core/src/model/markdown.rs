use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A Markdown file as a tree the webview draws itself. It holds text only: raw HTML is kept as literal text, a link
/// keeps a URL only when it is http, https or mailto, and an image is its alternative text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MdDocument {
    pub blocks: Vec<MdBlock>,
    /// The file was cut (too long, too nested or too many nodes); what is shown is the start.
    pub truncated: bool,
}

/// A block of a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum MdBlock {
    Paragraph {
        inlines: Vec<MdInline>,
    },
    Heading {
        /// 1 to 6.
        level: u8,
        inlines: Vec<MdInline>,
    },
    CodeBlock {
        /// The first word of the info string of a fenced block.
        language: Option<String>,
        text: String,
    },
    Quote {
        blocks: Vec<MdBlock>,
    },
    List {
        ordered: bool,
        /// The number of the first item of an ordered list.
        #[ts(type = "number | null")]
        start: Option<u64>,
        items: Vec<MdListItem>,
    },
    Table {
        header: Vec<Vec<MdInline>>,
        rows: Vec<Vec<Vec<MdInline>>>,
    },
    Rule,
}

/// One item of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MdListItem {
    /// The state of a task list box (`- [x]`); `null` for an item without one.
    pub checked: Option<bool>,
    pub blocks: Vec<MdBlock>,
}

/// Text inside a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum MdInline {
    Text {
        text: String,
    },
    Code {
        text: String,
    },
    Emphasis {
        children: Vec<MdInline>,
    },
    Strong {
        children: Vec<MdInline>,
    },
    Strike {
        children: Vec<MdInline>,
    },
    /// `href` is `null` unless the URL is http, https or mailto.
    Link {
        href: Option<String>,
        children: Vec<MdInline>,
    },
    /// An image is never loaded; its alternative text stands in for it.
    Image {
        alt: String,
    },
    /// A hard line break.
    Break,
}

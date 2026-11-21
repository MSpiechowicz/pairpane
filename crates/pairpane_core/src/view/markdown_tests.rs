//! Markdown to data: the structure, and that nothing in a file can become markup, a script or a link that runs.

use pretty_assertions::assert_eq;

use crate::model::{MdBlock, MdDocument, MdInline, MdListItem};

use super::testing::{file_of, idle, opened};
use super::{MAX_MARKDOWN, markdown};

fn parse(source: &str) -> MdDocument {
    let (_folder, file) = file_of(source.as_bytes());

    markdown(&file, "UTF-8", &idle()).expect("a document")
}

fn plain(text: &str) -> MdInline {
    MdInline::Text { text: text.to_owned() }
}

fn paragraph(inlines: Vec<MdInline>) -> MdBlock {
    MdBlock::Paragraph { inlines }
}

/// Every inline of the document, depth first.
fn inlines_of(blocks: &[MdBlock], out: &mut Vec<MdInline>) {
    fn walk(inline: &MdInline, out: &mut Vec<MdInline>) {
        out.push(inline.clone());

        if let MdInline::Emphasis { children }
        | MdInline::Strong { children }
        | MdInline::Strike { children }
        | MdInline::Link { children, .. } = inline
        {
            children.iter().for_each(|child| walk(child, out));
        }
    }

    for block in blocks {
        match block {
            MdBlock::Paragraph { inlines } | MdBlock::Heading { inlines, .. } => {
                inlines.iter().for_each(|inline| walk(inline, out))
            }
            MdBlock::Quote { blocks } => inlines_of(blocks, out),
            MdBlock::List { items, .. } => items.iter().for_each(|item| inlines_of(&item.blocks, out)),
            MdBlock::Table { header, rows } => {
                let cells = header.iter().chain(rows.iter().flatten());
                cells.flatten().for_each(|inline| walk(inline, out));
            }
            MdBlock::CodeBlock { .. } | MdBlock::Rule => {}
        }
    }
}

fn depth(blocks: &[MdBlock]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            MdBlock::Quote { blocks } => 1 + depth(blocks),
            MdBlock::List { items, .. } => 1 + items.iter().map(|item| depth(&item.blocks)).max().unwrap_or(0),
            _ => 1,
        })
        .max()
        .unwrap_or(0)
}

fn hrefs(document: &MdDocument) -> Vec<Option<String>> {
    let mut all = Vec::new();
    inlines_of(&document.blocks, &mut all);

    all.into_iter()
        .filter_map(|inline| match inline {
            MdInline::Link { href, .. } => Some(href),
            _ => None,
        })
        .collect()
}

#[test]
fn headings_paragraphs_emphasis_and_code() {
    let document =
        parse("# Title\n\nSome *emphasis*, **strong**, ~~gone~~ and `code`.\nNext line  \nhard break\n\n---\n");

    assert_eq!(
        document.blocks,
        [
            MdBlock::Heading {
                level: 1,
                inlines: vec![plain("Title")]
            },
            paragraph(vec![
                plain("Some "),
                MdInline::Emphasis {
                    children: vec![plain("emphasis")]
                },
                plain(", "),
                MdInline::Strong {
                    children: vec![plain("strong")]
                },
                plain(", "),
                MdInline::Strike {
                    children: vec![plain("gone")]
                },
                plain(" and "),
                MdInline::Code {
                    text: "code".to_owned()
                },
                plain(". Next line"),
                MdInline::Break,
                plain("hard break"),
            ]),
            MdBlock::Rule,
        ]
    );
    assert!(!document.truncated);
}

#[test]
fn lists_tasks_quotes_code_and_tables() {
    let document = parse(
        "1. one\n2. two\n\n- [x] done\n- [ ] open\n\n> quoted\n\n```rust title\nlet a = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
    );

    assert_eq!(
        document.blocks,
        [
            MdBlock::List {
                ordered: true,
                start: Some(1),
                items: vec![
                    MdListItem {
                        checked: None,
                        blocks: vec![paragraph(vec![plain("one")])]
                    },
                    MdListItem {
                        checked: None,
                        blocks: vec![paragraph(vec![plain("two")])]
                    },
                ],
            },
            MdBlock::List {
                ordered: false,
                start: None,
                items: vec![
                    MdListItem {
                        checked: Some(true),
                        blocks: vec![paragraph(vec![plain("done")])]
                    },
                    MdListItem {
                        checked: Some(false),
                        blocks: vec![paragraph(vec![plain("open")])]
                    },
                ],
            },
            MdBlock::Quote {
                blocks: vec![paragraph(vec![plain("quoted")])]
            },
            MdBlock::CodeBlock {
                language: Some("rust".to_owned()),
                text: "let a = 1;\n".to_owned()
            },
            MdBlock::Table {
                header: vec![vec![plain("a")], vec![plain("b")]],
                rows: vec![vec![vec![plain("1")], vec![plain("2")]]],
            },
        ]
    );
}

#[test]
fn html_is_literal_text() {
    let document =
        parse("<script>alert(1)</script>\n\nText with <b onclick=\"x()\">inline</b> html and <img src=x onerror=y>.\n");

    let [
        MdBlock::Paragraph { inlines: block },
        MdBlock::Paragraph { inlines: line },
    ] = document.blocks.as_slice()
    else {
        panic!("two paragraphs: {:?}", document.blocks);
    };

    assert_eq!(block, &[plain("<script>alert(1)</script>\n")]);
    let joined: String = line
        .iter()
        .map(|inline| match inline {
            MdInline::Text { text } => text.as_str(),
            other => panic!("only text: {other:?}"),
        })
        .collect();
    assert_eq!(
        joined,
        "Text with <b onclick=\"x()\">inline</b> html and <img src=x onerror=y>."
    );
}

#[test]
fn only_http_https_and_mailto_links_keep_their_url() {
    let document = parse(
        "[a](http://example.com) [b](https://example.com/x?y=1) [c](mailto:me@example.com) [d](javascript:alert(1)) \
         [e](JaVaScRiPt:alert(1)) [f](data:text/html,<b>) [g](file:///etc/passwd) [h](/relative) [i](vbscript:x) \
         <https://auto.example> <javascript:alert(1)>\n",
    );

    assert_eq!(
        hrefs(&document),
        [
            Some("http://example.com".to_owned()),
            Some("https://example.com/x?y=1".to_owned()),
            Some("mailto:me@example.com".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("https://auto.example".to_owned()),
            None,
        ]
    );
}

#[test]
fn a_dropped_link_keeps_its_text() {
    let document = parse("[click me](javascript:alert(1))\n");

    assert_eq!(
        document.blocks,
        [paragraph(vec![MdInline::Link {
            href: None,
            children: vec![plain("click me")]
        }])]
    );
}

#[test]
fn a_very_long_url_is_dropped() {
    let document = parse(&format!("[x](https://example.com/{})\n", "a".repeat(3000)));

    assert_eq!(hrefs(&document), [None]);
}

#[test]
fn an_image_is_its_alternative_text() {
    let document = parse("![the *alt* text](http://example.com/a.png \"title\") and ![](x.png)\n");

    assert_eq!(
        document.blocks,
        [paragraph(vec![
            MdInline::Image {
                alt: "the alt text".to_owned()
            },
            plain(" and "),
            MdInline::Image { alt: String::new() },
        ])]
    );
}

#[test]
fn nesting_deeper_than_the_limit_is_dropped() {
    let document = parse(&format!("{}deep\n\nafter\n", "> ".repeat(100)));

    assert!(document.truncated);
    assert!(depth(&document.blocks) <= 33, "{}", depth(&document.blocks));
    assert!(depth(&document.blocks) >= 30);
}

#[test]
fn the_node_limit_cuts_the_document() {
    let source = "a\n\n".repeat(60_000);

    let document = parse(&source);

    assert!(document.truncated);
    assert!(document.blocks.len() > 10_000);
    assert!(document.blocks.len() < 60_000);
}

#[test]
fn a_file_over_the_size_limit_is_cut() {
    let (_folder, file) = file_of(&b"word ".repeat(MAX_MARKDOWN / 5 + 1000));

    let document = markdown(&file, "UTF-8", &idle()).expect("a document");

    assert!(document.truncated);
    let MdBlock::Paragraph { inlines } = &document.blocks[0] else {
        panic!("a paragraph");
    };
    let MdInline::Text { text } = &inlines[0] else {
        panic!("text");
    };
    assert!(text.len() <= MAX_MARKDOWN);
    assert!(text.len() > MAX_MARKDOWN - 10);
}

#[test]
fn a_small_file_is_not_truncated() {
    assert!(!parse("# hi\n").truncated);
    assert_eq!(parse("").blocks, []);
}

#[test]
fn the_encoding_decides_the_characters_and_a_mark_is_dropped() {
    let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode("# Łódź\n");
    let (_folder, file) = file_of(&bytes);
    assert_eq!(
        markdown(&file, "windows-1250", &idle()).expect("a document").blocks,
        [MdBlock::Heading {
            level: 1,
            inlines: vec![plain("Łódź")]
        }]
    );

    let (_folder, opened) = opened("a.md", b"\xEF\xBB\xBF# Mark\n");
    assert_eq!(
        markdown(&opened.file, "UTF-8", &idle()).expect("a document").blocks,
        [MdBlock::Heading {
            level: 1,
            inlines: vec![plain("Mark")]
        }]
    );

    let error = markdown(&file, "klingon", &idle()).expect_err("unknown");
    assert_eq!(error.to_string(), "the encoding is not known");
}

#[test]
fn binary_garbage_never_panics() {
    let bytes: Vec<u8> = (0..20_000u32)
        .map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let (_folder, file) = file_of(&bytes);

    let document = markdown(&file, "UTF-8", &idle()).expect("a document");

    assert!(depth(&document.blocks) <= 40);
}

#[test]
fn a_raised_flag_stops_the_document() {
    let (_folder, file) = file_of(b"# a title\n\nsome text\n");
    let cancel = std::sync::atomic::AtomicBool::new(true);

    let error = markdown(&file, "UTF-8", &cancel).expect_err("cancelled");

    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "the read was cancelled");
}

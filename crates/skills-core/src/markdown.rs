//! Turns SKILL.md bodies into a flat list of display blocks the UI can render
//! with plain text widgets: headings, paragraphs, list items, code and quotes.
//! Inline formatting is flattened to text; links keep their label. HTML is
//! shown as text, never interpreted.

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::sanitize::sanitize_display;

const MAX_BLOCKS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Heading1,
    Heading2,
    Heading3,
    Paragraph,
    /// A list item; `depth` is its nesting level (0 = top).
    Bullet,
    Code,
    Quote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
    pub depth: usize,
}

pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut text = String::new();
    let mut kind: Option<BlockKind> = None;
    let mut list_depth = 0usize;
    let mut quote = false;

    let flush = |out: &mut Vec<Block>, text: &mut String, kind: BlockKind, depth: usize| {
        let t = if kind == BlockKind::Code {
            // Keep line breaks in code, but strip other control characters.
            text.trim_end()
                .lines()
                .map(sanitize_display)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            sanitize_display(text)
        };
        if !t.is_empty() && out.len() < MAX_BLOCKS {
            out.push(Block {
                kind,
                text: t,
                depth,
            });
        }
        text.clear();
    };

    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                kind = Some(match level {
                    HeadingLevel::H1 => BlockKind::Heading1,
                    HeadingLevel::H2 => BlockKind::Heading2,
                    _ => BlockKind::Heading3,
                });
            }
            Event::Start(Tag::Paragraph) => {
                if kind.is_none() {
                    kind = Some(if quote {
                        BlockKind::Quote
                    } else {
                        BlockKind::Paragraph
                    });
                }
            }
            Event::Start(Tag::List(_)) => {
                if let Some(k) = kind.take() {
                    flush(&mut out, &mut text, k, list_depth.saturating_sub(1));
                }
                list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Item) => kind = Some(BlockKind::Bullet),
            Event::Start(Tag::BlockQuote(_)) => quote = true,
            Event::End(TagEnd::BlockQuote(_)) => quote = false,
            Event::Start(Tag::CodeBlock(_)) => kind = Some(BlockKind::Code),
            Event::End(
                TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::Item | TagEnd::CodeBlock,
            ) => {
                if let Some(k) = kind {
                    // A paragraph inside a list item belongs to the item.
                    if k == BlockKind::Bullet && matches!(event, Event::End(TagEnd::Paragraph)) {
                        continue;
                    }
                    flush(&mut out, &mut text, k, list_depth.saturating_sub(1));
                    kind = None;
                }
            }
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::Html(t) | Event::InlineHtml(t) => text.push_str(&t),
            Event::SoftBreak => text.push(' '),
            Event::HardBreak => text.push('\n'),
            _ => {}
        }
    }
    if let Some(k) = kind {
        flush(&mut out, &mut text, k, list_depth.saturating_sub(1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_common_markdown() {
        let md = "# Title\n\nSome **bold** and [a link](http://x).\n\n## Steps\n\n- one\n- two\n  - nested\n\n```sh\nls -la\necho hi\n```\n\n> note\n";
        let b = blocks(md);
        let kinds: Vec<BlockKind> = b.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading1,
                BlockKind::Paragraph,
                BlockKind::Heading2,
                BlockKind::Bullet,
                BlockKind::Bullet,
                BlockKind::Bullet,
                BlockKind::Code,
                BlockKind::Quote,
            ]
        );
        assert_eq!(b[1].text, "Some bold and a link.");
        assert_eq!(b[5].text, "nested");
        assert_eq!(b[5].depth, 1);
        assert_eq!(b[6].text, "ls -la\necho hi");
    }

    #[test]
    fn strips_control_characters() {
        let b = blocks("hello \u{1b}[31mred\u{202e}");
        assert_eq!(b[0].text, "hello [31mred");
    }
}

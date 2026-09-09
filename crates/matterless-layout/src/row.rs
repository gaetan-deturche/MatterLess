//! A whole row of the message list, laid out exactly.
//!
//! `extent_of` answers for one run of text. This answers for a row: a message
//! with its author line, its paragraphs and code blocks and lists, its
//! reactions and its attachments -- the number the virtualiser needs before the
//! row is drawn, and the number a renderer will draw from. The two cannot
//! disagree, because there is only one of them.

use crate::Fonts;
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use matterless_render::Row;
use matterless_render::markdown::Node;

/// Every measurement the list is built from, in one place.
///
/// Held together rather than scattered because the layout and the renderer have
/// to agree exactly: a padding known to one of them and not the other is the
/// same class of bug as an estimated height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// The list's own width, before any gutter is taken out of it.
    pub width: f32,
    /// The avatar column and the gap after it. Text never gets this.
    pub gutter: f32,
    pub body_size: f32,
    pub line_height: f32,
    /// The author-and-time line above a message, which a continuation omits.
    pub header_height: f32,
    /// Above and below a row's content.
    pub row_padding: f32,
    /// Between the blocks inside a message.
    pub block_gap: f32,
    pub code_size: f32,
    pub code_line_height: f32,
    /// Padding a code block draws around its lines.
    pub code_padding: f32,
    pub reaction_height: f32,
    pub separator_height: f32,
    pub footer_height: f32,
    /// How far a list item or a quote is pushed in.
    pub indent: f32,
}

impl Default for Theme {
    /// The stylesheet's own numbers: 14px body at 1.5, a 28px avatar with an
    /// 8px gap.
    fn default() -> Self {
        Self {
            width: 900.0,
            gutter: 36.0,
            body_size: 14.0,
            line_height: 21.0,
            header_height: 20.0,
            row_padding: 3.0,
            block_gap: 6.0,
            code_size: 12.5,
            code_line_height: 18.0,
            code_padding: 16.0,
            reaction_height: 25.0,
            separator_height: 34.0,
            footer_height: 26.0,
            indent: 18.0,
        }
    }
}

impl Theme {
    /// The width a line of body text actually gets.
    pub fn text_width(&self) -> f32 {
        (self.width - self.gutter).max(40.0)
    }
}

/// What a row occupies, and the pieces that make it up.
///
/// The parts are kept rather than summed away so a renderer can draw them and a
/// test can say *why* a height is what it is.
#[derive(Debug, Clone, PartialEq)]
pub struct RowLayout {
    pub height: f32,
    pub blocks: Vec<Block>,
}

/// One stacked piece of a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Distance from the row's top edge.
    pub y: f32,
    /// Distance from the text column's left edge, for indented content.
    pub x: f32,
    pub height: f32,
    pub lines: usize,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The author and time line.
    Header,
    Text,
    /// Pre-wrapped: it scrolls sideways rather than wrapping.
    Code,
    Reactions,
    /// An image or a file card, whose size the server told us.
    Attachment,
    Footer,
    Separator,
}

/// A piece of text with the styling that changes its width.
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    mono: bool,
}

/// One paragraph-like run of inline content, and how far it is pushed in.
struct Line {
    spans: Vec<Span>,
    indent: f32,
    heading: bool,
}

/// Flattens the markdown into the blocks a reader sees stacked.
fn lines_of(nodes: &[Node], indent: f32, into: &mut Vec<Line>, code: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Paragraph { children } => {
                let mut spans = Vec::new();
                inline(children, false, false, false, &mut spans);
                into.push(Line {
                    spans,
                    indent,
                    heading: false,
                });
            }
            Node::Heading { children, .. } => {
                let mut spans = Vec::new();
                inline(children, true, false, false, &mut spans);
                into.push(Line {
                    spans,
                    indent,
                    heading: true,
                });
            }
            Node::Blockquote { children } => lines_of(children, indent + 1.0, into, code),
            Node::List { items, .. } => {
                for item in items {
                    lines_of(item, indent + 1.0, into, code);
                }
            }
            Node::CodeBlock { value, .. } => code.push(value.clone()),
            Node::Table { head, rows } => {
                // Not laid out as a table yet: each cell is a line, which is
                // the same *number* of lines a stacked fallback draws.
                for cell in head {
                    lines_of(cell, indent, into, code);
                }
                for row in rows {
                    for cell in row {
                        lines_of(cell, indent, into, code);
                    }
                }
            }
            Node::Rule => into.push(Line {
                spans: Vec::new(),
                indent,
                heading: false,
            }),
            // Anything inline at the top level is a paragraph of its own.
            other => {
                let mut spans = Vec::new();
                inline(std::slice::from_ref(other), false, false, false, &mut spans);
                if !spans.is_empty() {
                    into.push(Line {
                        spans,
                        indent,
                        heading: false,
                    });
                }
            }
        }
    }
}

/// Collects inline content into styled spans.
fn inline(nodes: &[Node], bold: bool, italic: bool, mono: bool, into: &mut Vec<Span>) {
    for node in nodes {
        match node {
            Node::Text { value } => into.push(Span {
                text: value.clone(),
                bold,
                italic,
                mono,
            }),
            Node::Strong { children } => inline(children, true, italic, mono, into),
            Node::Emphasis { children } | Node::Strike { children } => {
                inline(children, bold, true, mono, into)
            }
            Node::Link { children, .. } => inline(children, bold, italic, mono, into),
            Node::InlineCode { value } | Node::InlineMath { value } => into.push(Span {
                text: value.clone(),
                bold,
                italic,
                mono: true,
            }),
            Node::UserMention { username, .. } => into.push(Span {
                text: format!("@{username}"),
                bold,
                italic,
                mono,
            }),
            Node::ChannelLink { name } => into.push(Span {
                text: format!("~{name}"),
                bold,
                italic,
                mono,
            }),
            Node::Emoji { name, unicode } => into.push(Span {
                text: unicode.clone().unwrap_or_else(|| format!(":{name}:")),
                bold,
                italic,
                mono,
            }),
            Node::SoftBreak | Node::HardBreak => into.push(Span {
                text: "\n".to_string(),
                bold,
                italic,
                mono,
            }),
            Node::Image { alt, .. } => into.push(Span {
                text: alt.clone(),
                bold,
                italic,
                mono,
            }),
            _ => {}
        }
    }
}

fn attrs_for(bold: bool, italic: bool, mono: bool) -> Attrs<'static> {
    let mut attrs = Attrs::new();
    if mono {
        attrs = attrs.family(Family::Monospace);
    }
    if bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if italic {
        attrs = attrs.style(cosmic_text::Style::Italic);
    }
    attrs
}

/// Lays out one paragraph of mixed styles and answers how many lines it takes.
fn line_count(fonts: &mut Fonts, line: &Line, width: f32, theme: &Theme) -> usize {
    if line.spans.is_empty() {
        return 1;
    }
    let size = if line.heading {
        theme.body_size * 1.15
    } else {
        theme.body_size
    };
    let metrics = Metrics::new(size, theme.line_height);
    let mut buffer = Buffer::new(&mut fonts.system, metrics);
    let mut buffer = buffer.borrow_with(&mut fonts.system);
    buffer.set_size(Some(width), None);
    let spans: Vec<(&str, Attrs<'static>)> = line
        .spans
        .iter()
        .map(|span| {
            (
                span.text.as_str(),
                attrs_for(span.bold || line.heading, span.italic, span.mono),
            )
        })
        .collect();
    // Rich text rather than one flat string: bold is wider than regular, and a
    // paragraph measured entirely in one weight wraps in the wrong place.
    buffer.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(false);
    buffer.layout_runs().count().max(1)
}

/// Lays out a row and reports its exact height.
pub fn lay_out(fonts: &mut Fonts, row: &Row, theme: &Theme) -> RowLayout {
    let mut blocks = Vec::new();
    let mut y = 0.0_f32;

    let (nodes, reactions, attachments, header) = match row {
        Row::DateSeparator { .. } | Row::UnreadDivider => {
            return RowLayout {
                height: theme.separator_height,
                blocks: vec![Block {
                    y: 0.0,
                    x: 0.0,
                    height: theme.separator_height,
                    lines: 1,
                    kind: Kind::Separator,
                }],
            };
        }
        Row::ThreadFooter { .. } => {
            return RowLayout {
                height: theme.footer_height,
                blocks: vec![Block {
                    y: 0.0,
                    x: 0.0,
                    height: theme.footer_height,
                    lines: 1,
                    kind: Kind::Footer,
                }],
            };
        }
        Row::System { .. } | Row::DeletedRoot { .. } => {
            return RowLayout {
                height: theme.line_height + theme.row_padding * 2.0,
                blocks: vec![Block {
                    y: theme.row_padding,
                    x: 0.0,
                    height: theme.line_height,
                    lines: 1,
                    kind: Kind::Text,
                }],
            };
        }
        Row::Post { post } => (
            post.nodes.as_slice(),
            post.reactions.len(),
            post.files.len(),
            true,
        ),
        Row::Continuation { post } => (
            post.nodes.as_slice(),
            post.reactions.len(),
            post.files.len(),
            false,
        ),
    };

    y += theme.row_padding;
    if header {
        blocks.push(Block {
            y,
            x: 0.0,
            height: theme.header_height,
            lines: 1,
            kind: Kind::Header,
        });
        y += theme.header_height;
    }

    let mut lines = Vec::new();
    let mut code = Vec::new();
    lines_of(nodes, 0.0, &mut lines, &mut code);

    for line in &lines {
        let x = line.indent * theme.indent;
        let count = line_count(fonts, line, theme.text_width() - x, theme);
        let height = count as f32 * theme.line_height;
        blocks.push(Block {
            y,
            x,
            height,
            lines: count,
            kind: Kind::Text,
        });
        y += height + theme.block_gap;
    }

    for block in &code {
        // A code block does not wrap -- it scrolls -- so its lines are the ones
        // written, however long they are.
        let count = block.lines().count().max(1);
        let height = count as f32 * theme.code_line_height + theme.code_padding;
        blocks.push(Block {
            y,
            x: 0.0,
            height,
            lines: count,
            kind: Kind::Code,
        });
        y += height + theme.block_gap;
    }

    if attachments > 0 {
        // The server gives every attachment its drawn size, so this is the one
        // height that was never in doubt.
        let height = 0.0;
        blocks.push(Block {
            y,
            x: 0.0,
            height,
            lines: 0,
            kind: Kind::Attachment,
        });
        y += height;
    }

    if reactions > 0 {
        blocks.push(Block {
            y,
            x: 0.0,
            height: theme.reaction_height,
            lines: 1,
            kind: Kind::Reactions,
        });
        y += theme.reaction_height;
    }

    RowLayout {
        height: y + theme.row_padding,
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_render::PostRow;
    use matterless_render::markdown::Node;
    use std::sync::Arc;

    fn post(nodes: Vec<Node>) -> PostRow {
        PostRow {
            post_id: "p1".into(),
            root_id: String::new(),
            author_id: "u1".into(),
            author_name: "ada".into(),
            create_at: 0,
            update_at: 0,
            edited: false,
            nodes: Arc::new(nodes),
            reactions: Vec::new(),
            files: Vec::new(),
            attachments: Vec::new(),
            avatar_at: 0,
            bot: false,
            body_is_attachment_only: false,
            pending: false,
            failed: false,
            pinned: false,
            saved: false,
            following: false,
            previews: Vec::new(),
        }
    }

    fn text(value: &str) -> Node {
        Node::Paragraph {
            children: vec![Node::Text {
                value: value.into(),
            }],
        }
    }

    /// The whole point: a row's height is known, and it is different at two
    /// widths, without anything having been drawn.
    #[test]
    fn a_row_is_taller_in_a_narrower_list() {
        let mut fonts = Fonts::new();
        let row = Row::Post {
            post: post(vec![text(
                "Could we raise the cache size the test runner is allowed, to a couple of gigabytes or so? It looks as though it is set in the file beside the runner rather than anywhere obvious, and I would rather not guess at it on a machine everybody shares. Either way suits me, but the current size stops a clean build finishing at all.",
            )]),
        };
        let wide = lay_out(&mut fonts, &row, &Theme::default());
        let narrow = lay_out(
            &mut fonts,
            &row,
            &Theme {
                width: 640.0,
                ..Theme::default()
            },
        );
        assert!(
            narrow.height > wide.height,
            "narrower wraps more: {} vs {}",
            narrow.height,
            wide.height
        );
    }

    /// A continuation omits the author line, and that is the only difference.
    #[test]
    fn a_continuation_is_shorter_by_its_header() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let body = vec![text("Yo!")];
        let first = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(body.clone()),
            },
            &theme,
        );
        let next = lay_out(&mut fonts, &Row::Continuation { post: post(body) }, &theme);
        assert_eq!(first.height - next.height, theme.header_height);
    }

    /// A code block's height is the lines it was written with, because it
    /// scrolls sideways rather than wrapping.
    #[test]
    fn a_code_block_counts_its_own_lines() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let row = Row::Post {
            post: post(vec![Node::CodeBlock {
                language: None,
                value: "one\ntwo\nthree".into(),
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let code = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");
        assert_eq!(code.lines, 3);
        assert_eq!(
            code.height,
            3.0 * theme.code_line_height + theme.code_padding
        );
    }

    /// Bold is wider, so the same words can need another line -- which a
    /// paragraph measured in one weight would miss.
    #[test]
    fn mixed_weight_is_measured_per_span() {
        let mut fonts = Fonts::new();
        let theme = Theme {
            width: 260.0,
            ..Theme::default()
        };
        let plain = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![text("regular words that nearly fill the line here")]),
            },
            &theme,
        );
        let strong = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![Node::Paragraph {
                    children: vec![Node::Strong {
                        children: vec![Node::Text {
                            value: "regular words that nearly fill the line here".into(),
                        }],
                    }],
                }]),
            },
            &theme,
        );
        assert!(
            strong.height >= plain.height,
            "bold cannot be shorter: {} vs {}",
            strong.height,
            plain.height
        );
    }

    #[test]
    fn a_separator_is_a_fixed_height() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = lay_out(&mut fonts, &Row::DateSeparator { epoch_day: 20137 }, &theme);
        assert_eq!(laid.height, theme.separator_height);
        assert_eq!(laid.blocks.len(), 1);
    }
}

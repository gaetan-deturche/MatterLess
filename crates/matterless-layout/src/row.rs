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
    /// A file card: an attachment that is not a picture, drawn as a fixed row
    /// with its name and size on it.
    pub card_height: f32,
    /// Inside a reaction pill, and between two of them.
    pub pill_padding: f32,
    pub pill_gap: f32,
    /// The square a custom emoji is drawn in, which has no character to shape.
    pub emoji_size: f32,
    pub reaction_height: f32,
    pub separator_height: f32,
    pub footer_height: f32,
    /// How far a list item or a quote is pushed in.
    pub indent: f32,
    /// The bar beside a preview card, and the gap between it and the text.
    pub quote_bar: f32,
    /// How many lines of a description or a quoted message are kept. Past this
    /// a card stops being a summary and starts being the page.
    pub preview_lines: usize,
    /// The reader's own offset, so a timestamp says what their clock says.
    pub utc_offset_minutes: i32,
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
            card_height: 56.0,
            pill_padding: 7.0,
            pill_gap: 4.0,
            emoji_size: 16.0,
            reaction_height: 25.0,
            separator_height: 34.0,
            footer_height: 26.0,
            indent: 18.0,
            quote_bar: 3.0,
            preview_lines: 3,
            utc_offset_minutes: 0,
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
    /// What to draw, for a block that is text. Empty for the rest.
    pub spans: Vec<TextSpan>,
    /// The size the spans were measured at, which a heading raises.
    pub size: f32,
    /// The width the spans were wrapped to. A renderer that wraps to anything
    /// else draws a different number of lines than the layout reserved.
    pub wrap: f32,
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
    /// One line of a link or permalink card. A card is several of these
    /// stacked with no gap, so the bar drawn beside them reads as one.
    Preview,
    Footer,
    Separator,
}

/// A piece of text with the styling that changes its width.
///
/// Carried out of the layout rather than consumed by it: a renderer has to draw
/// exactly what was measured, and re-deriving the spans from the markdown is how
/// the two drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// Where this text points, when it is part of a link. Carried out of the
    /// layout for the same reason the styling is: only the shaping knows where
    /// the words landed, and re-deriving them from the markdown is how a hit
    /// box and the text under it come to disagree.
    pub link: Option<String>,
    /// Drawn in the quieter ink: a timestamp, a reaction's count, anything the
    /// eye should pass over on its way to the message.
    pub faint: bool,
    /// A custom emoji standing in this span, by name.
    ///
    /// It has no character to shape, so the span holds non-breaking spaces and
    /// the renderer draws the picture over exactly the room they took.
    /// Reserving it in the text is what keeps the line count right, which is
    /// what keeps the row height right.
    pub emoji: Option<String>,
}

/// How many spaces a custom emoji reserves in a line.
///
/// Four measure close to the sixteen pixels the picture is drawn at, at the
/// sizes this app uses. Close rather than exact on purpose: the picture is
/// drawn to whatever they actually measured, so the reserved space and the
/// image agree however the font shapes them.
const EMOJI_ROOM: usize = 4;

/// One of them. Non-breaking, or a wrap could split the placeholder in half and
/// leave the picture straddling two lines.
const NBSP: &str = "\u{00A0}";

/// One paragraph-like run of inline content, and how far it is pushed in.
struct Line {
    spans: Vec<TextSpan>,
    indent: f32,
    heading: bool,
}

/// Flattens the markdown into the blocks a reader sees stacked.
fn lines_of(nodes: &[Node], indent: f32, into: &mut Vec<Line>, code: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Paragraph { children } => {
                let mut spans = Vec::new();
                inline(children, false, false, false, None, &mut spans);
                into.push(Line {
                    spans,
                    indent,
                    heading: false,
                });
            }
            Node::Heading { children, .. } => {
                let mut spans = Vec::new();
                inline(children, true, false, false, None, &mut spans);
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
                inline(
                    std::slice::from_ref(other),
                    false,
                    false,
                    false,
                    None,
                    &mut spans,
                );
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

/// The href, if it is one this window would follow.
///
/// A message is text somebody else wrote, and a markdown link can name any
/// scheme: `file:` reaching into the reader's disk, or anything the shell has
/// been taught to run. Only the two the web uses are carried; everything else
/// draws as words and does nothing when pressed.
fn openable(href: &str) -> Option<&str> {
    let scheme = href.split_once("://")?.0;
    matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https").then_some(href)
}

/// Collects inline content into styled spans.
///
/// `link` is the href the words are inside, threaded down like the styling is:
/// a link's text can be emphasised, code, or an emoji, and every piece of it
/// points at the same place.
fn inline(
    nodes: &[Node],
    bold: bool,
    italic: bool,
    mono: bool,
    link: Option<&str>,
    into: &mut Vec<TextSpan>,
) {
    for node in nodes {
        match node {
            Node::Text { value } => into.push(TextSpan {
                text: value.clone(),
                bold,
                italic,
                mono,
                link: link.map(str::to_string),
                faint: false,
                emoji: None,
            }),
            Node::Strong { children } => inline(children, true, italic, mono, link, into),
            Node::Emphasis { children } | Node::Strike { children } => {
                inline(children, bold, true, mono, link, into)
            }
            // Only what is worth opening. A message is untrusted text and a
            // href in it can name any scheme at all; anything but the web is
            // drawn as words and does nothing when pressed.
            Node::Link { children, href } => {
                inline(children, bold, italic, mono, openable(href), into)
            }
            Node::InlineCode { value } | Node::InlineMath { value } => into.push(TextSpan {
                text: value.clone(),
                bold,
                italic,
                mono: true,
                link: link.map(str::to_string),
                faint: false,
                emoji: None,
            }),
            Node::UserMention { username, .. } => into.push(TextSpan {
                text: format!("@{username}"),
                bold,
                italic,
                mono,
                link: link.map(str::to_string),
                faint: false,
                emoji: None,
            }),
            Node::ChannelLink { name } => into.push(TextSpan {
                text: format!("~{name}"),
                bold,
                italic,
                mono,
                link: link.map(str::to_string),
                faint: false,
                emoji: None,
            }),
            // A standard emoji is a character and shapes like any other letter.
            // A custom one has no character at all, so the span holds spaces
            // wide enough for the picture and carries the name for whoever
            // draws it. Non-breaking, or a wrap could split the placeholder in
            // half and the picture would land across two lines.
            Node::Emoji { name, unicode } => into.push(match unicode {
                Some(character) => TextSpan {
                    text: character.clone(),
                    bold,
                    italic,
                    mono,
                    link: link.map(str::to_string),
                    faint: false,
                    emoji: None,
                },
                None => TextSpan {
                    text: NBSP.repeat(EMOJI_ROOM),
                    bold,
                    italic,
                    mono,
                    link: link.map(str::to_string),
                    faint: false,
                    emoji: Some(name.clone()),
                },
            }),
            Node::SoftBreak | Node::HardBreak => into.push(TextSpan {
                text: "\n".to_string(),
                bold,
                italic,
                mono,
                link: link.map(str::to_string),
                faint: false,
                emoji: None,
            }),
            Node::Image { alt, .. } => into.push(TextSpan {
                text: alt.clone(),
                bold,
                italic,
                mono,
                link: link.map(str::to_string),
                faint: false,
                emoji: None,
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

    /// A message's own time, on the reader's clock.
    ///
    /// Hours and minutes only. A date belongs to the separator above the run,
    /// which is why one exists.
    fn clock(at: i64, offset_minutes: i32) -> String {
        let local = at / 1000 + i64::from(offset_minutes) * 60;
        let day = local.rem_euclid(86_400);
        format!("{:02}:{:02}", day / 3600, (day % 3600) / 60)
    }

    fn plain(text: String) -> TextSpan {
        TextSpan {
            text,
            bold: false,
            italic: false,
            mono: false,
            link: None,
            faint: false,
            emoji: None,
        }
    }

    let (nodes, post, header) = match row {
        Row::DateSeparator { .. } | Row::UnreadDivider => {
            return RowLayout {
                height: theme.separator_height,
                blocks: vec![Block {
                    y: 0.0,
                    x: 0.0,
                    height: theme.separator_height,
                    lines: 1,
                    kind: Kind::Separator,
                    spans: Vec::new(),
                    size: theme.body_size,
                    wrap: theme.text_width(),
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
                    spans: Vec::new(),
                    size: theme.body_size,
                    wrap: theme.text_width(),
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
                    spans: Vec::new(),
                    size: theme.body_size,
                    wrap: theme.text_width(),
                }],
            };
        }
        Row::Post { post } => (post.nodes.as_slice(), Some(post), true),
        Row::Continuation { post } => (post.nodes.as_slice(), Some(post), false),
    };

    let attachments = post.map(|post| post.files.len()).unwrap_or(0);

    y += theme.row_padding;
    if header {
        // Who and when, as text rather than a reserved rectangle: it is a line
        // like any other and it is what tells one message from the next.
        let mut spans = Vec::new();
        if let Some(post) = post {
            spans.push(TextSpan {
                text: post.author_name.clone(),
                bold: true,
                italic: false,
                mono: false,
                link: None,
                faint: false,
                emoji: None,
            });
            spans.push(plain("   ".to_string()));
            spans.push(TextSpan {
                text: clock(post.create_at, theme.utc_offset_minutes),
                bold: false,
                italic: false,
                mono: false,
                link: None,
                faint: true,
                emoji: None,
            });
            if post.edited {
                spans.push(TextSpan {
                    text: "  edited".to_string(),
                    bold: false,
                    italic: false,
                    mono: false,
                    link: None,
                    faint: true,
                    emoji: None,
                });
            }
        }
        blocks.push(Block {
            y,
            x: 0.0,
            height: theme.header_height,
            lines: 1,
            kind: Kind::Header,
            spans,
            size: theme.body_size,
            wrap: theme.text_width(),
        });
        y += theme.header_height;
    }

    let mut lines = Vec::new();
    let mut code = Vec::new();
    lines_of(nodes, 0.0, &mut lines, &mut code);

    for line in lines {
        let x = line.indent * theme.indent;
        let wrap = theme.text_width() - x;
        let count = line_count(fonts, &line, wrap, theme);
        let height = count as f32 * theme.line_height;
        let size = if line.heading {
            theme.body_size * 1.15
        } else {
            theme.body_size
        };
        blocks.push(Block {
            y,
            x,
            height,
            lines: count,
            kind: Kind::Text,
            spans: line.spans,
            size,
            wrap,
        });
        y += height + theme.block_gap;
    }

    for block in &code {
        // Measured at the width it will actually be shaped at, rather than
        // counting the lines as typed. Those two disagreed: the painter wrapped
        // at the column while this counted newlines, so one long log line
        // reserved a single line's height and drew over the message below it.
        //
        // The app scrolls a code block sideways rather than wrapping it, and
        // this will too once the renderer can scroll sideways. That is a change
        // to `wrap` alone -- the count stays right, because it asks what will be
        // drawn rather than what was written.
        let wrap = (theme.text_width() - theme.code_padding).max(40.0);
        let count = crate::extent_of(
            fonts,
            block,
            wrap,
            crate::Style {
                size: theme.code_size,
                line_height: theme.code_line_height,
                bold: false,
                italic: false,
                mono: true,
            },
        )
        .lines;
        let height = count as f32 * theme.code_line_height + theme.code_padding;
        blocks.push(Block {
            y,
            x: 0.0,
            height,
            lines: count,
            kind: Kind::Code,
            spans: vec![TextSpan {
                text: block.clone(),
                bold: false,
                italic: false,
                mono: true,
                link: None,
                faint: false,
                emoji: None,
            }],
            size: theme.code_size,
            wrap,
        });
        y += height + theme.block_gap;
    }

    if attachments > 0 {
        // The server gives every attachment its drawn size, so this is the one
        // height that was never in doubt: `box_width` and `box_height` are what
        // the planner already worked out, and reserving exactly them means a
        // picture arriving moves nothing.
        //
        // One block per file, stacked. Laying them side by side is a gallery,
        // which is a later decision about arrangement rather than about height.
        for file in post.map(|post| post.files.as_slice()).unwrap_or(&[]) {
            let height = if file.image || file.video {
                file.box_height.max(0) as f32
            } else {
                // A file card: a fixed row with a name and a size on it.
                theme.card_height
            };
            blocks.push(Block {
                y,
                x: 0.0,
                height,
                lines: 0,
                kind: Kind::Attachment,
                spans: Vec::new(),
                size: theme.body_size,
                wrap: (file.box_width.max(0) as f32).min(theme.text_width()),
            });
            y += height + theme.block_gap;
        }
    }

    // A link or permalink card, as a stack of lines rather than one block: a
    // card mixes sizes -- a small site name over a bold title over a quieter
    // description -- and one block carries one size. Stacked with no gap
    // between them, so the bar drawn beside the run reads as a single card.
    for preview in post.map(|post| post.previews.as_slice()).unwrap_or(&[]) {
        let x = theme.quote_bar + theme.indent / 2.0;
        let wrap = (theme.text_width() - x).max(40.0);
        let mut push = |text: &str, bold: bool, faint: bool, cap: usize, fonts: &mut Fonts| {
            if text.is_empty() {
                return;
            }
            let style = crate::Style {
                size: theme.body_size,
                line_height: theme.line_height,
                bold,
                italic: false,
                mono: false,
            };
            let count = crate::extent_of(fonts, text, wrap, style)
                .lines
                .max(1)
                .min(cap);
            blocks.push(Block {
                y,
                x,
                height: count as f32 * theme.line_height,
                lines: count,
                kind: Kind::Preview,
                spans: vec![TextSpan {
                    text: text.to_string(),
                    bold,
                    italic: false,
                    mono: false,
                    faint,
                    link: None,
                    emoji: None,
                }],
                size: theme.body_size,
                wrap,
            });
            y += count as f32 * theme.line_height;
        };
        match preview {
            matterless_render::Preview::Page {
                title,
                description,
                site_name,
                ..
            } => {
                push(site_name, false, true, 1, fonts);
                push(title, true, false, 2, fonts);
                push(description, false, true, theme.preview_lines, fonts);
            }
            matterless_render::Preview::Permalink {
                channel_label,
                author_name,
                nodes,
                ..
            } => {
                push(
                    &format!("{author_name} in {channel_label}"),
                    true,
                    false,
                    1,
                    fonts,
                );
                // The quoted body as plain text: a card is a summary, and a
                // quote that re-renders headings and lists inside a message is
                // a second message.
                let quoted = matterless_render::markdown::plain_text(nodes);
                push(&quoted, false, true, theme.preview_lines, fonts);
            }
        }
        y += theme.block_gap;
    }

    // One block per reaction rather than one line of them all, because each is
    // a thing to click: a pill nobody can press is a picture of a feature. The
    // blocks come out in the post's own reaction order, so the caller pairs the
    // nth with the nth reaction exactly as it does for attachments.
    if let Some(post) = post.filter(|post| !post.reactions.is_empty()) {
        let mut x = 0.0;
        for reaction in &post.reactions {
            // A standard emoji is a character and shapes like any other. A
            // custom one is an image behind the session token: it has no
            // character at all, so the pill reserves a square for it and the
            // renderer puts the picture there. Printing the name instead is
            // what made a reaction read ":bongo: 1".
            let custom = reaction.unicode.is_none();
            let label = match &reaction.unicode {
                Some(face) => format!("{face} {}", reaction.count),
                None => reaction.count.to_string(),
            };
            let width = crate::extent_of(
                fonts,
                &label,
                f32::MAX,
                crate::Style {
                    size: theme.body_size,
                    line_height: theme.reaction_height,
                    bold: false,
                    italic: false,
                    mono: false,
                },
            )
            .width
                + theme.pill_padding * 2.0
                + if custom { theme.emoji_size + 4.0 } else { 0.0 };
            // Wrapped by hand: a row of pills is a row of boxes, not a run of
            // text, so nothing else is going to wrap it.
            if x > 0.0 && x + width > theme.text_width() {
                x = 0.0;
                y += theme.reaction_height;
            }
            blocks.push(Block {
                y,
                x,
                height: theme.reaction_height,
                lines: 1,
                kind: Kind::Reactions,
                spans: vec![plain(label)],
                size: theme.body_size,
                // The pill's own width, which is what it is drawn and hit as.
                wrap: width,
            });
            x += width + theme.pill_gap;
        }
        y += theme.reaction_height;
    }

    // A message that has not been confirmed reads quieter than one that has,
    // which is the whole of what "pending" means to a reader: it is there, and
    // it is not certain yet. Done by marking the spans rather than by a state
    // the renderer would have to know about, so it draws through the same path
    // every other quiet thing does.
    if post.is_some_and(|post| post.pending) {
        for block in &mut blocks {
            for span in &mut block.spans {
                span.faint = true;
            }
        }
    }

    // A send that failed says so, in a line of its own that takes real height.
    // Leaving it to a colour would say nothing on a row already drawn faint,
    // and a message that silently looks delivered when it is not is the worst
    // of the available answers.
    if post.is_some_and(|post| post.failed) {
        blocks.push(Block {
            y,
            x: 0.0,
            height: theme.line_height,
            lines: 1,
            kind: Kind::Text,
            spans: vec![TextSpan {
                text: "Not sent. Click to try again.".to_string(),
                bold: false,
                italic: false,
                mono: false,
                link: None,
                faint: true,
                emoji: None,
            }],
            size: theme.body_size,
            wrap: theme.text_width(),
        });
        y += theme.line_height;
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

    /// A card takes room, or the message under it is drawn over.
    ///
    /// Reserved by the layout rather than discovered by the painter, which is
    /// the contract every other block keeps: the renderer draws exactly what
    /// was measured.
    #[test]
    fn a_link_card_reserves_the_room_it_draws_in() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let bare = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![text("look")]),
            },
            &theme,
        );

        let mut carded = post(vec![text("look")]);
        carded.previews = vec![matterless_render::Preview::Page {
            url: "https://example.invalid/thing".into(),
            title: "A page with a title".into(),
            description: "And a description long enough to say something about it.".into(),
            site_name: "example.invalid".into(),
            image: None,
        }];
        let with = lay_out(&mut fonts, &Row::Post { post: carded }, &theme);

        assert!(
            with.height > bare.height,
            "a card added no height: {} vs {}",
            with.height,
            bare.height
        );
        let card: Vec<&Block> = with
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Preview)
            .collect();
        // Site, title, description: three stacked lines, one card.
        assert_eq!(card.len(), 3);
        // Stacked with no gap, so the bar beside them reads as one.
        for pair in card.windows(2) {
            assert!(
                (pair[1].y - (pair[0].y + pair[0].height)).abs() < 0.5,
                "a gap opened between two lines of one card"
            );
        }
    }

    /// A description of a thousand words is not a summary. Past the cap the
    /// card would be the page.
    #[test]
    fn a_long_description_is_cut_rather_than_drawn_whole() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut carded = post(vec![text("look")]);
        carded.previews = vec![matterless_render::Preview::Page {
            url: "https://example.invalid/thing".into(),
            title: "Title".into(),
            description: "word ".repeat(500),
            site_name: "example.invalid".into(),
            image: None,
        }];
        let laid = lay_out(&mut fonts, &Row::Post { post: carded }, &theme);
        let longest = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Preview)
            .map(|block| block.lines)
            .max()
            .expect("a card");
        assert!(
            longest <= theme.preview_lines,
            "{longest} lines is not a summary"
        );
    }

    /// The whole point: a row's height is known, and it is different at two
    /// widths, without anything having been drawn.
    #[test]
    fn a_row_is_taller_in_a_narrower_list() {
        let mut fonts = Fonts::new();
        let row = Row::Post {
            post: post(vec![text(
                "I would like to raise this limit on swarm please, to fifty or perhaps a hundred, and it appears to live in the configuration file under the data directory. Do you have access to that in the tools team, or is it more a question for the people who look after the servers? Either way suits me, but the current limit stops the review expanding at all.",
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

    /// Short lines do not wrap, so the count is the lines as written.
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

    /// A pasted build error is one line as typed and several as drawn. Counting
    /// what was written reserved a single line, and the message below it was
    /// drawn over the top.
    #[test]
    fn a_long_code_line_reserves_the_height_it_will_be_drawn_at() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let long = "UATHelper: Packaging (Windows): Module.Curiosity.27.cpp.obj : error \
                    LNK2019: unresolved external symbol \"protected: void __cdecl \
                    U3DFlowManager::ProcessPendingSpawningRequest(class FSCBudgetedWork &)\" \
                    referenced in function \"public: void __cdecl U3DFlowManager::Initialize\"";
        let row = Row::Post {
            post: post(vec![Node::CodeBlock {
                language: None,
                value: long.into(),
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let code = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");

        assert_eq!(long.lines().count(), 1, "one line as typed");
        assert!(
            code.lines > 1,
            "a line far wider than the column has to wrap"
        );
        assert_eq!(
            code.height,
            code.lines as f32 * theme.code_line_height + theme.code_padding
        );
        // The block has to fit inside the row it is part of, or it draws over
        // whatever comes next.
        assert!(laid.height >= code.y + code.height);
    }

    /// The layout and the painter have to shape at the same width, or the count
    /// is right about a wrap that never happens.
    #[test]
    fn a_code_block_is_shaped_inside_its_own_padding() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let row = Row::Post {
            post: post(vec![Node::CodeBlock {
                language: None,
                value: "one".into(),
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let code = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");
        assert_eq!(code.wrap, theme.text_width() - theme.code_padding);
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

    fn picture(id: &str, width: i32, height: i32) -> matterless_render::FileRef {
        matterless_render::FileRef {
            id: id.into(),
            name: "shot.png".into(),
            extension: "png".into(),
            size: 1024,
            mime_type: "image/png".into(),
            width,
            height,
            image: true,
            video: false,
            variant: matterless_render::ImageVariant::Preview,
            mini_preview: None,
            box_width: width,
            box_height: height,
            archived: false,
        }
    }

    /// The height the server already told us, reserved before the bytes
    /// arrive. Without this an image appearing would push the conversation
    /// down under it -- the one thing this whole layout exists to prevent.
    #[test]
    fn an_attachment_reserves_the_box_the_server_gave_it() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 300, 200)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let block = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Attachment)
            .expect("an attachment block");
        assert_eq!(block.height, 200.0);
        assert_eq!(block.wrap, 300.0);
        assert!(laid.height >= block.y + block.height);
    }

    /// Two pictures are two blocks, stacked, and the row is tall enough for
    /// both.
    #[test]
    fn every_attachment_gets_its_own_room() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 300, 200), picture("f2", 120, 90)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let blocks: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Attachment)
            .collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].height, 200.0);
        assert_eq!(blocks[1].height, 90.0);
        assert!(blocks[1].y >= blocks[0].y + blocks[0].height);
        assert!(laid.height >= blocks[1].y + blocks[1].height);
    }

    /// A picture wider than the column is drawn no wider than the column.
    #[test]
    fn an_attachment_never_runs_past_the_column() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 4000, 100)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let block = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Attachment)
            .expect("an attachment block");
        assert_eq!(block.wrap, theme.text_width());
    }

    /// Something that is not a picture is a card, and a card has a fixed row of
    /// its own rather than a zero-height nothing.
    #[test]
    fn a_file_that_is_not_a_picture_gets_a_card() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        let mut document = picture("f1", 0, 0);
        document.image = false;
        document.name = "notes.pdf".into();
        with.files = vec![document];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let block = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Attachment)
            .expect("an attachment block");
        assert_eq!(block.height, theme.card_height);
    }

    /// A message that has not been confirmed reads quieter than one that has.
    /// Without this an optimistic send looks exactly like a delivered one, and
    /// the reader has no way to tell a message that went from one that might
    /// not have.
    #[test]
    fn a_pending_message_is_drawn_quietly() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut guess = post(vec![Node::Paragraph {
            children: vec![Node::Text {
                value: "on its way".into(),
            }],
        }]);
        guess.pending = true;
        let laid = lay_out(&mut fonts, &Row::Post { post: guess }, &theme);
        let spans: Vec<&TextSpan> = laid.blocks.iter().flat_map(|block| &block.spans).collect();
        assert!(!spans.is_empty());
        assert!(
            spans.iter().all(|span| span.faint),
            "every span of a pending message is quiet"
        );
    }

    /// A send that failed says so in a line of its own, which takes real
    /// height: a message that silently looks delivered when it is not is the
    /// worst of the available answers.
    #[test]
    fn a_failed_message_says_so_and_takes_the_room_to() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let body = vec![Node::Paragraph {
            children: vec![Node::Text {
                value: "did not go".into(),
            }],
        }];
        let sent = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(body.clone()),
            },
            &theme,
        );

        let mut broken = post(body);
        broken.failed = true;
        let laid = lay_out(&mut fonts, &Row::Post { post: broken }, &theme);

        assert!(
            laid.blocks
                .iter()
                .flat_map(|block| &block.spans)
                .any(|span| span.text.contains("Not sent")),
            "a failed message says it did not go"
        );
        assert_eq!(
            laid.height,
            sent.height + theme.line_height,
            "and the row is a line taller for saying it"
        );
    }
}
